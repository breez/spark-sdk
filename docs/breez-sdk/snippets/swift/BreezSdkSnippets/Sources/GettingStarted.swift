import BreezSdkSpark

func initSdk() async throws -> BreezSdk {
    // ANCHOR: init-sdk
    // Construct the seed using mnemonic words or entropy bytes
    let mnemonic = "<mnemonic words>"
    let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Connect to the SDK using the simplified connect method
    let sdk = try await connect(request: ConnectRequest(
        config: config,
        seed: seed,
        storageDir: "./.data"
    ))
    // ANCHOR_END: init-sdk

    return sdk
}

func initSdkAdvanced() async throws -> BreezSdk {
    // ANCHOR: init-sdk-advanced
    // Construct the seed using mnemonic words or entropy bytes
    let mnemonic = "<mnemonic words>"
    let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Create the default storage
    let storage = try defaultStorage(dataDir: "./.data")

    // Build the SDK using the config, seed and storage
    let builder = SdkBuilder(config: config, seed: seed, storage: storage)

    // You can also pass your custom implementations:
    // builder = builder.withChainService(<your chain service implementation>)
    // builder = builder.withRestClient(<your rest client implementation>)
    let sdk = try await builder.build()
    // ANCHOR_END: init-sdk-advanced

    return sdk
}

func gettingStartedNodeInfo(sdk: BreezSdk) async throws {
    // ANCHOR: fetch-balance
    let info = try await sdk.getInfo(request: GetInfoRequest())
    let balanceSats = info.balanceSats
    // ANCHOR_END: fetch-balance
    print(balanceSats)
}

// ANCHOR: logging
class SdkLogger: Logger {
    func log(l: LogEntry) {
        print("Received log [", l.level, "]: ", l.line)
    }
}

func logging() throws {
    try initLogging(logDir: nil, appLogger: SdkLogger(), logFilter: nil)
}
// ANCHOR_END: logging

// ANCHOR: add-event-listener
class SdkEventListener: EventListener {
    func onEvent(event: SdkEvent) async {
        print("Received event: ", event)
    }
}

func addEventListener(sdk: BreezSdk, listener: SdkEventListener) async -> String {
    let listenerId = await sdk.addEventListener(listener: listener)
    return listenerId
}
// ANCHOR_END: add-event-listener

// ANCHOR: remove-event-listener
func removeEventListener(sdk: BreezSdk, listenerId: String) async {
    await sdk.removeEventListener(id: listenerId)
}
// ANCHOR_END: remove-event-listener

// ANCHOR: disconnect
func disconnect(sdk: BreezSdk) throws {
    try sdk.disconnect()
}
// ANCHOR_END: disconnect
