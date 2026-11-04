using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezCli;

/// <summary>
/// Represents a single bridging subcommand.
/// </summary>
public class BridgingCliCommand
{
    public required string Name { get; init; }
    public required string Description { get; init; }
    public required Func<BreezSdk, Func<string, string?>, string[], Task> Run { get; init; }
}

/// <summary>
/// Bridging subcommand names (used for REPL completion).
/// </summary>
public static class BridgingCommandNames
{
    public static readonly string[] All =
    {
        "bridging create",
        "bridging from-cash-app",
        "bridging to-cash-app",
        "bridging status",
    };
}

/// <summary>
/// Bridging subcommand handlers.
/// </summary>
public static class BridgingCommands
{
    /// <summary>
    /// Builds the bridging subcommand registry.
    /// </summary>
    public static Dictionary<string, BridgingCliCommand> BuildRegistry()
    {
        return new Dictionary<string, BridgingCliCommand>
        {
            ["create"] = new()
            {
                Name = "create",
                Description = "Create a bridge that pays a recipient from another chain or over Lightning",
                Run = HandleCreate
            },
            ["from-cash-app"] = new()
            {
                Name = "from-cash-app",
                Description = "Bridge from Cash App to USDC/USDT on an external chain",
                Run = HandleFromCashApp
            },
            ["to-cash-app"] = new()
            {
                Name = "to-cash-app",
                Description = "Bridge USDC/USDT from an external chain to a Cash App user",
                Run = HandleToCashApp
            },
            ["status"] = new()
            {
                Name = "status",
                Description = "Read a bridge's status",
                Run = HandleStatus
            },
        };
    }

    /// <summary>
    /// Dispatches a bridging subcommand.
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
            Console.WriteLine("Bridging subcommands:");
            var names = registry.Keys.OrderBy(k => k).ToList();
            foreach (var name in names)
            {
                Console.WriteLine($"  bridging {name,-30} {registry[name].Description}");
            }
            Console.WriteLine();
            return;
        }

        var subName = args[0];
        var subArgs = args.Skip(1).ToArray();

        if (!registry.TryGetValue(subName, out var cmd))
        {
            Console.WriteLine($"Unknown bridging subcommand: {subName}. Use 'bridging help' for available commands.");
            return;
        }

        await cmd.Run(sdk, readline, subArgs);
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

    private static readonly HashSet<string> BooleanFlags = new()
    {
        "--fees-included"
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
                    continue;
                }
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

    private static uint? ParseOptionalUint(string? value)
    {
        if (value == null) return null;
        return uint.Parse(value);
    }

    // -----------------------------------------------------------------------
    // Bridging command handlers
    // -----------------------------------------------------------------------

    // --- create ---

    private static async Task HandleCreate(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var positional = GetPositionalArgs(args);
        var amountStr = GetFlag(args, "--amount");
        var refundAddress = GetFlag(args, "--refund-address");
        var feesIncluded = HasFlag(args, "--fees-included");
        var maxSlippageStr = GetFlag(args, "--max-slippage-bps");

        if (positional.Length < 1 || amountStr == null)
        {
            Console.WriteLine("Usage: bridging create <recipient> --amount <amount> [--refund-address <addr>] [--fees-included] [--max-slippage-bps <bps>]");
            return;
        }

        var recipient = positional[0];
        var client = sdk.GetBridgingClient();

        var routesResponse = await client.GetRoutes(new GetBridgeRoutesRequest(
            recipient: recipient
        ));
        var route = ChooseBridgeRoute(readline, routesResponse.routes);
        if (route == null) return;

        FeePolicy? feePolicy = feesIncluded ? FeePolicy.FeesIncluded : null;

        var response = await client.CreateBridge(new CreateBridgeRequest(
            recipient: recipient,
            route: route,
            amount: BigInteger.Parse(amountStr),
            refundAddress: refundAddress,
            feePolicy: feePolicy,
            maxSlippageBps: ParseOptionalUint(maxSlippageStr)
        ));

        Console.WriteLine("Ask the payer to pay:");
        Console.WriteLine(response.paymentRequest);
        Serialization.PrintValue(response);
        PrintBridgeTracking(response);
    }

    // --- from-cash-app ---

    private static async Task HandleFromCashApp(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var positional = GetPositionalArgs(args);
        var amountStr = GetFlag(args, "--amount");
        var feesIncluded = HasFlag(args, "--fees-included");
        var maxSlippageStr = GetFlag(args, "--max-slippage-bps");

        if (positional.Length < 1 || amountStr == null)
        {
            Console.WriteLine("Usage: bridging from-cash-app <recipient> --amount <amount> [--fees-included] [--max-slippage-bps <bps>]");
            return;
        }

        var recipient = positional[0];
        var client = sdk.GetBridgingClient();

        var routesResponse = await client.GetRoutes(new GetBridgeRoutesRequest(
            recipient: recipient
        ));
        var routes = routesResponse.routes
            .Where(r => r.source.chain == "lightning")
            .ToArray();
        var route = ChooseBridgeRoute(readline, routes);
        if (route == null) return;

        FeePolicy? feePolicy = feesIncluded ? FeePolicy.FeesIncluded : null;

        var response = await client.BridgeFromCashApp(new CreateBridgeRequest(
            recipient: recipient,
            route: route,
            amount: BigInteger.Parse(amountStr),
            refundAddress: null,
            feePolicy: feePolicy,
            maxSlippageBps: ParseOptionalUint(maxSlippageStr)
        ));

        var info = response.info;
        Console.WriteLine("Open this URL in a browser to complete the purchase:");
        Console.WriteLine(response.paymentRequest);
        var serviceFeeDenom = ServiceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals);
        Console.WriteLine(
            $"Deposit ~{info.depositAmount} sats; recipient receives ~{info.expectedReceivedAmount} {info.destinationAsset}, " +
            $"service fee {info.serviceFeeAmount} {serviceFeeDenom}, expires {info.expiresAt}");
        PrintBridgeTracking(response);
    }

    // --- to-cash-app ---

    private static async Task HandleToCashApp(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var positional = GetPositionalArgs(args);
        var amountStr = GetFlag(args, "--amount");
        var refundAddress = GetFlag(args, "--refund-address");
        var feesIncluded = HasFlag(args, "--fees-included");
        var maxSlippageStr = GetFlag(args, "--max-slippage-bps");

        if (positional.Length < 1 || amountStr == null || refundAddress == null)
        {
            Console.WriteLine("Usage: bridging to-cash-app <recipient> --amount <amount> --refund-address <address> [--fees-included] [--max-slippage-bps <bps>]");
            return;
        }

        var recipient = positional[0];
        var client = sdk.GetBridgingClient();

        var routesResponse = await client.GetRoutes(new GetBridgeRoutesRequest(
            recipient: recipient
        ));
        var route = ChooseBridgeRoute(readline, routesResponse.routes);
        if (route == null) return;

        FeePolicy? feePolicy = feesIncluded ? FeePolicy.FeesIncluded : null;

        var response = await client.BridgeToCashApp(new CreateBridgeRequest(
            recipient: recipient,
            route: route,
            amount: BigInteger.Parse(amountStr),
            refundAddress: refundAddress,
            feePolicy: feePolicy,
            maxSlippageBps: ParseOptionalUint(maxSlippageStr)
        ));

        var info = response.info;
        Console.WriteLine("Ask the payer to pay:");
        Console.WriteLine(response.paymentRequest);
        var serviceFeeDenom = ServiceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals);
        Console.WriteLine(
            $"Deposit {info.depositAmount} {route.source.asset} on {route.source.chain} to {info.depositAddress}. " +
            $"Recipient receives ~{info.expectedReceivedAmount} sats, " +
            $"service fee {info.serviceFeeAmount} {serviceFeeDenom}, expires at {info.expiresAt}");
        PrintBridgeTracking(response);
    }

    // --- status ---

    private static async Task HandleStatus(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var positional = GetPositionalArgs(args);
        var statusToken = GetFlag(args, "--status-token");

        if (positional.Length < 1)
        {
            Console.WriteLine("Usage: bridging status <bridge_id> [--status-token <token>]");
            return;
        }

        var client = sdk.GetBridgingClient();
        var response = await client.GetBridgeStatus(new GetBridgeStatusRequest(
            bridgeId: positional[0],
            statusToken: statusToken
        ));
        Serialization.PrintValue(response);
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    private static BridgeRoutePair? ChooseBridgeRoute(
        Func<string, string?> readline,
        BridgeRoutePair[] routes)
    {
        if (routes.Length == 0)
        {
            Console.Error.WriteLine("No bridge routes available");
            return null;
        }

        if (routes.Length == 1)
        {
            var route = routes[0];
            Console.WriteLine($"Auto-selected route: {RouteLabel(route)}");
            return route;
        }

        Console.WriteLine("Available bridge routes:");
        for (int i = 0; i < routes.Length; i++)
        {
            Console.WriteLine($"  {i + 1}. {RouteLabel(routes[i])}");
        }

        var line = readline("Select route: ");
        if (line == null || !int.TryParse(line.Trim(), out int choice) || choice < 1 || choice > routes.Length)
        {
            Console.Error.WriteLine("Invalid selection");
            return null;
        }

        return routes[choice - 1];
    }

    private static string RouteLabel(BridgeRoutePair route)
    {
        return $"{SideLabel(route.source)} -> {SideLabel(route.destination)} [{route.provider}]";
    }

    private static string SideLabel(BridgeRoutePairSide side)
    {
        var addr = MaybeTruncateAddress(side.contractAddress);
        return $"{side.asset} on {side.chain}{addr}";
    }

    private static string MaybeTruncateAddress(string? addr)
    {
        if (addr == null) return "";
        if (addr.Length > 12)
        {
            return $" ({addr[..6]}...{addr[^6..]})";
        }
        return $" ({addr})";
    }

    private static void PrintBridgeTracking(CreateBridgeResponse response)
    {
        var token = response.statusToken != null
            ? $" --status-token {response.statusToken}"
            : "";
        Console.WriteLine($"Track it with: bridging status {response.bridgeId}{token}");
    }

    private static string ServiceFeeDenomination(string? asset, uint? decimals)
    {
        if (asset == null) return "sats";
        if (decimals != null) return $"{asset} ({decimals} decimals)";
        return asset;
    }
}
