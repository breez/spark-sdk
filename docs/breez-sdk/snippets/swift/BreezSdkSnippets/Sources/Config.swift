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

func configureOptimizationConfiguration() async throws {
    // ANCHOR: optimization-configuration
    var config = defaultConfig(network: Network.mainnet)
    config.leafOptimizationConfig = LeafOptimizationConfig(autoEnabled: true, multiplicity: 1)
    config.tokenOptimizationConfig = TokenOptimizationConfig(
        autoEnabled: true,
        targetOutputCount: 5,
        minOutputsThreshold: 50
    )
    // ANCHOR_END: optimization-configuration
    print("Config: \(config)")
}

func configureStableBalance() async throws {
    // ANCHOR: stable-balance-config
    var config = defaultConfig(network: Network.mainnet)

    // Enable stable balance with USDB conversion
    config.stableBalanceConfig = StableBalanceConfig(
        tokens: [StableBalanceToken(
            label: "USDB",
            tokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87"
        )],
        defaultActiveLabel: "USDB"
    )
    // ANCHOR_END: stable-balance-config
    print("Config: \(config)")
}

func configureBackgroundTasks() {
    // ANCHOR: config-background-tasks
    // Server-mode profile: equivalent to defaultServerConfig(network: .mainnet).
    // Recommended when you build the SDK per request in a multi-tenant server
    // deployment. See the "Server mode" page for the full profile.
    var config = defaultConfig(network: Network.mainnet)
    config.backgroundTasksEnabled = false
    // ANCHOR_END: config-background-tasks
    print("Config: \(config)")
}

func configureCrossChain() {
    // ANCHOR: cross-chain-config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Override the default slippage tolerance (basis points; 10 to 500).
    // Set crossChainConfig to nil to disable the feature.
    config.crossChainConfig = CrossChainConfig(
        defaultSlippageBps: 50,
        defaultTargetOverpayBps: nil
    )
    // ANCHOR_END: cross-chain-config
    print("Config: \(config)")
}
