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
    // await builder.withChainService(<your chain service implementation>)
    // await builder.withRestClient(<your rest client implementation>)
    // await builder.withKeySet(<your key set type>, <use address index>, <account number>)
    // await builder.withPaymentObserver(<your payment observer implementation>)
    let sdk = try await builder.build()
    // ANCHOR_END: init-sdk-advanced

    return sdk
}

func gettingStartedNodeInfo(sdk: BreezSdk) async throws {
    // ANCHOR: fetch-balance
    // ensureSynced: true will ensure the SDK is synced with the Spark network
    // before returning the balance
    let info = try await sdk.getInfo(request: GetInfoRequest(
      ensureSynced: false
    ))
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
func disconnect(sdk: BreezSdk) async throws {
    try await sdk.disconnect()
}
// ANCHOR_END: disconnect
