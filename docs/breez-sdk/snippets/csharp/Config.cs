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
                // Enable stable balance with USDB conversion
                stableBalanceConfig = new StableBalanceConfig(
                    tokens: new StableBalanceToken[] {
                        new StableBalanceToken(
                            label: "USDB",
                            tokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87"
                        )
                    },
                    defaultActiveLabel: "USDB"
                )
            };
            // ANCHOR_END: stable-balance-config
        }

        void ConfigureSparkConfig()
        {
            // ANCHOR: spark-config
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                // Connect to a custom Spark environment
                sparkConfig = new SparkConfig(
                    coordinatorIdentifier: "0000000000000000000000000000000000000000000000000000000000000001",
                    threshold: 2,
                    signingOperators: new[]
                    {
                        new SparkSigningOperator(
                            id: 0,
                            identifier: "0000000000000000000000000000000000000000000000000000000000000001",
                            address: "https://0.spark.example.com",
                            identityPublicKey: "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651"
                        ),
                        new SparkSigningOperator(
                            id: 1,
                            identifier: "0000000000000000000000000000000000000000000000000000000000000002",
                            address: "https://1.spark.example.com",
                            identityPublicKey: "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23"
                        ),
                        new SparkSigningOperator(
                            id: 2,
                            identifier: "0000000000000000000000000000000000000000000000000000000000000003",
                            address: "https://2.spark.example.com",
                            identityPublicKey: "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853"
                        )
                    },
                    sspConfig: new SparkSspConfig(
                        baseUrl: "https://api.example.com",
                        identityPublicKey: "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5",
                        schemaEndpoint: "graphql/spark/rc"
                    ),
                    expectedWithdrawBondSats: 10000,
                    expectedWithdrawRelativeBlockLocktime: 1000
                )
            };
            // ANCHOR_END: spark-config
        }
    }
}
