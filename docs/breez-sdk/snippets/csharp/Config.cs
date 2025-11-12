using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class ConfigSnippets
    {
        void ConfigureSdk()
        {
            // ANCHOR: max-deposit-claim-fee
            // Create the default config with API key
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "<breez api key>"
            };

            // Disable automatic claiming
            config = config with { maxDepositClaimFee = null };

            // Set a maximum feerate of 10 sat/vB
            config = config with { maxDepositClaimFee = new Fee.Rate(satPerVbyte: 10) };

            // Set a maximum fee of 1000 sat
            config = config with { maxDepositClaimFee = new Fee.Fixed(amount: 1000) };
            // ANCHOR_END: max-deposit-claim-fee
        }
    }
}
