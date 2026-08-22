import BreezSdkSpark

func initSdk() async throws -> BreezSdk {
    // ANCHOR: init-sdk
    // Construct the seed using a mnemonic, entropy or passkey
    let mnemonic = "<mnemonic words>"
    let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Connect to the SDK using the simplified connect method
    let sdk = try await connect(
        request: ConnectRequest(
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
    let info = try await sdk.getInfo(
        request: GetInfoRequest(
            ensureSynced: false
        ))
    let identityPubkey = info.identityPubkey
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
        case .newDeposits(let newDeposits):
            // Detected deposits, as DepositInfo. Only those with isMature set
            // have enough confirmations to be claimed. Show the rest as pending.
            let _ = newDeposits
        case .unclaimedDeposits(let unclaimedDeposits):
            // Deposits the SDK could not claim. Each claimError says why,
            // most often the fee exceeded the configured maximum.
            let _ = unclaimedDeposits
        case .claimedDeposits(let claimedDeposits):
            // Deposits claimed into the wallet. The resulting payment
            // arrives separately as its own event.
            let _ = claimedDeposits
        case .paymentSucceeded(let paymentSucceeded):
            // A payment completed. The cached balance is already refreshed,
            // so getInfo returns the new value.
            let _ = paymentSucceeded
        case .paymentPending(let paymentPending):
            // A payment is awaiting confirmation. It arrives again as
            // succeeded or failed once it settles.
            let _ = paymentPending
        case .paymentFailed(let paymentFailed):
            // A payment failed. payment.details carries the method-specific
            // context to show the user.
            let _ = paymentFailed
        case .autoOptimization(let optimizationEvent):
            // Background optimizer progress: started, round completed, or a
            // terminal outcome. Manual optimizeLeaves calls do not emit these.
            let _ = optimizationEvent
        case .lightningAddressChanged(let lightningAddress):
            // The lightning address changed on another device. Unset when the
            // address was deleted.
            let _ = lightningAddress
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

// ANCHOR: spark-status
func gettingStartedSparkStatus() async throws {
    let sparkStatus = try await getSparkStatus()

    switch sparkStatus.status {
    case .operational:
        print("Spark is fully operational")
    case .degraded:
        print("Spark is experiencing degraded performance")
    case .partial:
        print("Spark is partially unavailable")
    case .major:
        print("Spark is experiencing a major outage")
    case .unknown:
        print("Spark status is unknown")
    }

    print("Last updated: \(sparkStatus.lastUpdated)")
}
// ANCHOR_END: spark-status

// ANCHOR: disconnect
func disconnect(sdk: BreezSdk) async throws {
    try await sdk.disconnect()
}
// ANCHOR_END: disconnect