using System.Numerics;
using System.Security.Cryptography;
using Breez.Sdk.Spark;

namespace BreezCli;

/// <summary>
/// Represents a single CLI command.
/// </summary>
public class CliCommand
{
    public required string Name { get; init; }
    public required string Description { get; init; }
    public required Func<BreezSdk, Func<string, string?> , string[], Task> Run { get; init; }
}

/// <summary>
/// All top-level command names (used for REPL completion).
/// </summary>
public static class CommandNames
{
    public static readonly string[] All =
    {
        "get-info",
        "get-payment",
        "sync",
        "list-payments",
        "receive",
        "pay",
        "lnurl-pay",
        "lnurl-withdraw",
        "lnurl-auth",
        "claim-htlc-payment",
        "claim-deposit",
        "parse",
        "refund-deposit",
        "list-unclaimed-deposits",
        "buy-bitcoin",
        "check-lightning-address-available",
        "get-lightning-address",
        "register-lightning-address",
        "accept-lightning-address-transfer",
        "delete-lightning-address",
        "list-fiat-currencies",
        "list-fiat-rates",
        "recommended-fees",
        "get-tokens-metadata",
        "fetch-conversion-limits",
        "get-user-settings",
        "set-user-settings",
        "get-spark-status",
    };
}

/// <summary>
/// Builds and dispatches all CLI commands.
/// </summary>
public static class Commands
{
    /// <summary>
    /// Builds the command registry mapping command names to handlers.
    /// </summary>
    public static Dictionary<string, CliCommand> BuildRegistry()
    {
        return new Dictionary<string, CliCommand>
        {
            ["get-info"] = new()
            {
                Name = "get-info",
                Description = "Get balance information",
                Run = HandleGetInfo
            },
            ["get-payment"] = new()
            {
                Name = "get-payment",
                Description = "Get the payment with the given ID",
                Run = HandleGetPayment
            },
            ["sync"] = new()
            {
                Name = "sync",
                Description = "Sync wallet state",
                Run = HandleSync
            },
            ["list-payments"] = new()
            {
                Name = "list-payments",
                Description = "List payments",
                Run = HandleListPayments
            },
            ["receive"] = new()
            {
                Name = "receive",
                Description = "Receive a payment",
                Run = HandleReceive
            },
            ["pay"] = new()
            {
                Name = "pay",
                Description = "Pay the given payment request",
                Run = HandlePay
            },
            ["lnurl-pay"] = new()
            {
                Name = "lnurl-pay",
                Description = "Pay using LNURL",
                Run = HandleLnurlPay
            },
            ["lnurl-withdraw"] = new()
            {
                Name = "lnurl-withdraw",
                Description = "Withdraw using LNURL",
                Run = HandleLnurlWithdraw
            },
            ["lnurl-auth"] = new()
            {
                Name = "lnurl-auth",
                Description = "Authenticate using LNURL",
                Run = HandleLnurlAuth
            },
            ["claim-htlc-payment"] = new()
            {
                Name = "claim-htlc-payment",
                Description = "Claim an HTLC payment",
                Run = HandleClaimHtlcPayment
            },
            ["claim-deposit"] = new()
            {
                Name = "claim-deposit",
                Description = "Claim an on-chain deposit",
                Run = HandleClaimDeposit
            },
            ["parse"] = new()
            {
                Name = "parse",
                Description = "Parse an input (invoice, address, LNURL)",
                Run = HandleParse
            },
            ["refund-deposit"] = new()
            {
                Name = "refund-deposit",
                Description = "Refund an on-chain deposit",
                Run = HandleRefundDeposit
            },
            ["list-unclaimed-deposits"] = new()
            {
                Name = "list-unclaimed-deposits",
                Description = "List unclaimed on-chain deposits",
                Run = HandleListUnclaimedDeposits
            },
            ["buy-bitcoin"] = new()
            {
                Name = "buy-bitcoin",
                Description = "Buy Bitcoin using an external provider",
                Run = HandleBuyBitcoin
            },
            ["check-lightning-address-available"] = new()
            {
                Name = "check-lightning-address-available",
                Description = "Check if a lightning address username is available",
                Run = HandleCheckLightningAddress
            },
            ["get-lightning-address"] = new()
            {
                Name = "get-lightning-address",
                Description = "Get registered lightning address",
                Run = HandleGetLightningAddress
            },
            ["register-lightning-address"] = new()
            {
                Name = "register-lightning-address",
                Description = "Register a lightning address",
                Run = HandleRegisterLightningAddress
            },
            ["accept-lightning-address-transfer"] = new()
            {
                Name = "accept-lightning-address-transfer",
                Description = "Produce a transfer authorization for the current username, granting it to a transferee pubkey",
                Run = HandleAcceptLightningAddressTransfer
            },
            ["delete-lightning-address"] = new()
            {
                Name = "delete-lightning-address",
                Description = "Delete lightning address",
                Run = HandleDeleteLightningAddress
            },
            ["list-fiat-currencies"] = new()
            {
                Name = "list-fiat-currencies",
                Description = "List fiat currencies",
                Run = HandleListFiatCurrencies
            },
            ["list-fiat-rates"] = new()
            {
                Name = "list-fiat-rates",
                Description = "List available fiat rates",
                Run = HandleListFiatRates
            },
            ["recommended-fees"] = new()
            {
                Name = "recommended-fees",
                Description = "Get recommended BTC fees",
                Run = HandleRecommendedFees
            },
            ["get-tokens-metadata"] = new()
            {
                Name = "get-tokens-metadata",
                Description = "Get metadata for token(s)",
                Run = HandleGetTokensMetadata
            },
            ["fetch-conversion-limits"] = new()
            {
                Name = "fetch-conversion-limits",
                Description = "Fetch conversion limits for a token",
                Run = HandleFetchConversionLimits
            },
            ["get-user-settings"] = new()
            {
                Name = "get-user-settings",
                Description = "Get user settings",
                Run = HandleGetUserSettings
            },
            ["set-user-settings"] = new()
            {
                Name = "set-user-settings",
                Description = "Update user settings",
                Run = HandleSetUserSettings
            },
            ["get-spark-status"] = new()
            {
                Name = "get-spark-status",
                Description = "Get Spark network service status",
                Run = HandleGetSparkStatus
            },
        };
    }

    /// <summary>
    /// Prints all available commands.
    /// </summary>
    public static void PrintHelp(Dictionary<string, CliCommand> registry)
    {
        Console.WriteLine();
        Console.WriteLine("Available commands:");
        var names = registry.Keys.OrderBy(k => k).ToList();
        foreach (var name in names)
        {
            Console.WriteLine($"  {name,-40} {registry[name].Description}");
        }
        Console.WriteLine($"  {"issuer <subcommand>",-40} Token issuer commands (use 'issuer help' for details)");
        Console.WriteLine($"  {"contacts <subcommand>",-40} Contact commands (use 'contacts help' for details)");
        Console.WriteLine($"  {"webhooks <subcommand>",-40} Webhook commands (use 'webhooks help' for details)");
        Console.WriteLine($"  {"exit / quit",-40} Exit the CLI");
        Console.WriteLine($"  {"help",-40} Show this help message");
        Console.WriteLine();
    }

    // -----------------------------------------------------------------------
    // Argument parsing helpers
    // -----------------------------------------------------------------------

    private static string? GetFlag(string[] args, params string[] names)
    {
        for (int i = 0; i < args.Length - 1; i++)
        {
            if (names.Contains(args[i]))
            {
                return args[i + 1];
            }
        }
        return null;
    }

    private static bool HasFlag(string[] args, params string[] names)
    {
        return args.Any(a => names.Contains(a));
    }

    /// <summary>
    /// Known boolean flags that do NOT take a value argument.
    /// </summary>
    private static readonly HashSet<string> BooleanFlags = new()
    {
        "--fees-included", "--from-bitcoin", "--hodl", "-f", "--freezable",
        "--from-bitcoin"
    };

    private static string[] GetPositionalArgs(string[] args)
    {
        var positional = new List<string>();
        for (int i = 0; i < args.Length; i++)
        {
            if (args[i].StartsWith('-'))
            {
                if (BooleanFlags.Contains(args[i]))
                {
                    continue; // boolean flag, no value to skip
                }
                // Value flag: skip the next argument
                if (i + 1 < args.Length)
                {
                    i++;
                }
                continue;
            }
            positional.Add(args[i]);
        }
        return positional.ToArray();
    }

    private static ulong? ParseOptionalUlong(string? value)
    {
        if (value == null) return null;
        return ulong.Parse(value);
    }

    private static uint? ParseOptionalUint(string? value)
    {
        if (value == null) return null;
        return uint.Parse(value);
    }

    private static BigInteger? ParseOptionalBigInt(string? value)
    {
        if (value == null) return null;
        return BigInteger.Parse(value);
    }

    // -----------------------------------------------------------------------
    // Command handlers
    // -----------------------------------------------------------------------

    // --- get-info ---

    private static async Task HandleGetInfo(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        bool? ensureSynced = null;
        var syncFlag = GetFlag(args, "-e", "--ensure-synced");
        if (syncFlag != null)
        {
            ensureSynced = syncFlag.ToLower() == "true";
        }

        var result = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: ensureSynced));
        Serialization.PrintValue(result);
    }

    // --- get-payment ---

    private static async Task HandleGetPayment(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: get-payment <payment_id>");
            return;
        }

        var result = await sdk.GetPayment(request: new GetPaymentRequest(paymentId: args[0]));
        Serialization.PrintValue(result);
    }

    // --- sync ---

    private static async Task HandleSync(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var result = await sdk.SyncWallet(request: new SyncWalletRequest());
        Serialization.PrintValue(result);
    }

    // --- list-payments ---

    private static async Task HandleListPayments(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var limitStr = GetFlag(args, "-l", "--limit") ?? "10";
        var offsetStr = GetFlag(args, "-o", "--offset") ?? "0";
        var typeFilterStr = GetFlag(args, "-t", "--type-filter");
        var statusFilterStr = GetFlag(args, "-s", "--status-filter");
        var assetFilterStr = GetFlag(args, "-a", "--asset-filter");
        var htlcStatusStr = GetFlag(args, "--spark-htlc-status-filter");
        var txHash = GetFlag(args, "--tx-hash");
        var txTypeStr = GetFlag(args, "--tx-type");
        var fromTimestampStr = GetFlag(args, "--from-timestamp");
        var toTimestampStr = GetFlag(args, "--to-timestamp");
        var sortAscStr = GetFlag(args, "--sort-ascending");

        var limit = uint.Parse(limitStr);
        var offset = uint.Parse(offsetStr);

        // Parse type filter
        PaymentType[]? typeFilter = null;
        if (typeFilterStr != null)
        {
            typeFilter = typeFilterStr.Split(',')
                .Select(s => Enum.Parse<PaymentType>(s.Trim(), ignoreCase: true))
                .ToArray();
        }

        // Parse status filter
        PaymentStatus[]? statusFilter = null;
        if (statusFilterStr != null)
        {
            statusFilter = statusFilterStr.Split(',')
                .Select(s => Enum.Parse<PaymentStatus>(s.Trim(), ignoreCase: true))
                .ToArray();
        }

        // Parse asset filter
        AssetFilter? assetFilter = null;
        if (assetFilterStr != null)
        {
            if (assetFilterStr.ToLower() == "bitcoin")
            {
                assetFilter = new AssetFilter.Bitcoin();
            }
            else
            {
                assetFilter = new AssetFilter.Token(tokenIdentifier: assetFilterStr);
            }
        }

        // Payment details filters
        var paymentDetailsFilterList = new List<PaymentDetailsFilter>();
        if (htlcStatusStr != null || txHash != null || txTypeStr != null)
        {

            if (htlcStatusStr != null)
            {
                var statuses = htlcStatusStr.Split(',')
                    .Select(s => Enum.Parse<SparkHtlcStatus>(s.Trim(), ignoreCase: true))
                    .ToArray();
                paymentDetailsFilterList.Add(new PaymentDetailsFilter.Spark(
                    htlcStatus: statuses,
                    conversionRefundNeeded: null
                ));
            }

            if (txHash != null)
            {
                paymentDetailsFilterList.Add(new PaymentDetailsFilter.Token(
                    conversionRefundNeeded: null,
                    txType: null,
                    txHash: txHash
                ));
            }

            if (txTypeStr != null)
            {
                var txType = Enum.Parse<TokenTransactionType>(txTypeStr, ignoreCase: true);
                paymentDetailsFilterList.Add(new PaymentDetailsFilter.Token(
                    conversionRefundNeeded: null,
                    txType: txType,
                    txHash: null
                ));
            }
        }

        bool? sortAscending = sortAscStr != null ? sortAscStr.ToLower() == "true" : null;

        var result = await sdk.ListPayments(request: new ListPaymentsRequest(
            limit: limit,
            offset: offset,
            typeFilter: typeFilter,
            statusFilter: statusFilter,
            assetFilter: assetFilter,
            paymentDetailsFilter: paymentDetailsFilterList.Count > 0 ? paymentDetailsFilterList.ToArray() : null,
            fromTimestamp: ParseOptionalUlong(fromTimestampStr),
            toTimestamp: ParseOptionalUlong(toTimestampStr),
            sortAscending: sortAscending
        ));
        Serialization.PrintValue(result);
    }

    // --- receive ---

    private static async Task HandleReceive(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var method = GetFlag(args, "-m", "--method");
        var description = GetFlag(args, "-d", "--description");
        var amountStr = GetFlag(args, "-a", "--amount");
        var tokenIdentifier = GetFlag(args, "-t", "--token-identifier");
        var expirySecsStr = GetFlag(args, "-e", "--expiry-secs");
        var senderPubKey = GetFlag(args, "-s", "--sender-public-key");
        var hodl = HasFlag(args, "--hodl");
        var newAddress = HasFlag(args, "--new-address");

        if (method == null)
        {
            Console.WriteLine("Usage: receive -m <method> [options]");
            Console.WriteLine("Methods: sparkaddress, sparkinvoice, bitcoin, bolt11");
            return;
        }

        ReceivePaymentMethod paymentMethod;

        switch (method.ToLower())
        {
            case "sparkaddress":
                paymentMethod = new ReceivePaymentMethod.SparkAddress();
                break;

            case "sparkinvoice":
                BigInteger? amount = ParseOptionalBigInt(amountStr);
                ulong? expiryTime = null;
                if (expirySecsStr != null)
                {
                    var secs = uint.Parse(expirySecsStr);
                    expiryTime = (ulong)DateTimeOffset.UtcNow.ToUnixTimeSeconds() + secs;
                }
                paymentMethod = new ReceivePaymentMethod.SparkInvoice(
                    amount: amount,
                    tokenIdentifier: tokenIdentifier,
                    expiryTime: expiryTime,
                    description: description,
                    senderPublicKey: senderPubKey
                );
                break;

            case "bitcoin":
                paymentMethod = new ReceivePaymentMethod.BitcoinAddress(newAddress: newAddress);
                break;

            case "bolt11":
                ulong? amountSats = null;
                if (amountStr != null)
                {
                    amountSats = ulong.Parse(amountStr);
                }

                string? paymentHash = null;
                if (hodl)
                {
                    var preimageBytes = new byte[32];
                    RandomNumberGenerator.Fill(preimageBytes);
                    var preimage = Convert.ToHexString(preimageBytes).ToLowerInvariant();
                    paymentHash = ComputeSha256Hex(preimageBytes);

                    Console.WriteLine($"HODL invoice preimage: {preimage}");
                    Console.WriteLine($"Payment hash: {paymentHash}");
                    Console.WriteLine("Save the preimage! Use `claim-htlc-payment` with it to settle.");
                }

                paymentMethod = new ReceivePaymentMethod.Bolt11Invoice(
                    description: description ?? "",
                    amountSats: amountSats,
                    expirySecs: ParseOptionalUint(expirySecsStr),
                    paymentHash: paymentHash
                );
                break;

            default:
                Console.WriteLine($"Invalid payment method: {method}");
                Console.WriteLine("Available methods: sparkaddress, sparkinvoice, bitcoin, bolt11");
                return;
        }

        var result = await sdk.ReceivePayment(request: new ReceivePaymentRequest(paymentMethod: paymentMethod));

        if (result.fee > 0)
        {
            Console.WriteLine($"Prepared payment requires fee of {result.fee} sats/token base units");
        }

        Serialization.PrintValue(result);
    }

    // --- pay ---

    private static async Task HandlePay(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var paymentRequest = GetFlag(args, "-r", "--payment-request");
        var amountStr = GetFlag(args, "-a", "--amount");
        var tokenIdentifier = GetFlag(args, "-t", "--token-identifier");
        var idempotencyKey = GetFlag(args, "-i", "--idempotency-key");
        var fromBitcoin = HasFlag(args, "--from-bitcoin");
        var fromTokenId = GetFlag(args, "--from-token");
        var maxSlippageStr = GetFlag(args, "-s", "--convert-max-slippage-bps");
        var feesIncluded = HasFlag(args, "--fees-included");

        if (paymentRequest == null)
        {
            Console.WriteLine("Usage: pay -r <payment_request> [-a <amount>] [-t <token_identifier>]");
            return;
        }

        // Build conversion options
        ConversionOptions? conversionOptions = null;
        if (fromBitcoin)
        {
            conversionOptions = new ConversionOptions(
                conversionType: new ConversionType.FromBitcoin(),
                maxSlippageBps: ParseOptionalUint(maxSlippageStr),
                completionTimeoutSecs: null
            );
        }
        else if (fromTokenId != null)
        {
            conversionOptions = new ConversionOptions(
                conversionType: new ConversionType.ToBitcoin(fromTokenIdentifier: fromTokenId),
                maxSlippageBps: ParseOptionalUint(maxSlippageStr),
                completionTimeoutSecs: null
            );
        }

        FeePolicy? feePolicy = feesIncluded ? FeePolicy.FeesIncluded : null;

        var prepareResponse = await sdk.PrepareSendPayment(request: new PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: ParseOptionalUlong(amountStr),
            tokenIdentifier: tokenIdentifier,
            conversionOptions: conversionOptions,
            feePolicy: feePolicy
        ));

        // Show conversion estimate if present
        if (prepareResponse.conversionEstimate != null)
        {
            var est = prepareResponse.conversionEstimate;
            Console.WriteLine($"Estimated conversion of {est.amountIn} → {est.amountOut} with a {est.fee} fee");
            var line = readline("Do you want to continue (y/n) [y]: ");
            if (line != null && line.Trim().ToLower() != "" && line.Trim().ToLower() != "y")
            {
                Console.WriteLine("Payment cancelled");
                return;
            }
        }

        // Read payment options
        var paymentOptions = ReadPaymentOptions(prepareResponse.paymentMethod, readline);

        var result = await sdk.SendPayment(request: new SendPaymentRequest(
            prepareResponse: prepareResponse,
            options: paymentOptions,
            idempotencyKey: idempotencyKey
        ));

        Serialization.PrintValue(result);
    }

    // --- lnurl-pay ---

    private static async Task HandleLnurlPay(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var comment = GetFlag(args, "-c", "--comment");
        var validateStr = GetFlag(args, "-v", "--validate");
        var idempotencyKey = GetFlag(args, "-i", "--idempotency-key");
        var fromTokenId = GetFlag(args, "--from-token");
        var maxSlippageStr = GetFlag(args, "-s", "--convert-max-slippage-bps");
        var feesIncluded = HasFlag(args, "--fees-included");

        var positional = GetPositionalArgs(args);
        if (positional.Length < 1)
        {
            Console.WriteLine("Usage: lnurl-pay <lnurl> [options]");
            return;
        }
        var lnurl = positional[0];

        // Build conversion options
        ConversionOptions? conversionOptions = null;
        if (fromTokenId != null)
        {
            conversionOptions = new ConversionOptions(
                conversionType: new ConversionType.ToBitcoin(fromTokenIdentifier: fromTokenId),
                maxSlippageBps: ParseOptionalUint(maxSlippageStr),
                completionTimeoutSecs: null
            );
        }

        FeePolicy? feePolicy = feesIncluded ? FeePolicy.FeesIncluded : null;

        var input = await sdk.Parse(lnurl);

        LnurlPayRequestDetails payRequest;
        if (input is InputType.LightningAddress la)
        {
            payRequest = la.v1.payRequest;
        }
        else if (input is InputType.LnurlPay lp)
        {
            payRequest = lp.v1;
        }
        else
        {
            Console.WriteLine("Error: input is not an LNURL-pay or lightning address");
            return;
        }

        var minSendable = (payRequest.minSendable + 999) / 1000; // div_ceil(1000)
        var maxSendable = payRequest.maxSendable / 1000;
        var prompt = $"Amount to pay (min {minSendable} sat, max {maxSendable} sat): ";
        var amountLine = readline(prompt);
        if (amountLine == null) return;
        var amountSats = ulong.Parse(amountLine.Trim());

        bool? validateSuccessUrl = validateStr != null ? validateStr.ToLower() == "true" : null;

        var prepareResponse = await sdk.PrepareLnurlPay(request: new PrepareLnurlPayRequest(
            amount: amountSats,
            payRequest: payRequest,
            comment: comment,
            validateSuccessActionUrl: validateSuccessUrl,
            tokenIdentifier: null,
            conversionOptions: conversionOptions,
            feePolicy: feePolicy
        ));

        // Show conversion estimate if present
        if (prepareResponse.conversionEstimate != null)
        {
            var est = prepareResponse.conversionEstimate;
            Console.WriteLine($"Estimated conversion of {est.amountIn} token base units → {est.amountOut} sats with a {est.fee} token base units fee");
            var line = readline("Do you want to continue (y/n) [y]: ");
            if (line != null && line.Trim().ToLower() != "" && line.Trim().ToLower() != "y")
            {
                Console.WriteLine("Payment cancelled");
                return;
            }
        }

        Console.WriteLine($"Prepared payment: fees = {prepareResponse.feeSats} sats");
        var confirmLine = readline("Do you want to continue (y/n) [y]: ");
        if (confirmLine != null && confirmLine.Trim().ToLower() != "" && confirmLine.Trim().ToLower() != "y")
        {
            return;
        }

        var result = await sdk.LnurlPay(new LnurlPayRequest(
            prepareResponse: prepareResponse,
            idempotencyKey: idempotencyKey
        ));

        Serialization.PrintValue(result);
    }

    // --- lnurl-withdraw ---

    private static async Task HandleLnurlWithdraw(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var timeoutStr = GetFlag(args, "-t", "--timeout");
        var positional = GetPositionalArgs(args);

        if (positional.Length < 1)
        {
            Console.WriteLine("Usage: lnurl-withdraw <lnurl> [--timeout N]");
            return;
        }

        var input = await sdk.Parse(positional[0]);

        if (input is not InputType.LnurlWithdraw lnurlWithdraw)
        {
            Console.WriteLine("Error: input is not an LNURL-withdraw");
            return;
        }

        var withdrawRequest = lnurlWithdraw.v1;
        var minWithdrawable = (withdrawRequest.minWithdrawable + 999) / 1000;
        var maxWithdrawable = withdrawRequest.maxWithdrawable / 1000;
        var prompt = $"Amount to withdraw (min {minWithdrawable} sat, max {maxWithdrawable} sat): ";
        var amountLine = readline(prompt);
        if (amountLine == null) return;
        var amountSats = ulong.Parse(amountLine.Trim());

        var result = await sdk.LnurlWithdraw(request: new LnurlWithdrawRequest(
            amountSats: amountSats,
            withdrawRequest: withdrawRequest,
            completionTimeoutSecs: ParseOptionalUint(timeoutStr)
        ));

        Serialization.PrintValue(result);
    }

    // --- lnurl-auth ---

    private static async Task HandleLnurlAuth(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: lnurl-auth <lnurl>");
            return;
        }

        var input = await sdk.Parse(args[0]);

        if (input is not InputType.LnurlAuth lnurlAuth)
        {
            Console.WriteLine("Error: input is not an LNURL-auth");
            return;
        }

        var authRequest = lnurlAuth.v1;
        var action = authRequest.action ?? "auth";
        var prompt = $"Authenticate with {authRequest.domain} (action: {action})? (y/n) [y]: ";
        var line = readline(prompt);
        if (line != null && line.Trim().ToLower() != "" && line.Trim().ToLower() != "y")
        {
            return;
        }

        var result = await sdk.LnurlAuth(authRequest);
        Serialization.PrintValue(result);
    }

    // --- claim-htlc-payment ---

    private static async Task HandleClaimHtlcPayment(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: claim-htlc-payment <preimage>");
            return;
        }

        var result = await sdk.ClaimHtlcPayment(new ClaimHtlcPaymentRequest(preimage: args[0]));
        Serialization.PrintValue(result.payment);
    }

    // --- claim-deposit ---

    private static async Task HandleClaimDeposit(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var feeSatStr = GetFlag(args, "--fee-sat");
        var satPerVbyteStr = GetFlag(args, "--sat-per-vbyte");
        var recFeeLeewayStr = GetFlag(args, "--recommended-fee-leeway");
        var positional = GetPositionalArgs(args);

        if (positional.Length < 2)
        {
            Console.WriteLine("Usage: claim-deposit <txid> <vout> [--fee-sat N | --sat-per-vbyte N | --recommended-fee-leeway N]");
            return;
        }

        var txid = positional[0];
        var vout = uint.Parse(positional[1]);

        MaxFee? maxFee = null;
        if (recFeeLeewayStr != null)
        {
            if (feeSatStr != null || satPerVbyteStr != null)
            {
                Console.WriteLine("Cannot specify fee-sat or sat-per-vbyte when using recommended fee");
                return;
            }
            maxFee = new MaxFee.NetworkRecommended(leewaySatPerVbyte: ulong.Parse(recFeeLeewayStr));
        }
        else if (feeSatStr != null && satPerVbyteStr != null)
        {
            Console.WriteLine("Cannot specify both --fee-sat and --sat-per-vbyte");
            return;
        }
        else if (feeSatStr != null)
        {
            maxFee = new MaxFee.Fixed(amount: ulong.Parse(feeSatStr));
        }
        else if (satPerVbyteStr != null)
        {
            maxFee = new MaxFee.Rate(satPerVbyte: ulong.Parse(satPerVbyteStr));
        }

        var result = await sdk.ClaimDeposit(new ClaimDepositRequest(
            txid: txid,
            vout: vout,
            maxFee: maxFee
        ));
        Serialization.PrintValue(result);
    }

    // --- parse ---

    private static async Task HandleParse(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: parse <input>");
            return;
        }

        var result = await sdk.Parse(args[0]);
        Serialization.PrintValue(result);
    }

    // --- refund-deposit ---

    private static async Task HandleRefundDeposit(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var feeSatStr = GetFlag(args, "--fee-sat");
        var satPerVbyteStr = GetFlag(args, "--sat-per-vbyte");
        var positional = GetPositionalArgs(args);

        if (positional.Length < 3)
        {
            Console.WriteLine("Usage: refund-deposit <txid> <vout> <destination_address> [--fee-sat N | --sat-per-vbyte N]");
            return;
        }

        var txid = positional[0];
        var vout = uint.Parse(positional[1]);
        var destAddr = positional[2];

        Fee fee;
        if (feeSatStr != null && satPerVbyteStr != null)
        {
            Console.WriteLine("Cannot specify both --fee-sat and --sat-per-vbyte");
            return;
        }
        else if (feeSatStr != null)
        {
            fee = new Fee.Fixed(amount: ulong.Parse(feeSatStr));
        }
        else if (satPerVbyteStr != null)
        {
            fee = new Fee.Rate(satPerVbyte: ulong.Parse(satPerVbyteStr));
        }
        else
        {
            Console.WriteLine("Must specify either --fee-sat or --sat-per-vbyte");
            return;
        }

        var result = await sdk.RefundDeposit(new RefundDepositRequest(
            txid: txid,
            vout: vout,
            destinationAddress: destAddr,
            fee: fee
        ));
        Serialization.PrintValue(result);
    }

    // --- list-unclaimed-deposits ---

    private static async Task HandleListUnclaimedDeposits(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var result = await sdk.ListUnclaimedDeposits(new ListUnclaimedDepositsRequest());
        Serialization.PrintValue(result);
    }

    // --- buy-bitcoin ---

    private static async Task HandleBuyBitcoin(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var provider = GetFlag(args, "--provider") ?? "moonpay";
        var amountSatStr = GetFlag(args, "--amount-sat");
        var redirectUrl = GetFlag(args, "--redirect-url");

        BuyBitcoinRequest request;
        switch (provider.ToLower())
        {
            case "cashapp":
            case "cash_app":
            case "cash-app":
                var cashAppAmount = ParseOptionalUlong(amountSatStr);
                if (cashAppAmount is null)
                {
                    Console.WriteLine("--amount-sat is required when --provider is cashapp");
                    return;
                }
                request = new BuyBitcoinRequest.CashApp(
                    amountSats: cashAppAmount.Value
                );
                break;
            default:
                request = new BuyBitcoinRequest.Moonpay(
                    lockedAmountSat: ParseOptionalUlong(amountSatStr),
                    redirectUrl: redirectUrl
                );
                break;
        }

        var result = await sdk.BuyBitcoin(request);
        Console.WriteLine("Open this URL in a browser to complete the purchase:");
        Console.WriteLine(result.url);
    }

    // --- check-lightning-address-available ---

    private static async Task HandleCheckLightningAddress(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: check-lightning-address-available <username>");
            return;
        }

        var result = await sdk.CheckLightningAddressAvailable(
            new CheckLightningAddressRequest(username: args[0])
        );
        Serialization.PrintValue(result);
    }

    // --- get-lightning-address ---

    private static async Task HandleGetLightningAddress(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var result = await sdk.GetLightningAddress();
        if (result == null)
        {
            Console.WriteLine("No lightning address registered");
        }
        else
        {
            Serialization.PrintValue(result);
        }
    }

    // --- register-lightning-address ---

    private static async Task HandleRegisterLightningAddress(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var description = GetFlag(args, "-d", "--description");
        var transferPubkey = GetFlag(args, "--transfer-pubkey");
        var transferSignature = GetFlag(args, "--transfer-signature");
        var positional = GetPositionalArgs(args);

        if (positional.Length < 1)
        {
            Console.WriteLine("Usage: register-lightning-address <username> [-d <description>] [--transfer-pubkey <pk> --transfer-signature <sig>]");
            return;
        }

        if ((transferPubkey == null) != (transferSignature == null))
        {
            Console.WriteLine("Error: --transfer-pubkey and --transfer-signature must be provided together");
            return;
        }
        LightningAddressTransfer? transfer = transferPubkey == null
            ? null
            : new LightningAddressTransfer(pubkey: transferPubkey, signature: transferSignature!);

        var result = await sdk.RegisterLightningAddress(new RegisterLightningAddressRequest(
            username: positional[0],
            description: description,
            transfer: transfer
        ));
        Serialization.PrintValue(result);
    }

    // --- accept-lightning-address-transfer ---

    private static async Task HandleAcceptLightningAddressTransfer(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var positional = GetPositionalArgs(args);
        if (positional.Length < 1)
        {
            Console.WriteLine("Usage: accept-lightning-address-transfer <transferee_pubkey>");
            return;
        }
        var result = await sdk.AcceptLightningAddressTransfer(new AcceptLightningAddressTransferRequest(
            transfereePubkey: positional[0]
        ));
        Serialization.PrintValue(result);
    }

    // --- delete-lightning-address ---

    private static async Task HandleDeleteLightningAddress(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        await sdk.DeleteLightningAddress();
        Console.WriteLine("Lightning address deleted");
    }

    // --- list-fiat-currencies ---

    private static async Task HandleListFiatCurrencies(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var result = await sdk.ListFiatCurrencies();
        Serialization.PrintValue(result);
    }

    // --- list-fiat-rates ---

    private static async Task HandleListFiatRates(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var result = await sdk.ListFiatRates();
        Serialization.PrintValue(result);
    }

    // --- recommended-fees ---

    private static async Task HandleRecommendedFees(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var result = await sdk.RecommendedFees();
        Serialization.PrintValue(result);
    }

    // --- get-tokens-metadata ---

    private static async Task HandleGetTokensMetadata(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: get-tokens-metadata <token_id> [<token_id2> ...]");
            return;
        }

        var result = await sdk.GetTokensMetadata(new GetTokensMetadataRequest(
            tokenIdentifiers: args
        ));
        Serialization.PrintValue(result);
    }

    // --- fetch-conversion-limits ---

    private static async Task HandleFetchConversionLimits(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var fromBitcoin = HasFlag(args, "-f", "--from-bitcoin");
        var tokenId = GetFlag(args, "--token");

        // Also accept positional token identifier
        var positional = GetPositionalArgs(args);
        if (tokenId == null && positional.Length > 0)
        {
            tokenId = positional[0];
        }

        if (tokenId == null)
        {
            Console.WriteLine("Usage: fetch-conversion-limits <token_id> [--from-bitcoin]");
            return;
        }

        ConversionType convType;
        string? reqTokenId = null;
        if (fromBitcoin)
        {
            convType = new ConversionType.FromBitcoin();
            reqTokenId = tokenId;
        }
        else
        {
            convType = new ConversionType.ToBitcoin(fromTokenIdentifier: tokenId);
        }

        var result = await sdk.FetchConversionLimits(new FetchConversionLimitsRequest(
            conversionType: convType,
            tokenIdentifier: reqTokenId
        ));
        Serialization.PrintValue(result);
    }

    // --- get-user-settings ---

    private static async Task HandleGetUserSettings(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var result = await sdk.GetUserSettings();
        Serialization.PrintValue(result);
    }

    // --- set-user-settings ---

    private static async Task HandleSetUserSettings(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var privateModeStr = GetFlag(args, "-p", "--private");
        bool? sparkPrivateMode = privateModeStr != null ? privateModeStr.ToLower() == "true" : null;

        await sdk.UpdateUserSettings(new UpdateUserSettingsRequest(
            sparkPrivateModeEnabled: sparkPrivateMode
        ));
        Console.WriteLine("User settings updated");
    }

    // --- get-spark-status ---

    private static async Task HandleGetSparkStatus(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var result = await BreezSdkSparkMethods.GetSparkStatus();
        Serialization.PrintValue(result);
    }

    // -----------------------------------------------------------------------
    // ReadPaymentOptions -- interactive fee/option selection
    // -----------------------------------------------------------------------

    public static SendPaymentOptions? ReadPaymentOptions(
        SendPaymentMethod paymentMethod,
        Func<string, string?> readline)
    {
        if (paymentMethod is SendPaymentMethod.BitcoinAddress btcMethod)
        {
            var feeQuote = btcMethod.feeQuote;
            var fastFee = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat;
            var mediumFee = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat;
            var slowFee = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat;
            Console.WriteLine("Please choose payment fee:");
            Console.WriteLine($"1. Fast: {fastFee} sats");
            Console.WriteLine($"2. Medium: {mediumFee} sats");
            Console.WriteLine($"3. Slow: {slowFee} sats");

            var line = readline("Choose (1/2/3) [1]: ")?.Trim();
            if (string.IsNullOrEmpty(line)) line = "1";

            OnchainConfirmationSpeed speed = line switch
            {
                "1" => OnchainConfirmationSpeed.Fast,
                "2" => OnchainConfirmationSpeed.Medium,
                "3" => OnchainConfirmationSpeed.Slow,
                _ => throw new ArgumentException("Invalid confirmation speed")
            };

            return new SendPaymentOptions.BitcoinAddress(confirmationSpeed: speed);
        }

        if (paymentMethod is SendPaymentMethod.Bolt11Invoice bolt11Method)
        {
            if (bolt11Method.sparkTransferFeeSats != null)
            {
                Console.WriteLine("Choose payment option:");
                Console.WriteLine($"1. Spark transfer fee: {bolt11Method.sparkTransferFeeSats} sats");
                Console.WriteLine($"2. Lightning fee: {bolt11Method.lightningFeeSats} sats");

                var line = readline("Choose (1/2) [1]: ")?.Trim();
                if (string.IsNullOrEmpty(line)) line = "1";

                if (line == "1")
                {
                    return new SendPaymentOptions.Bolt11Invoice(
                        preferSpark: true,
                        completionTimeoutSecs: 0
                    );
                }
            }

            return new SendPaymentOptions.Bolt11Invoice(
                preferSpark: false,
                completionTimeoutSecs: 0
            );
        }

        if (paymentMethod is SendPaymentMethod.SparkAddress sparkMethod)
        {
            // HTLC options are only valid for Bitcoin payments, not token payments
            if (sparkMethod.tokenIdentifier != null)
            {
                return null;
            }

            var htlcLine = readline("Do you want to create an HTLC transfer? (y/n) [n]: ")?.Trim()?.ToLower();
            if (htlcLine != "y")
            {
                return null;
            }

            var hashLine = readline("Please enter the HTLC payment hash (hex string) or leave empty to generate: ")?.Trim();
            string paymentHash;
            if (string.IsNullOrEmpty(hashLine))
            {
                var preimageBytes = new byte[32];
                RandomNumberGenerator.Fill(preimageBytes);
                var preimage = Convert.ToHexString(preimageBytes).ToLowerInvariant();
                paymentHash = ComputeSha256Hex(preimageBytes);

                Console.WriteLine($"Generated preimage: {preimage}");
                Console.WriteLine($"Associated payment hash: {paymentHash}");
            }
            else
            {
                paymentHash = hashLine;
            }

            var expiryLine = readline("Please enter the HTLC expiry duration in seconds: ")?.Trim();
            if (expiryLine == null) return null;
            var expiryDurationSecs = ulong.Parse(expiryLine);

            return new SendPaymentOptions.SparkAddress(
                htlcOptions: new SparkHtlcOptions(
                    paymentHash: paymentHash,
                    expiryDurationSecs: expiryDurationSecs
                )
            );
        }

        // SparkInvoice -> no options
        return null;
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    private static string ComputeSha256Hex(byte[] data)
    {
        var hash = System.Security.Cryptography.SHA256.HashData(data);
        return Convert.ToHexString(hash).ToLowerInvariant();
    }
}
