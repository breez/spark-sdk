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
        config.maxDepositClaimFee = Fee.Rate(10u)

        // Set a maximum fee of 1000 sat
        config.maxDepositClaimFee = Fee.Fixed(1000u)
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
}
