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
            var leafIds = new string[] { "leaf-id-1", "leaf-id-2" };
            var signer = new SingleKeySigner(secretKeyBytes: Convert.FromHexString("your-secret-key-hex"));

            var response = await sdk.PrepareUnilateralExit(
                request: new PrepareUnilateralExitRequest(
                    feeRate: 2,
                    leafIds: leafIds,
                    inputs: new UnilateralExitCpfpInput[]
                    {
                        new UnilateralExitCpfpInput.P2wpkh(
                            txid: "your-utxo-txid",
                            vout: 0,
                            value: 50_000,
                            pubkey: "your-compressed-pubkey-hex"
                        )
                    },
                    destination: "bc1q...your-destination-address"
                ),
                signer: signer
            );

            foreach (var leaf in response.leaves)
            {
                foreach (var pair in leaf.TxCpfpPairs)
                {
                    if (pair.CsvTimelockBlocks != null)
                    {
                        Console.WriteLine($"Timelock: wait {pair.CsvTimelockBlocks} blocks");
                    }
                    // pair.ParentTxHex: pre-signed Spark transaction
                    // pair.ChildTxHex: signed CPFP transaction — broadcast alongside parent
                }
            }
            // ANCHOR_END: prepare-unilateral-exit
        }
    }
}
