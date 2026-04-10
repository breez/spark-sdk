import BreezSdkSpark
import Foundation

// ANCHOR: implement-prf-provider
// Implement the interface for custom logic if the built-in PlatformPasskeyPrfProvider doesn't fit your needs.
class CustomPasskeyPrfProvider: PasskeyPrfProvider {
    func derivePrfSeed(salt: String) async throws -> Data {
        // Call platform passkey API with PRF extension
        // Returns 32-byte PRF output
        fatalError("Implement using WebAuthn or native passkey APIs")
    }

    func isPrfAvailable() async throws -> Bool {
        // Check if PRF-capable passkey exists
        fatalError("Check platform passkey availability")
    }
}
// ANCHOR_END: implement-prf-provider

func connectWithPasskey() async throws -> BreezSdk {
    // ANCHOR: connect-with-passkey
    // Use the built-in platform PRF provider (or pass a custom implementation)
    let prfProvider = PlatformPasskeyPrfProvider()
    let passkey = Passkey(prfProvider: prfProvider, relayConfig: nil)

    // Derive the wallet from the passkey (pass nil for the default wallet)
    let wallet = try await passkey.getWallet(label: "personal")

    let config = defaultConfig(network: .mainnet)
    let sdk = try await connect(
        request: ConnectRequest(
            config: config,
            seed: wallet.seed,
            storageDir: "./.data"
        ))
    // ANCHOR_END: connect-with-passkey
    return sdk
}

func listLabels() async throws -> [String] {
    // ANCHOR: list-labels
    let prfProvider = PlatformPasskeyPrfProvider()
    let relayConfig = NostrRelayConfig(breezApiKey: "<breez api key>")
    let passkey = Passkey(prfProvider: prfProvider, relayConfig: relayConfig)

    // Query Nostr for labels associated with this passkey
    let labels = try await passkey.listLabels()

    for label in labels {
        print("Found label: \(label)")
    }
    // ANCHOR_END: list-labels
    return labels
}

func storeLabel() async throws {
    // ANCHOR: store-label
    let prfProvider = PlatformPasskeyPrfProvider()
    let relayConfig = NostrRelayConfig(breezApiKey: "<breez api key>")
    let passkey = Passkey(prfProvider: prfProvider, relayConfig: relayConfig)

    // Publish the label to Nostr for later discovery
    try await passkey.storeLabel(label: "personal")
    // ANCHOR_END: store-label
}
