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
        "advanced recover-funds",
        "advanced check-recover-funds",
        "advanced export-unilateral-exit-state",
        "advanced import-unilateral-exit-state",
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
            ["recover-funds"] = new()
            {
                Name = "recover-funds",
                Description = "Recover the funds that left the balance, or with --all every leaf",
                Run = HandleRecoverFunds
            },
            ["check-recover-funds"] = new()
            {
                Name = "check-recover-funds",
                Description = "Read a recovery written by recover-funds back against the chain",
                Run = HandleCheckRecoverFunds
            },
            ["export-unilateral-exit-state"] = new()
            {
                Name = "export-unilateral-exit-state",
                Description = "Export the wallet's unilateral exit state to a file",
                Run = HandleExportUnilateralExitState
            },
            ["import-unilateral-exit-state"] = new()
            {
                Name = "import-unilateral-exit-state",
                Description = "Import a previously exported unilateral exit state",
                Run = HandleImportUnilateralExitState
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

    private static bool HasFlag(string[] args, params string[] names)
    {
        return args.Any(a => names.Contains(a));
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

    // --- recover-funds ---

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
                txid: txid, vout: vout, valueSats: value, pubkey: pubkey),
            "p2tr" => new CpfpInput.P2tr(
                txid: txid, vout: vout, valueSats: value, pubkey: pubkey),
            _ => throw new ArgumentException($"Invalid funding kind: {kind}")
        };
    }

    private static async Task HandleRecoverFunds(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var feeRateStr = GetFlag(args, "--fee-rate");
        var fundingKindStr = GetFlag(args, "--funding-kind") ?? "p2tr";
        var destination = GetFlag(args, "--destination");
        var all = HasFlag(args, "--all");
        var leafIds = GetAllFlags(args, "--leaf");
        var outputFile = GetFlag(args, "--output-file");

        if (feeRateStr == null || destination == null)
        {
            Console.WriteLine("Usage: advanced recover-funds --fee-rate <N> --destination <addr> [--funding-kind p2tr|p2wpkh] [--all | --leaf <id> ...] [--output-file <path>]");
            return;
        }

        if (all && leafIds.Length > 0)
        {
            Console.WriteLine("Cannot specify both --all and --leaf");
            return;
        }

        var feeRate = ulong.Parse(feeRateStr);

        CpfpFundingKind fundingKind = fundingKindStr.ToLower() switch
        {
            "p2wpkh" => new CpfpFundingKind.P2wpkh(),
            "p2tr" => new CpfpFundingKind.P2tr(),
            _ => throw new ArgumentException($"Invalid funding kind: {fundingKindStr}. Use 'p2wpkh' or 'p2tr'")
        };

        var request = new PrepareRecoverFundsRequest(
            feeRateSatPerVbyte: feeRate,
            fundingKind: fundingKind,
            destination: destination,
            selection: RecoverySelection(all, leafIds)
        );

        var prepared = await sdk.PrepareRecoverFunds(request: request);
        if (prepared.leaves.Length == 0)
        {
            Console.WriteLine("Nothing to recover.");
            PrintSkipped(prepared.skipped);
            return;
        }
        PrintQuote(prepared);
        if (outputFile == null)
        {
            Console.WriteLine(
                "Without --output-file the recovery is only printed: check-recover-funds cannot read it back.");
        }

        var fundingInputs = Array.Empty<CpfpInput>();
        CpfpSigner? signer = null;
        if (prepared.funding is { } funding)
        {
            var utxoLine = readline(
                $"Funding UTXO(s) of at least {funding.singleUtxoSats} sats, as txid:vout:value:pubkey " +
                "(space-separated; for P2TR the internal key; blank to skip the unilateral exit): ");
            if (utxoLine == null || string.IsNullOrWhiteSpace(utxoLine))
            {
                var cooperative = prepared.leaves
                    .Where(leaf => leaf.method == RecoveryMethod.Cooperative)
                    .Select(leaf => leaf.leafId)
                    .ToArray();
                if (cooperative.Length == 0)
                {
                    Console.WriteLine("Nothing to recover without funding.");
                    return;
                }
                Console.WriteLine("Recovering only the cooperative leaves:");
                prepared = await sdk.PrepareRecoverFunds(
                    request: request with { selection = new ExitLeafSelection.Specific(leafIds: cooperative) }
                );
                PrintQuote(prepared);
            }
            else
            {
                fundingInputs = utxoLine.Trim()
                    .Split(' ', StringSplitOptions.RemoveEmptyEntries)
                    .Select(u => ParseCpfpInput(u, fundingKindStr.ToLower()))
                    .ToArray();
                var keyLine = readline("Hex secret key for the funding UTXO(s): ") ?? "";
                signer = BreezSdkSparkMethods.SingleKeyCpfpSigner(Convert.FromHexString(keyLine.Trim()));
            }
        }

        var answer = readline("Sign this recovery? (y/n) [y]: ")?.Trim()?.ToLower();
        if (answer != "" && answer != "y")
        {
            return;
        }

        var response = await sdk.RecoverFunds(
            request: new RecoverFundsRequest(
                prepared: prepared,
                fundingInputs: fundingInputs
            ),
            signer: signer
        );

        PrintRecovery(response);
        if (outputFile != null)
        {
            WriteRecovery(outputFile, response);
            Console.WriteLine(
                "Next: broadcast the Ready packages. After new blocks, run check-recover-funds " +
                $"--input-file {outputFile} to see what is ready next.");
        }
        else
        {
            Console.WriteLine("Next: broadcast the Ready packages.");
        }
    }

    private static void PrintQuote(PrepareRecoverFundsResponse prepared)
    {
        Serialization.PrintValue(prepared);
        var cooperative = prepared.leaves.Count(leaf => leaf.method == RecoveryMethod.Cooperative);
        Console.WriteLine(
            $"{prepared.leaves.Length} leaf(s), {cooperative} cooperative and " +
            $"{prepared.leaves.Length - cooperative} unilateral: " +
            $"recovering {prepared.recoverableValueSats} sats for {prepared.totalFeeSats} sats in fees");
        PrintSkipped(prepared.skipped);
    }

    private static void PrintSkipped(SkippedLeaf[] skipped)
    {
        foreach (var leaf in skipped)
        {
            var reason = leaf.reason switch
            {
                SkippedLeafReason.FeeExceedsValue => "recovering it costs too much at this fee rate",
                SkippedLeafReason.FundsNotFound => "its funds were not found on-chain",
                SkippedLeafReason.Unverified => "its funds could not be looked up",
                SkippedLeafReason.NotRecoverable nr => nr.message,
                _ => leaf.reason.ToString()
            };
            Console.WriteLine($"Left out: leaf {leaf.leafId} ({leaf.valueSats} sats): {reason}");
        }
    }

    private static ExitLeafSelection RecoverySelection(bool all, string[] leafIds)
    {
        if (all)
        {
            return new ExitLeafSelection.All();
        }
        if (leafIds.Length == 0)
        {
            return new ExitLeafSelection.RecoverableOnly();
        }
        return new ExitLeafSelection.Specific(leafIds: leafIds);
    }

    // --- check-recover-funds ---

    private static async Task HandleCheckRecoverFunds(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var inputFile = GetFlag(args, "--input-file");
        var outputFile = GetFlag(args, "--output-file");

        if (inputFile == null)
        {
            Console.WriteLine("Usage: advanced check-recover-funds --input-file <path> [--output-file <path>]");
            return;
        }

        var recovery = ReadRecovery(inputFile);
        var checked_ = await sdk.CheckRecoverFunds(
            request: new CheckRecoverFundsRequest(recovery: recovery)
        );
        Console.WriteLine($"Verdict: {checked_.verdict}");
        if (checked_.verdict is RecoveryVerdict.Redo)
        {
            Console.WriteLine($"  (this recovery cannot finish: run {RedoCommand(checked_.recovery)})");
        }
        PrintRecovery(checked_.recovery);
        WriteRecovery(outputFile ?? inputFile, checked_.recovery);
    }

    private static string RedoCommand(RecoverFundsResponse recovery)
    {
        var leaves = string.Concat(recovery.leaves.Select(leaf => $" --leaf {leaf.leafId}"));
        return $"recover-funds --fee-rate {recovery.feeRateSatPerVbyte} " +
               $"--destination {recovery.destination}{leaves}";
    }

    // --- export-unilateral-exit-state ---

    private static async Task HandleExportUnilateralExitState(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var outputFile = GetFlag(args, "--output-file");
        if (outputFile == null)
        {
            Console.WriteLine("Usage: advanced export-unilateral-exit-state --output-file <path>");
            return;
        }

        var exported = await sdk.ExportUnilateralExitState();
        await File.WriteAllTextAsync(outputFile, exported.exitState);
        Console.WriteLine($"Wrote {exported.exitState.Length} bytes to {outputFile}");
    }

    // --- import-unilateral-exit-state ---

    private static async Task HandleImportUnilateralExitState(BreezSdk sdk, Func<string, string?> readline, string[] args)
    {
        var inputFile = GetFlag(args, "--input-file");
        if (inputFile == null)
        {
            Console.WriteLine("Usage: advanced import-unilateral-exit-state --input-file <path>");
            return;
        }

        var exitState = await File.ReadAllTextAsync(inputFile);
        var imported = await sdk.ImportUnilateralExitState(
            request: new ImportUnilateralExitStateRequest(exitState: exitState)
        );
        Console.WriteLine(
            $"Imported {imported.importedLeaves} leaf(s), " +
            $"skipped {imported.skippedForeignLeaves} leaf(s) from a different wallet " +
            $"and {imported.skippedConflictingLeaves} that disagree with what this wallet holds, " +
            $"left out the exit data of {imported.skippedChains} leaf(s)");
    }

    private static RecoverFundsResponse ReadRecovery(string path)
    {
        var json = File.ReadAllText(path);
        return Serialization.Deserialize<RecoverFundsResponse>(json);
    }

    private static void WriteRecovery(string path, RecoverFundsResponse recovery)
    {
        var temporary = path + ".tmp";
        File.WriteAllText(temporary, Serialization.SerializePretty(recovery));
        File.Move(temporary, path, overwrite: true);
        Console.WriteLine($"Wrote the recovery to {path}");
    }

    private static void PrintRecovery(RecoverFundsResponse response)
    {
        Console.WriteLine(
            $"Recoverable {response.recoverableValueSats} sats, " +
            $"total fee {response.totalFeeSats} sats " +
            $"(cooperative {response.cooperativeFeeSats}, cpfp {response.cpfpFeeSats}, " +
            $"fanout {response.fanoutFeeSats}, sweep {response.sweepFeeSats}), " +
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
            var node = tx.nodeId != null
                ? $" node={tx.nodeId}"
                : "";
            Console.WriteLine(
                $"  [{i}] {tx.kind}{node} status={tx.status} txid={tx.txid}{after}{csv}");

            switch (tx.status)
            {
                case ExitTransactionStatus.Confirmed confirmed:
                    if (confirmed.blockHeight != null)
                    {
                        Console.WriteLine(
                            $"      (confirmed in block {confirmed.blockHeight}, nothing to broadcast)");
                    }
                    else
                    {
                        Console.WriteLine("      (already confirmed, nothing to broadcast)");
                    }
                    continue;
                case ExitTransactionStatus.WaitingForDependencies:
                    Console.WriteLine("      (waiting on the transactions it depends on)");
                    break;
                case ExitTransactionStatus.WaitingForTimelock wft:
                    if (wft.spendableAtHeight != null)
                    {
                        Console.WriteLine(
                            $"      (waiting for its timelock, until block {wft.spendableAtHeight})");
                    }
                    else
                    {
                        Console.WriteLine("      (waiting for its timelock)");
                    }
                    break;
                case ExitTransactionStatus.Ready:
                case ExitTransactionStatus.Unverified:
                    break;
            }

            var package = tx.cpfpTxHex != null
                ? $"{tx.txHex},{tx.cpfpTxHex}"
                : tx.txHex;
            Console.WriteLine($"      Package: {package}");
        }

        if (response.failed.Length > 0)
        {
            Console.WriteLine($"Not recovered, {response.failed.Length} leaf(s):");
        }
        foreach (var failure in response.failed)
        {
            Console.WriteLine(
                $"  leaf {failure.leafId} " +
                $"(output {failure.outputTxid}:{failure.outputVout}): " +
                CooperativeRecoveryErrorMessage(failure.error));
        }
    }

    private static string CooperativeRecoveryErrorMessage(CooperativeRecoveryError error)
    {
        return error switch
        {
            CooperativeRecoveryError.ReplacementFeeTooLow e =>
                "A recovery of this output is already on the network: replacing it takes at least " +
                $"{e.requiredFeeSats} sats or {e.requiredFeeRateSatPerVbyte} sats/vbyte",
            CooperativeRecoveryError.OperatorsUnavailable e => $"Operators unavailable: {e.message}",
            CooperativeRecoveryError.Generic e => $"Generic error: {e.message}",
            _ => error.ToString()
        };
    }
}
