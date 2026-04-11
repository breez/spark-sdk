using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class UnilateralExit
    {
        async Task ListLeavesForExit(BreezSdk sdk)
        {
            // ANCHOR: list-leaves
            var response = await sdk.ListLeaves(
                request: new ListLeavesRequest(minValueSats: 10_000)
            );

            foreach (var leaf in response.leaves)
            {
                Console.WriteLine($"Leaf {leaf.id}: {leaf.value} sats");
            }
            // ANCHOR_END: list-leaves
        }

        async Task PrepareExit(BreezSdk sdk)
        {
            // ANCHOR: prepare-unilateral-exit
            var leafIds = new List<string> { "leaf-id-1", "leaf-id-2" };

            var response = await sdk.PrepareUnilateralExit(
                request: new PrepareUnilateralExitRequest(
                    feeRate: 2,
                    leafIds: leafIds,
                    utxos: new List<UnilateralExitCpfpUtxo>
                    {
                        new UnilateralExitCpfpUtxo(
                            txid: "your-utxo-txid",
                            vout: 0,
                            value: 50_000,
                            pubkey: "your-compressed-pubkey-hex",
                            utxoType: UnilateralExitCpfpUtxoType.P2wpkh
                        )
                    },
                    destination: "bc1q...your-destination-address"
                )
            );

            // The response contains:
            // - response.leaves: transaction/PSBT pairs to sign and broadcast
            // - response.sweepTxHex: signed sweep transaction for the final step
            foreach (var leaf in response.leaves)
            {
                foreach (var pair in leaf.txCpfpPsbts)
                {
                    if (pair.csvTimelockBlocks != null)
                    {
                        Console.WriteLine($"Timelock: wait {pair.csvTimelockBlocks} blocks");
                    }
                    // pair.parentTxHex: pre-signed Spark transaction
                    // pair.childPsbtHex: unsigned CPFP PSBT — sign with your UTXO key
                }
            }
            // ANCHOR_END: prepare-unilateral-exit
        }
    }
}
