import BreezSdkSpark

func configureSdk() async throws {
    // ANCHOR: max-deposit-claim-fee
    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Disable automatic claiming
    config.maxDepositClaimFee = nil

    // Set a maximum feerate of 10 sat/vB
    config.maxDepositClaimFee = MaxFee.rate(satPerVbyte: 10)

    // Set a maximum fee of 1000 sat
    config.maxDepositClaimFee = MaxFee.fixed(amount: 1000)

    // Set the maximum fee to the fastest network recommended fee at the time of claim
    // with a leeway of 1 sats/vbyte
    config.maxDepositClaimFee = MaxFee.networkRecommended(leewaySatPerVbyte: 1)
    // ANCHOR_END: max-deposit-claim-fee
    print("Config: \(config)")
}

func configurePrivateEnabledDefault() async throws {
    // ANCHOR: private-enabled-default
    // Disable Spark private mode by default
    var config = defaultConfig(network: Network.mainnet)
    config.privateEnabledDefault = false
    // ANCHOR_END: private-enabled-default
    print("Config: \(config)")
}
