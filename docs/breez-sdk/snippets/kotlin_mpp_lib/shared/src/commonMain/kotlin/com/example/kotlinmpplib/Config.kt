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
            tokenIdentifier = "<token_identifier>",
            thresholdSats = 10_000u,
            maxSlippageBps = 100u,
            reservedSats = 1_000u
        )
        // ANCHOR_END: stable-balance-config
        println("Config: $config")
    }
}
