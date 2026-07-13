using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class UnilateralExit
    {
        async Task<PrepareUnilateralExitResponse> QuoteExit(BreezSdk sdk)
        {
            // ANCHOR: prepare-unilateral-exit
            var quote = await sdk.PrepareUnilateralExit(
                request: new PrepareUnilateralExitRequest(
                    feeRateSatPerVbyte: 2,
                    fundingKind: new CpfpFundingKind.P2wpkh(),
                    destination: "bc1q...your-destination-address",
                    selection: new ExitLeafSelection.Auto()
                )
            );

            Console.WriteLine($"Recovering {quote.recoverableValueSat} sats for {quote.totalFeeSat} sats in fees");
            Console.WriteLine($"Fund a single UTXO of at least {quote.singleUtxoFundingSat} sats");
            // ANCHOR_END: prepare-unilateral-exit
            return quote;
        }

        async Task BuildExit(BreezSdk sdk, PrepareUnilateralExitResponse quote)
        {
            // ANCHOR: unilateral-exit
            var secretKeyBytes = Convert.FromHexString("your-secret-key-hex");
            var signer = BreezSdkSparkMethods.SingleKeyCpfpSigner(secretKeyBytes);

            var response = await sdk.UnilateralExit(
                request: new UnilateralExitRequest(
                    prepared: quote,
                    fundingInputs: new CpfpInput[]
                    {
                        new CpfpInput.P2wpkh(
                            txid: "your-utxo-txid",
                            vout: 0,
                            value: 50_000,
                            pubkey: "your-compressed-pubkey-hex"
                        )
                    }
                ),
                signer: signer
            );

            foreach (var tx in response.transactions)
            {
                if (tx.csvTimelockBlocks != null)
                {
                    Console.WriteLine($"{tx.txid}: wait {tx.csvTimelockBlocks} blocks after its parents confirm");
                }
            }
            // ANCHOR_END: unilateral-exit
        }

        // ANCHOR: custom-cpfp-signer
        class MyCpfpSigner : CpfpSigner
        {
            public async Task<byte[]> SignPsbt(byte[] psbtBytes)
            {
                return await SignPsbtWithYourKeys(psbtBytes);
            }

            async Task<byte[]> SignPsbtWithYourKeys(byte[] psbtBytes)
            {
                return await Task.FromResult(psbtBytes);
            }
        }
        // ANCHOR_END: custom-cpfp-signer
    }
}
