using System.Text;
using System.Text.Json;
using Breez.Sdk.Spark;

namespace BreezCli;

/// <summary>
/// Represents a single x402 subcommand.
/// </summary>
public class X402CliCommand
{
    public required string Name { get; init; }
    public required string Description { get; init; }
    public required Func<BreezSdk, Func<string, string?>, string[], Task> Run { get; init; }
}

/// <summary>
/// x402 subcommand names (used for REPL completion).
/// </summary>
public static class X402CommandNames
{
    public static readonly string[] All =
    {
        "x402 fetch",
    };
}

/// <summary>
/// x402 subcommand handlers.
/// </summary>
public static class X402Commands
{
    private const string PaymentRequiredHeader = "payment-required";
    private const string PaymentSignatureHeader = "payment-signature";
    private const string PaymentResponseHeader = "payment-response";
    // Times to pay the same challenge again while its payment is pending. Fetching
    // again instead would bring a new challenge with a new invoice to pay.
    private const int PendingRetries = 3;
    // Times to send the paid request again while the server answers that the
    // payment may still settle.
    private const int SettleRetries = 5;
    private static readonly TimeSpan SettleRetryDelay = TimeSpan.FromSeconds(2);

    /// <summary>
    /// Builds the x402 subcommand registry.
    /// </summary>
    public static Dictionary<string, X402CliCommand> BuildRegistry()
    {
        return new Dictionary<string, X402CliCommand>
        {
            ["fetch"] = new()
            {
                Name = "fetch",
                Description = "Fetch a URL, paying its x402 Lightning challenge if the server asks for one",
                Run = HandleFetch
            },
        };
    }

    /// <summary>
    /// Dispatches an x402 subcommand.
    /// </summary>
    public static async Task DispatchCommand(
        string[] args,
        BreezSdk sdk,
        Func<string, string?> readline)
    {
        var registry = BuildRegistry();

        if (args.Length == 0 || args[0] == "help")
        {
            Console.WriteLine();
            Console.WriteLine("x402 subcommands:");
            var names = registry.Keys.OrderBy(k => k).ToList();
            foreach (var name in names)
            {
                Console.WriteLine($"  x402 {name,-30} {registry[name].Description}");
            }
            Console.WriteLine();
            return;
        }

        var subName = args[0];
        var subArgs = args.Skip(1).ToArray();

        if (!registry.TryGetValue(subName, out var cmd))
        {
            Console.WriteLine($"Unknown x402 subcommand: {subName}. Use 'x402 help' for available commands.");
            return;
        }

        await cmd.Run(sdk, readline, subArgs);
    }

    // -----------------------------------------------------------------------
    // x402 command handlers
    // -----------------------------------------------------------------------

    // --- fetch ---

    private static async Task HandleFetch(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        string? url = null;
        string? maxAmountSatsStr = null;
        var method = "GET";
        string? body = null;
        var headerLines = new List<string>();

        for (int i = 0; i < args.Length; i++)
        {
            var arg = args[i];
            var hasValue = i + 1 < args.Length;
            switch (arg)
            {
                case "--max-amount-sats" when hasValue:
                    maxAmountSatsStr = args[++i];
                    break;
                case "-X" or "--method" when hasValue:
                    method = args[++i];
                    break;
                case "-d" or "--body" when hasValue:
                    body = args[++i];
                    break;
                case "-H" or "--header" when hasValue:
                    headerLines.Add(args[++i]);
                    break;
                default:
                    if (!arg.StartsWith('-') && url == null)
                    {
                        url = arg;
                    }
                    break;
            }
        }

        if (url == null || maxAmountSatsStr == null)
        {
            Console.WriteLine("Usage: x402 fetch <url> --max-amount-sats <sats> [-X GET|POST] [-d <body>] [-H \"Name: value\"]...");
            return;
        }
        var maxAmountSats = ulong.Parse(maxAmountSatsStr);
        if (method == "GET" && body != null)
        {
            throw new ArgumentException("a GET request has no body: use -X POST to send one");
        }

        using var client = new HttpClient(new HttpClientHandler
        {
            // Redirects aren't followed: the payment then commits to exactly
            // the request sent.
            AllowAutoRedirect = false,
            // The cookie container would send cookies the server set, which
            // the payment does not bind.
            UseCookies = false,
        });
        // The server hashes the headers it binds, so send every header
        // explicitly: anything the HTTP client added on its own would be
        // missing from the hash computed here.
        var headers = ParseHeaders(headerLines);
        headers.TryAdd("accept", "*/*");
        // The URL as HttpClient sends it: `http://host` goes out as
        // `http://host/`, and a fragment never goes out.
        url = new Uri(url).GetLeftPart(UriPartial.Query);

        var response = await Send(client, method, url, headers, body);
        if (response.Status != 402)
        {
            PrintResponse(response);
            return;
        }
        var paymentRequired = response.Header(PaymentRequiredHeader)
            ?? throw new InvalidOperationException("402 response has no PAYMENT-REQUIRED header");

        var x402 = sdk.GetX402();
        var request = new PayX402Request(
            paymentRequired: paymentRequired,
            request: new X402Request.Http(
                method: method,
                url: url,
                headers: headers.Select(h => new X402Header(name: h.Key, value: h.Value)).ToArray(),
                body: body != null ? Encoding.UTF8.GetBytes(body) : null
            ),
            maxAmount: new X402PaymentAmount.Lightning(amountSats: maxAmountSats),
            conversionOptions: null,
            completionTimeoutSecs: null
        );
        var paid = await x402.Pay(request: request);
        for (int i = 0; i < PendingRetries; i++)
        {
            if (paid.status == X402PaymentStatus.Paid)
            {
                break;
            }
            Console.WriteLine("Payment is pending, waiting for it to complete...");
            paid = await x402.Pay(request: request);
        }
        // The payment signature proves the payment, so it isn't printed.
        Console.WriteLine(
            $"x402 payment {paid.payment.id} is {paid.status}: {paid.payment.amount} sats, fee {paid.payment.fees} sats");
        if (paid.paymentSignature == null)
        {
            Console.WriteLine("Payment is still pending. Check its status with list-payments.");
            return;
        }

        headers[PaymentSignatureHeader] = paid.paymentSignature;
        response = await Send(client, method, url, headers, body);
        for (int i = 0; i < SettleRetries; i++)
        {
            if (!SettlementPending(response))
            {
                break;
            }
            Console.WriteLine("Payment is not settled yet, sending it again...");
            await Task.Delay(SettleRetryDelay);
            response = await Send(client, method, url, headers, body);
        }
        PrintResponse(response);
    }

    private static Dictionary<string, string> ParseHeaders(List<string> lines)
    {
        var headers = new Dictionary<string, string>();
        foreach (var line in lines)
        {
            var colon = line.IndexOf(':');
            if (colon < 0)
            {
                throw new ArgumentException($"header \"{line}\" is not \"Name: value\"");
            }
            headers[line[..colon].Trim().ToLowerInvariant()] = line[(colon + 1)..].Trim();
        }
        return headers;
    }

    private sealed record FetchResponse(int Status, Dictionary<string, string> Headers, string Body)
    {
        public string? Header(string name) => Headers.TryGetValue(name, out var value) ? value : null;
    }

    private static async Task<FetchResponse> Send(
        HttpClient client,
        string method,
        string url,
        Dictionary<string, string> headers,
        string? body)
    {
        using var request = method switch
        {
            "GET" => new HttpRequestMessage(HttpMethod.Get, url),
            "POST" => new HttpRequestMessage(HttpMethod.Post, url)
            {
                Content = body != null ? new ByteArrayContent(Encoding.UTF8.GetBytes(body)) : null
            },
            _ => throw new ArgumentException($"unsupported method {method}: use GET or POST"),
        };
        foreach (var (name, value) in headers)
        {
            // Content headers such as content-type are refused on the request itself.
            if (!request.Headers.TryAddWithoutValidation(name, value))
            {
                request.Content ??= new ByteArrayContent(Array.Empty<byte>());
                request.Content.Headers.TryAddWithoutValidation(name, value);
            }
        }

        using var response = await client.SendAsync(request);
        var responseHeaders = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var header in response.Headers.Concat(response.Content.Headers))
        {
            responseHeaders.TryAdd(header.Key, header.Value.First());
        }
        var status = (int)response.StatusCode;
        if (status is 301 or 302 or 303 or 307 or 308)
        {
            var location = responseHeaders.GetValueOrDefault("location") ?? "<no location>";
            throw new HttpRequestException(
                $"Redirect error: endpoint returned a {status} redirect to {location}, which this client does not follow");
        }
        var responseBody = await response.Content.ReadAsStringAsync();
        return new FetchResponse(status, responseHeaders, responseBody);
    }

    // Whether the server answered that the payment may still settle: a 402 with
    // no new challenge and the core x402 specification's non-final reason.
    private static bool SettlementPending(FetchResponse response)
    {
        var settlement = response.Header(PaymentResponseHeader);
        if (response.Status != 402 || response.Header(PaymentRequiredHeader) != null || settlement == null)
        {
            return false;
        }
        try
        {
            using var json = JsonDocument.Parse(Convert.FromBase64String(settlement));
            return json.RootElement.ValueKind == JsonValueKind.Object
                && json.RootElement.TryGetProperty("errorReason", out var reason)
                && reason.ValueKind == JsonValueKind.String
                && reason.ValueEquals("settlement_pending");
        }
        catch (Exception e) when (e is FormatException or JsonException)
        {
            return false;
        }
    }

    private static void PrintResponse(FetchResponse response)
    {
        Console.WriteLine($"HTTP {response.Status}");
        var settlement = response.Header(PaymentResponseHeader);
        if (settlement != null)
        {
            try
            {
                var json = Convert.FromBase64String(settlement);
                Console.WriteLine($"PAYMENT-RESPONSE: {Encoding.UTF8.GetString(json)}");
            }
            catch (FormatException)
            {
            }
        }
        Console.WriteLine(response.Body);
    }
}
