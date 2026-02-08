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
            config = config with { maxDepositClaimFee = new MaxFee.Rate(satPerVbyte: 10) };

            // Set a maximum fee of 1000 sat
            config = config with { maxDepositClaimFee = new MaxFee.Fixed(amount: 1000) };

            // Set the maximum fee to the fastest network recommended fee at the time of claim
            // with a leeway of 1 sats/vbyte
            config = config with { maxDepositClaimFee = new MaxFee.NetworkRecommended(leewaySatPerVbyte: 1) };
            // ANCHOR_END: max-deposit-claim-fee
        }

        void ConfigurePrivateEnabledDefault()
        {
            // ANCHOR: private-enabled-default
            // Disable Spark private mode by default
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                privateEnabledDefault = false
            };
            // ANCHOR_END: private-enabled-default
        }

        void ConfigureOptimizationConfiguration()
        {
            // ANCHOR: optimization-configuration
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                optimizationConfig = new OptimizationConfig(autoEnabled: true, multiplicity: 1)
            };
            // ANCHOR_END: optimization-configuration
        }

        void ConfigureStableBalance()
        {
            // ANCHOR: stable-balance-config
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                // Enable stable balance with auto-conversion to a specific token
                stableBalanceConfig = new StableBalanceConfig(
                    tokenIdentifier: "<token_identifier>",
                    thresholdSats: 10000,
                    maxSlippageBps: 100,
                    reservedSats: 1000
                )
            };
            // ANCHOR_END: stable-balance-config
        }
    }
}
