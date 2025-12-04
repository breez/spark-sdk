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
        switch event {
        case .synced:
            // Data has been synchronized with the network. When this event is received,
            // it is recommended to refresh the payment list and wallet balance.
            break
        case .unclaimedDeposits(let unclaimedDeposits):
            // SDK was unable to claim some deposits automatically
            let _ = unclaimedDeposits
        case .claimedDeposits(let claimedDeposits):
            // Deposits were successfully claimed
            let _ = claimedDeposits
        case .paymentSucceeded(let paymentSucceeded):
            // A payment completed successfully
            let _ = paymentSucceeded
        case .paymentPending(let paymentPending):
            // A payment is pending (waiting for confirmation)
            let _ = paymentPending
        case .paymentFailed(let paymentFailed):
            // A payment failed
            let _ = paymentFailed
        default:
            // Handle any future event types
            break
        }
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
