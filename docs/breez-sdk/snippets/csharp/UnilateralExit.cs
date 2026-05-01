using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class UnilateralExit
    {
        async Task PrepareExit(BreezSdk sdk)
        {
            // ANCHOR: prepare-unilateral-exit
            // Create a signer from your UTXO private key (32-byte secret key)
            var secretKeyBytes = Convert.FromHexString("your-secret-key-hex");
            var signer = BreezSdkSparkMethods.SingleKeyCpfpSigner(secretKeyBytes);

            var response = await sdk.PrepareUnilateralExit(
                request: new PrepareUnilateralExitRequest(
                    feeRateSatPerVbyte: 2,
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
            foreach (var leaf in response.leaves)
            {
                Console.WriteLine($"Leaf {leaf.leafId}: {leaf.value} sats (exit cost: ~{leaf.estimatedCost} sats)");
                foreach (var tx in leaf.transactions)
                {
                    if (tx.csvTimelockBlocks != null)
                    {
                        Console.WriteLine($"Timelock: wait {tx.csvTimelockBlocks} blocks");
                    }
                    // tx.txHex: pre-signed Spark transaction
                    // tx.cpfpTxHex: signed CPFP transaction — broadcast alongside parent
                }
            }

            if (response.unverifiedNodeIds.Length > 0)
            {
                Console.WriteLine($"Warning: could not verify confirmation status for {response.unverifiedNodeIds.Length} nodes");
            }
            // ANCHOR_END: prepare-unilateral-exit
        }
    }
}
