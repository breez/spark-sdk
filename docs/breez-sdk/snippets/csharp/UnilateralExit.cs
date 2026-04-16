using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class UnilateralExit
    {
        // ANCHOR: prepare-unilateral-exit
        // Implement the CpfpSigner to sign CPFP transactions with your UTXO key
        class MyCpfpSigner : CpfpSigner
        {
            public async Task<byte[]> SignPsbt(byte[] psbtBytes)
            {
                // Sign the PSBT with your UTXO private key and return the signed bytes
                throw new NotImplementedException();
            }
        }

        async Task PrepareExit(BreezSdk sdk)
        {
            var signer = new MyCpfpSigner();

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
            foreach (var leaf in response.Leaves)
            {
                Console.WriteLine($"Leaf {leaf.LeafId}: {leaf.Value} sats (exit cost: ~{leaf.EstimatedCost} sats)");
                foreach (var tx in leaf.Transactions)
                {
                    if (tx.CsvTimelockBlocks != null)
                    {
                        Console.WriteLine($"Timelock: wait {tx.CsvTimelockBlocks} blocks");
                    }
                    // tx.TxHex: pre-signed Spark transaction
                    // tx.CpfpTxHex: signed CPFP transaction — broadcast alongside parent
                }
            }

            if (response.UnverifiedNodeIds.Length > 0)
            {
                Console.WriteLine($"Warning: could not verify confirmation status for {response.UnverifiedNodeIds.Length} nodes");
            }
            // ANCHOR_END: prepare-unilateral-exit
        }
    }
}
