import BreezSdkSpark

func configureSdk() async throws {
    // ANCHOR: max-deposit-claim-fee
    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Disable automatic claiming
    config.maxDepositClaimFee = nil

    // Set a maximum feerate of 10 sat/vB
    config.maxDepositClaimFee = Fee.rate(satPerVbyte: 10)

    // Set a maximum fee of 1000 sat
    config.maxDepositClaimFee = Fee.fixed(amount: 1000)
    // ANCHOR_END: max-deposit-claim-fee
    print("Config: \(config)")
}
