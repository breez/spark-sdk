package com.example.kotlinmpplib

import breez_sdk_spark.*

class Config {
    fun configureSdk() {
        // ANCHOR: max-deposit-claim-fee
        // Create the default config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        // Disable automatic claiming
        config.maxDepositClaimFee = null

        // Set a maximum feerate of 10 sat/vB
        config.maxDepositClaimFee = MaxFee.Rate(10u)

        // Set a maximum fee of 1000 sat
        config.maxDepositClaimFee = MaxFee.Fixed(1000u)

        // Set the maximum fee to the fastest network recommended fee at the time of claim
        // with a leeway of 1 sats/vbyte
        config.maxDepositClaimFee = MaxFee.NetworkRecommended(1u)
        // ANCHOR_END: max-deposit-claim-fee
        println("Config: $config")
    }

    fun configurePrivateEnabledDefault() {
        // ANCHOR: private-enabled-default
        // Disable Spark private mode by default
        val config = defaultConfig(Network.MAINNET)
        config.privateEnabledDefault = false
        // ANCHOR_END: private-enabled-default
        println("Config: $config")
    }

    fun configureOptimizationConfiguration() {
        // ANCHOR: optimization-configuration
        val config = defaultConfig(Network.MAINNET)
        config.optimizationConfig = OptimizationConfig(autoEnabled = true, multiplicity = 1u)
        // ANCHOR_END: optimization-configuration
        println("Config: $config")
    }

    fun configureStableBalance() {
        // ANCHOR: stable-balance-config
        val config = defaultConfig(Network.MAINNET)

        // Enable stable balance with auto-conversion to a specific token
        config.stableBalanceConfig = StableBalanceConfig(
            tokens = listOf(StableBalanceToken(
                label = "USDB",
                tokenIdentifier = "<token_identifier>",
            )),
            defaultActiveLabel = "USDB",
            thresholdSats = 10_000u,
            maxSlippageBps = 100u,
        )
        // ANCHOR_END: stable-balance-config
        println("Config: $config")
    }

    fun configureSparkConfig() {
        // ANCHOR: spark-config
        val config = defaultConfig(Network.MAINNET)

        // Connect to a custom Spark environment
        config.sparkConfig = SparkConfig(
            coordinatorIdentifier = "0000000000000000000000000000000000000000000000000000000000000001",
            threshold = 2u,
            signingOperators = listOf(
                SparkSigningOperator(
                    id = 0u,
                    identifier = "0000000000000000000000000000000000000000000000000000000000000001",
                    address = "https://0.spark.example.com",
                    identityPublicKey = "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651"
                ),
                SparkSigningOperator(
                    id = 1u,
                    identifier = "0000000000000000000000000000000000000000000000000000000000000002",
                    address = "https://1.spark.example.com",
                    identityPublicKey = "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23"
                ),
                SparkSigningOperator(
                    id = 2u,
                    identifier = "0000000000000000000000000000000000000000000000000000000000000003",
                    address = "https://2.spark.example.com",
                    identityPublicKey = "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853"
                )
            ),
            sspConfig = SparkSspConfig(
                baseUrl = "https://api.example.com",
                identityPublicKey = "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5",
                schemaEndpoint = "graphql/spark/rc"
            ),
            expectedWithdrawBondSats = 10_000u,
            expectedWithdrawRelativeBlockLocktime = 1_000u
        )
        // ANCHOR_END: spark-config
        println("Config: $config")
    }
}
