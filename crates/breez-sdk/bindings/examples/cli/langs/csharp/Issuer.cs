using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezCli;

/// <summary>
/// Represents a single issuer subcommand.
/// </summary>
public class IssuerCliCommand
{
    public required string Name { get; init; }
    public required string Description { get; init; }
    public required Func<TokenIssuer, Func<string, string?>, string[], Task> Run { get; init; }
}

/// <summary>
/// Issuer subcommand names (used for REPL completion).
/// </summary>
public static class IssuerCommandNames
{
    public static readonly string[] All =
    {
        "issuer token-balance",
        "issuer token-metadata",
        "issuer create-token",
        "issuer mint-token",
        "issuer burn-token",
        "issuer freeze-token",
        "issuer unfreeze-token",
    };
}

/// <summary>
/// Token issuer subcommand handlers.
/// </summary>
public static class IssuerCommands
{
    /// <summary>
    /// Builds the issuer subcommand registry.
    /// </summary>
    public static Dictionary<string, IssuerCliCommand> BuildRegistry()
    {
        return new Dictionary<string, IssuerCliCommand>
        {
            ["token-balance"] = new()
            {
                Name = "token-balance",
                Description = "Get issuer token balance",
                Run = HandleTokenBalance
            },
            ["token-metadata"] = new()
            {
                Name = "token-metadata",
                Description = "Get issuer token metadata",
                Run = HandleTokenMetadata
            },
            ["create-token"] = new()
            {
                Name = "create-token",
                Description = "Create a new issuer token",
                Run = HandleCreateToken
            },
            ["mint-token"] = new()
            {
                Name = "mint-token",
                Description = "Mint supply of the issuer token",
                Run = HandleMintToken
            },
            ["burn-token"] = new()
            {
                Name = "burn-token",
                Description = "Burn supply of the issuer token",
                Run = HandleBurnToken
            },
            ["freeze-token"] = new()
            {
                Name = "freeze-token",
                Description = "Freeze tokens at an address",
                Run = HandleFreezeToken
            },
            ["unfreeze-token"] = new()
            {
                Name = "unfreeze-token",
                Description = "Unfreeze tokens at an address",
                Run = HandleUnfreezeToken
            },
        };
    }

    /// <summary>
    /// Dispatches an issuer subcommand.
    /// </summary>
    public static async Task DispatchCommand(
        string[] args,
        TokenIssuer tokenIssuer,
        Func<string, string?> readline)
    {
        var registry = BuildRegistry();

        if (args.Length == 0 || args[0] == "help")
        {
            Console.WriteLine();
            Console.WriteLine("Issuer subcommands:");
            var names = registry.Keys.OrderBy(k => k).ToList();
            foreach (var name in names)
            {
                Console.WriteLine($"  issuer {name,-30} {registry[name].Description}");
            }
            Console.WriteLine();
            return;
        }

        var subName = args[0];
        var subArgs = args.Skip(1).ToArray();

        if (!registry.TryGetValue(subName, out var cmd))
        {
            Console.WriteLine($"Unknown issuer subcommand: {subName}. Use 'issuer help' for available commands.");
            return;
        }

        await cmd.Run(tokenIssuer, readline, subArgs);
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

    // -----------------------------------------------------------------------
    // Issuer command handlers
    // -----------------------------------------------------------------------

    // --- token-balance ---

    private static async Task HandleTokenBalance(TokenIssuer issuer, Func<string, string?> readline, string[] args)
    {
        var result = await issuer.GetIssuerTokenBalance();
        Serialization.PrintValue(result);
    }

    // --- token-metadata ---

    private static async Task HandleTokenMetadata(TokenIssuer issuer, Func<string, string?> readline, string[] args)
    {
        var result = await issuer.GetIssuerTokenMetadata();
        Serialization.PrintValue(result);
    }

    // --- create-token ---

    private static async Task HandleCreateToken(TokenIssuer issuer, Func<string, string?> readline, string[] args)
    {
        var name = GetFlag(args, "--name");
        var ticker = GetFlag(args, "--ticker");
        var decimalsStr = GetFlag(args, "--decimals") ?? "6";
        var freezable = HasFlag(args, "-f", "--freezable");
        var maxSupplyStr = GetFlag(args, "--max-supply");

        if (name == null || ticker == null)
        {
            Console.WriteLine("Usage: issuer create-token --name <name> --ticker <ticker> [--decimals N] [--freezable] [--max-supply N]");
            return;
        }

        var decimals = uint.Parse(decimalsStr);
        BigInteger maxSupply = maxSupplyStr != null ? BigInteger.Parse(maxSupplyStr) : BigInteger.Zero;

        var result = await issuer.CreateIssuerToken(new CreateIssuerTokenRequest(
            name: name,
            ticker: ticker,
            decimals: decimals,
            isFreezable: freezable,
            maxSupply: maxSupply
        ));
        Serialization.PrintValue(result);
    }

    // --- mint-token ---

    private static async Task HandleMintToken(TokenIssuer issuer, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: issuer mint-token <amount>");
            return;
        }

        var amount = BigInteger.Parse(args[0]);
        var result = await issuer.MintIssuerToken(new MintIssuerTokenRequest(amount: amount));
        Serialization.PrintValue(result);
    }

    // --- burn-token ---

    private static async Task HandleBurnToken(TokenIssuer issuer, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: issuer burn-token <amount>");
            return;
        }

        var amount = BigInteger.Parse(args[0]);
        var result = await issuer.BurnIssuerToken(new BurnIssuerTokenRequest(amount: amount));
        Serialization.PrintValue(result);
    }

    // --- freeze-token ---

    private static async Task HandleFreezeToken(TokenIssuer issuer, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: issuer freeze-token <address>");
            return;
        }

        var result = await issuer.FreezeIssuerToken(new FreezeIssuerTokenRequest(address: args[0]));
        Serialization.PrintValue(result);
    }

    // --- unfreeze-token ---

    private static async Task HandleUnfreezeToken(TokenIssuer issuer, Func<string, string?> readline, string[] args)
    {
        if (args.Length < 1)
        {
            Console.WriteLine("Usage: issuer unfreeze-token <address>");
            return;
        }

        var result = await issuer.UnfreezeIssuerToken(new UnfreezeIssuerTokenRequest(address: args[0]));
        Serialization.PrintValue(result);
    }
}
