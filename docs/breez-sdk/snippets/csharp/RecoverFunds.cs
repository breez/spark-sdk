using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class RecoverFunds
    {
        async Task FetchRecoverableFunds(BreezSdk sdk)
        {
            // ANCHOR: recoverable-funds
            var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));

            if (info.recoverableFundsSats > 0)
            {
                Console.WriteLine($"{info.recoverableFundsSats} sats can be recovered on-chain");
            }
            // ANCHOR_END: recoverable-funds
        }

        async Task<PrepareRecoverFundsResponse> QuoteRecovery(BreezSdk sdk)
        {
            // ANCHOR: prepare-recover-funds
            var quote = await sdk.PrepareRecoverFunds(
                request: new PrepareRecoverFundsRequest(
                    feeRateSatPerVbyte: 2,
                    fundingKind: new CpfpFundingKind.P2wpkh(),
                    destination: "bc1q...your-destination-address",
                    selection: new ExitLeafSelection.RecoverableOnly()
                )
            );

            if (quote.leaves.Length == 0)
            {
                Console.WriteLine("Nothing to recover");
                return quote;
            }
            foreach (var leaf in quote.leaves)
            {
                Console.WriteLine($"{leaf.leafId}: {leaf.valueSats} sats, {leaf.method}");
            }
            Console.WriteLine($"Recovering {quote.recoverableValueSats} sats " +
                $"for {quote.totalFeeSats} sats in fees");
            if (quote.funding is { } funding)
            {
                Console.WriteLine($"Fund one UTXO of at least {funding.singleUtxoSats} sats");
            }
            // ANCHOR_END: prepare-recover-funds
            return quote;
        }

        async Task RecoverCooperatively(BreezSdk sdk, PrepareRecoverFundsResponse quote)
        {
            // ANCHOR: recover-cooperatively
            // A quote that asks for funding holds a unilateral exit: quote the
            // cooperative leaves alone to recover them without it.
            if (quote.funding != null)
            {
                var leafIds = quote.leaves
                    .Where(leaf => leaf.method == RecoveryMethod.Cooperative)
                    .Select(leaf => leaf.leafId)
                    .ToArray();
                if (leafIds.Length == 0)
                {
                    return;
                }
                quote = await sdk.PrepareRecoverFunds(
                    request: new PrepareRecoverFundsRequest(
                        feeRateSatPerVbyte: quote.feeRateSatPerVbyte,
                        destination: quote.destination,
                        selection: new ExitLeafSelection.Specific(leafIds: leafIds)
                    )
                );
            }
            var response = await sdk.RecoverFunds(
                request: new RecoverFundsRequest(prepared: quote),
                signer: null
            );

            // Keep the whole response: CheckRecoverFunds follows the recovery from it.
            foreach (var tx in response.transactions)
            {
                Console.WriteLine($"Broadcast {tx.txid}: {tx.txHex}");
            }
            foreach (var failure in response.failed)
            {
                Console.WriteLine($"Leaf {failure.leafId} was not recovered: {failure.error}");
            }
            // ANCHOR_END: recover-cooperatively
        }

        async Task<RecoverFundsResponse> RecoverWithFunding(
            BreezSdk sdk,
            PrepareRecoverFundsResponse quote)
        {
            // ANCHOR: recover-funds
            var secretKeyBytes = Convert.FromHexString("your-secret-key-hex");
            var signer = BreezSdkSparkMethods.SingleKeyCpfpSigner(secretKeyBytes);

            var response = await sdk.RecoverFunds(
                request: new RecoverFundsRequest(
                    prepared: quote,
                    fundingInputs: new CpfpInput[]
                    {
                        new CpfpInput.P2wpkh(
                            txid: "your-utxo-txid",
                            vout: 0,
                            valueSats: 50_000,
                            pubkey: "your-compressed-pubkey-hex"
                        )
                    }
                ),
                signer: signer
            );

            // Keep the whole response: CheckRecoverFunds follows the recovery from it.
            foreach (var tx in response.transactions)
            {
                if (tx.csvTimelockBlocks is uint blocks)
                {
                    Console.WriteLine(
                        $"{tx.txid}: wait {blocks} blocks after its parents confirm");
                }
            }
            // ANCHOR_END: recover-funds
            return response;
        }

        async Task CheckRecovery(BreezSdk sdk, RecoverFundsResponse stored)
        {
            // ANCHOR: check-recover-funds
            var checkedRecovery = await sdk.CheckRecoverFunds(
                request: new CheckRecoverFundsRequest(recovery: stored)
            );

            // Store this one in place of the one you had.
            var recovery = checkedRecovery.recovery;

            switch (checkedRecovery.verdict)
            {
                case RecoveryVerdict.Valid:
                    foreach (var tx in recovery.transactions)
                    {
                        if (tx.status is ExitTransactionStatus.Ready)
                        {
                            Console.WriteLine($"ready to broadcast: {tx.txid}");
                        }
                    }
                    break;
                case RecoveryVerdict.Done:
                    Console.WriteLine("Every transaction confirmed: the recovery is done");
                    break;
                case RecoveryVerdict.Redo { reason: var reason }:
                    // Quote and build again, naming the same leaves. Pass
                    // recovery.fundingInputs back and the SDK follows them to whatever
                    // they have become.
                    Console.WriteLine($"Build the recovery again: {reason}");
                    break;
            }
            // ANCHOR_END: check-recover-funds
        }

        async Task<string> BackUpExitState(BreezSdk sdk)
        {
            // ANCHOR: export-exit-state
            var exported = await sdk.ExportUnilateralExitState();

            // Keep the state somewhere the wallet's own storage cannot take with it.
            Console.WriteLine($"Exit state is {exported.exitState.Length} bytes");
            // ANCHOR_END: export-exit-state
            return exported.exitState;
        }

        async Task RestoreExitState(BreezSdk sdk, string exitState)
        {
            // ANCHOR: import-exit-state
            var imported = await sdk.ImportUnilateralExitState(
                request: new ImportUnilateralExitStateRequest(exitState: exitState)
            );

            Console.WriteLine($"Imported {imported.importedLeaves} leaves, " +
                $"skipped {imported.skippedForeignLeaves}");
            // ANCHOR_END: import-exit-state
        }

        async Task CollectExitData(BreezSdk sdk)
        {
            // ANCHOR: sync-exit-data
            // With automatic collection off, an explicit sync is what collects the data
            // a unilateral exit needs, and it waits for the collection to finish. Needs
            // the Spark operators reachable, so run it on a schedule rather than at the
            // moment an exit is needed.
            await sdk.SyncWallet(request: new SyncWalletRequest());
            // ANCHOR_END: sync-exit-data
        }

        // ANCHOR: custom-cpfp-signer
        class MyFundingSigner : CpfpSigner
        {
            public async Task<byte[]> SignPsbt(byte[] psbtBytes)
            {
                var signedPsbtBytes = await SignWithFundingKeys(psbtBytes);
                return signedPsbtBytes;
            }

            async Task<byte[]> SignWithFundingKeys(byte[] psbtBytes)
            {
                return await Task.FromResult(psbtBytes);
            }
        }
        // ANCHOR_END: custom-cpfp-signer
    }
}
