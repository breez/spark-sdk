import BreezSdkSpark

func initSdk() async throws -> BreezSdk {
    // ANCHOR: init-sdk
    let mnemonic = "<mnemonic words>"
    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Create the default storage
    let storage = try defaultStorage(dataDir: "./.data")

    // Build the SDK using the config, mnemonic and storage
    let builder = SdkBuilder(config: config, mnemonic: mnemonic, storage: storage)
    let sdk = try await builder.build()
    // ANCHOR_END: init-sdk

    return sdk
}

func gettingStartedNodeInfo(sdk: BreezSdk) async throws {
    // ANCHOR: fetch-balance
    let info = try await sdk.getInfo(GetInfoRequest())
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
    try initLogging(logger: SdkLogger())
}
// ANCHOR_END: logging

// ANCHOR: add-event-listener
class SdkEventListener: EventListener {
    func onEvent(e: SdkEvent) {
        print("Received event: ", e)
    }
}

func addEventListener(sdk: BreezSdk, listener: SdkEventListener) throws -> String {
    let listenerId = try sdk.addEventListener(listener: listener)
    return listenerId
}
// ANCHOR_END: add-event-listener

// ANCHOR: remove-event-listener
func removeEventListener(sdk: BreezSdk, listenerId: String) throws {
    try sdk.removeEventListener(id: listenerId)
}
// ANCHOR_END: remove-event-listener

// ANCHOR: disconnect
func disconnect(sdk: BreezSdk) throws {
    try sdk.disconnect()
}
// ANCHOR_END: disconnect
