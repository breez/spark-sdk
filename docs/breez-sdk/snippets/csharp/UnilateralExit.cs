using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class UnilateralExit
    {
        async Task PrepareExit(BreezSdk sdk)
        {
            // ANCHOR: prepare-unilateral-exit
            var signer = new SingleKeySigner(secretKeyBytes: Convert.FromHexString("your-secret-key-hex"));

            var response = await sdk.PrepareUnilateralExit(
                request: new PrepareUnilateralExitRequest(
                    feeRate: 2,
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

            // The SDK automatically selects which leaves are profitable to exit.
            foreach (var leaf in response.SelectedLeaves)
            {
                Console.WriteLine($"Leaf {leaf.Id}: {leaf.Value} sats (exit cost: ~{leaf.EstimatedCost} sats)");
            }

            foreach (var leaf in response.Transactions)
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
