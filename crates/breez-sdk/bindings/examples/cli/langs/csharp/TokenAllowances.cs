using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezCli;

public static class TokenAllowanceCommandNames
{
    public static readonly string[] All =
    {
        "token-allowances create",
        "token-allowances revoke",
        "token-allowances list",
    };
}

public static class TokenAllowanceCommands
{
    public static async Task DispatchCommand(string[] args, BreezSdk sdk, Func<string, string?> readline)
    {
        if (args.Length == 0)
        {
            PrintHelp();
            return;
        }

        var subcommand = args[0];
        var subArgs = args.Skip(1).ToArray();

        switch (subcommand)
        {
            case "create":
                await HandleCreate(sdk, subArgs);
                break;
            case "revoke":
                await HandleRevoke(sdk, subArgs);
                break;
            case "list":
                await HandleList(sdk, subArgs);
                break;
            case "help":
                PrintHelp();
                break;
            default:
                Console.WriteLine($"Unknown token-allowances subcommand: {subcommand}. Type 'token-allowances help' for details.");
                break;
        }
    }

    private static void PrintHelp()
    {
        Console.WriteLine();
        Console.WriteLine("Token allowance commands:");
        Console.WriteLine($"  {"create",-20} Grant a spender an allowance over this wallet's tokens");
        Console.WriteLine($"  {"revoke",-20} Revoke a token allowance this wallet granted");
        Console.WriteLine($"  {"list",-20} List token allowances this wallet granted (owner) or received (spender)");
        Console.WriteLine($"  {"help",-20} Show this help message");
        Console.WriteLine();
    }

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

    private static string[] GetAllFlags(string[] args, params string[] names)
    {
        var values = new List<string>();
        for (int i = 0; i < args.Length - 1; i++)
        {
            if (names.Contains(args[i]))
            {
                values.Add(args[i + 1]);
            }
        }
        return values.ToArray();
    }

    private static bool HasFlag(string[] args, params string[] names)
    {
        return args.Any(a => names.Contains(a));
    }

    private static readonly HashSet<string> BooleanFlags = new()
    {
        "--include-inactive"
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

    private static TokenAllowanceLimit ParseLimit(string value)
    {
        if (value.Equals("unlimited", StringComparison.OrdinalIgnoreCase))
        {
            return new TokenAllowanceLimit.Unlimited();
        }
        return new TokenAllowanceLimit.Amount(amount: BigInteger.Parse(value));
    }

    // --- create ---

    private static async Task HandleCreate(BreezSdk sdk, string[] args)
    {
        var maxPerPayment = GetFlag(args, "--max-per-payment");
        var maxTotal = GetFlag(args, "--max-total");
        var recipientStrs = GetAllFlags(args, "-r", "--recipient");
        var positional = GetPositionalArgs(args);

        if (positional.Length < 3 || maxPerPayment == null || maxTotal == null)
        {
            Console.WriteLine("Usage: token-allowances create <spender_address> <token_identifier> <expiry_time> --max-per-payment <limit> --max-total <limit> [-r <recipient> ...]");
            Console.WriteLine("  limit: an amount in token base units, or 'unlimited'");
            return;
        }

        var spenderAddress = positional[0];
        var tokenIdentifier = positional[1];
        var expiryTime = ulong.Parse(positional[2]);

        var response = await sdk.CreateTokenAllowance(
            request: new CreateTokenAllowanceRequest(
                spenderAddress: spenderAddress,
                tokenIdentifier: tokenIdentifier,
                maxPerPayment: ParseLimit(maxPerPayment),
                maxTotal: ParseLimit(maxTotal),
                expiryTime: expiryTime,
                allowedRecipients: recipientStrs
            )
        );
        Serialization.PrintValue(response.allowance);
    }

    // --- revoke ---

    private static async Task HandleRevoke(BreezSdk sdk, string[] args)
    {
        var positional = GetPositionalArgs(args);
        if (positional.Length < 1)
        {
            Console.WriteLine("Usage: token-allowances revoke <allowance_id>");
            return;
        }

        await sdk.RevokeTokenAllowance(
            request: new RevokeTokenAllowanceRequest(allowanceId: positional[0])
        );
        Console.WriteLine("Token allowance revoked");
    }

    // --- list ---

    private static async Task HandleList(BreezSdk sdk, string[] args)
    {
        var counterpartyAddress = GetFlag(args, "--counterparty-address");
        var tokenIdentifier = GetFlag(args, "--token-identifier");
        var includeInactive = HasFlag(args, "--include-inactive");
        var offsetStr = GetFlag(args, "--offset");
        var limitStr = GetFlag(args, "--limit");
        var positional = GetPositionalArgs(args);

        var roleStr = positional.Length > 0 ? positional[0] : "owner";
        TokenAllowanceRole role = roleStr.ToLower() switch
        {
            "owner" => TokenAllowanceRole.Owner,
            "spender" => TokenAllowanceRole.Spender,
            _ => throw new ArgumentException($"Invalid role '{roleStr}'. Expected 'owner' or 'spender'")
        };

        uint? offset = offsetStr != null ? uint.Parse(offsetStr) : null;
        uint? limit = limitStr != null ? uint.Parse(limitStr) : null;

        var response = await sdk.ListTokenAllowances(
            request: new ListTokenAllowancesRequest(
                role: role,
                counterpartyAddress: counterpartyAddress,
                tokenIdentifier: tokenIdentifier,
                includeInactive: includeInactive ? true : null,
                offset: offset,
                limit: limit
            )
        );
        Serialization.PrintValue(response.allowances);
    }
}
