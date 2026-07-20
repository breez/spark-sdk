using Breez.Sdk.Spark;

namespace BreezCli;

/// <summary>
/// Represents a single advanced subcommand.
/// </summary>
public class AdvancedCliCommand
{
    public required string Name { get; init; }
    public required string Description { get; init; }
    public required Func<BreezSdk, Func<string, string?>, string[], Task> Run { get; init; }
}

/// <summary>
/// Advanced subcommand names (used for REPL completion).
/// </summary>
public static class AdvancedCommandNames
{
    public static readonly string[] All =
    {
        "advanced unilateral-exit",
    };
}

/// <summary>
/// Advanced subcommand handlers.
/// </summary>
public static class AdvancedCommands
{
    /// <summary>
    /// Builds the advanced subcommand registry.
    /// </summary>
    public static Dictionary<string, AdvancedCliCommand> BuildRegistry()
    {
        return new Dictionary<string, AdvancedCliCommand>
        {
            ["unilateral-exit"] = new()
            {
                Name = "unilateral-exit",
                Description = "Build and sign a unilateral exit",
                Run = HandleUnilateralExit
            },
        };
    }

    /// <summary>
    /// Dispatches an advanced subcommand.
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
            Console.WriteLine("Advanced subcommands:");
            var names = registry.Keys.OrderBy(k => k).ToList();
            foreach (var name in names)
            {
                Console.WriteLine($"  advanced {name,-30} {registry[name].Description}");
            }
            Console.WriteLine();
            return;
        }

        var subName = args[0];
        var subArgs = args.Skip(1).ToArray();

        if (!registry.TryGetValue(subName, out var cmd))
        {
            Console.WriteLine($"Unknown advanced subcommand: {subName}. Use 'advanced help' for available commands.");
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

    private static string[] GetAllFlags(string[] args, params string[] names)
    {
        var results = new List<string>();
        for (int i = 0; i < args.Length - 1; i++)
        {
            if (names.Contains(args[i]))
            {
                results.Add(args[i + 1]);
            }
        }
        return results.ToArray();
    }

    // -----------------------------------------------------------------------
    // Advanced command handlers
    // -----------------------------------------------------------------------

    // --- unilateral-exit ---

    private static async Task HandleUnilateralExit(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var feeRateStr = GetFlag(args, "--fee-rate");
        var fundingKindStr = GetFlag(args, "--funding-kind") ?? "p2tr";
        var destination = GetFlag(args, "--destination");
        var leafIds = GetAllFlags(args, "--leaf");

        if (feeRateStr == null || destination == null)
        {
            Console.WriteLine("Usage: advanced unilateral-exit --fee-rate <N> --destination <addr> [--funding-kind p2tr|p2wpkh] [--leaf <id> ...]");
            return;
        }

        var feeRate = ulong.Parse(feeRateStr);

        CpfpFundingKind fundingKind = fundingKindStr.ToLower() switch
        {
            "p2wpkh" => new CpfpFundingKind.P2wpkh(),
            "p2tr" => new CpfpFundingKind.P2tr(),
            _ => throw new ArgumentException($"Invalid funding kind: {fundingKindStr}. Use 'p2wpkh' or 'p2tr'")
        };

        ExitLeafSelection selection = leafIds.Length == 0
            ? new ExitLeafSelection.Auto()
            : new ExitLeafSelection.Specific(leafIds: leafIds);

        var prepared = await sdk.PrepareUnilateralExit(
            request: new PrepareUnilateralExitRequest(
                feeRateSatPerVbyte: feeRate,
                fundingKind: fundingKind,
                destination: destination,
                selection: selection
            )
        );
        Serialization.PrintValue(prepared);

        if (prepared.leaves.Length == 0)
        {
            Console.WriteLine("No leaves to exit.");
            return;
        }

        var utxoLine = readline(
            "Funding UTXO(s) as txid:vout:value:pubkey (space-separated, blank to stop): ");
        if (utxoLine == null || string.IsNullOrWhiteSpace(utxoLine))
        {
            Console.WriteLine("No funding provided; showing the quote only.");
            return;
        }

        var fundingInputs = utxoLine.Trim()
            .Split(' ', StringSplitOptions.RemoveEmptyEntries)
            .Select(u => ParseCpfpInput(u, fundingKindStr.ToLower()))
            .ToArray();

        var keyLine = readline("Hex secret key for the funding UTXO(s): ");
        if (keyLine == null || string.IsNullOrWhiteSpace(keyLine))
        {
            Console.WriteLine("No key provided.");
            return;
        }

        var secretKeyBytes = Convert.FromHexString(keyLine.Trim());
        var signer = BreezSdkSparkMethods.SingleKeyCpfpSigner(secretKeyBytes);

        var response = await sdk.UnilateralExit(
            request: new UnilateralExitRequest(
                prepared: prepared,
                fundingInputs: fundingInputs
            ),
            signer: signer
        );

        PrintExitTransactions(response);
    }

    private static CpfpInput ParseCpfpInput(string s, string kind)
    {
        var parts = s.Split(':');
        if (parts.Length != 4)
        {
            throw new ArgumentException(
                $"Invalid funding UTXO '{s}', expected txid:vout:value:pubkey");
        }

        var txid = parts[0];
        var vout = uint.Parse(parts[1]);
        var value = ulong.Parse(parts[2]);
        var pubkey = parts[3];

        return kind switch
        {
            "p2wpkh" => new CpfpInput.P2wpkh(
                txid: txid, vout: vout, value: value, pubkey: pubkey),
            "p2tr" => new CpfpInput.P2tr(
                txid: txid, vout: vout, value: value, pubkey: pubkey),
            _ => throw new ArgumentException($"Invalid funding kind: {kind}")
        };
    }

    private static void PrintExitTransactions(UnilateralExitResponse response)
    {
        Console.WriteLine(
            $"Recoverable {response.recoverableValueSat} sats, " +
            $"total fee {response.totalFeeSat} sats, " +
            $"{response.transactions.Length} transaction(s):");

        for (int i = 0; i < response.transactions.Length; i++)
        {
            var tx = response.transactions[i];
            var after = tx.dependsOn.Length == 0
                ? ""
                : $", after {string.Join(",", tx.dependsOn)}";
            var csv = tx.csvTimelockBlocks != null
                ? $", csv {tx.csvTimelockBlocks} blocks"
                : "";
            Console.WriteLine(
                $"  [{i}] {tx.kind} status={tx.status} txid={tx.txid}{after}{csv}");

            if (tx.status == ConfirmationStatus.Confirmed)
            {
                Console.WriteLine("      (already confirmed, nothing to broadcast)");
                continue;
            }

            var package = tx.cpfpTxHex != null
                ? $"{tx.txHex},{tx.cpfpTxHex}"
                : tx.txHex;
            Console.WriteLine($"      Package: {package}");
        }
    }
}
