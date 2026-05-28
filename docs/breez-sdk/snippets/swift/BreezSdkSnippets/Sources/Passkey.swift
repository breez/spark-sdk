import BreezSdkSpark
import Foundation

// ANCHOR: implement-prf-provider
// Implement the interface for custom logic if the built-in PasskeyProvider doesn't fit your needs.
class CustomPrfProvider: PrfProvider {
    func derivePrfSeed(salt: String) async throws -> Data {
        // Call platform passkey API with PRF extension
        // Returns 32-byte PRF output
        fatalError("Implement using WebAuthn or native passkey APIs")
    }

    func isPrfAvailable() async throws -> Bool {
        // Check if PRF-capable passkey exists
        fatalError("Check platform passkey availability")
    }

    func checkDomainAssociation() async throws -> DomainAssociation {
        // Optional: verify the app's identity against the platform's domain
        // verification source (e.g., iOS AASA CDN, Android assetlinks).
        // Built-in providers do this automatically; custom providers that
        // don't have a platform cache to verify against should return
        // `.skipped`, which tells callers "proceed with WebAuthn as normal".
        return .skipped(reason: "CustomPrfProvider does not verify domain association")
    }
}
// ANCHOR_END: implement-prf-provider

func checkAvailability() async throws {
    // ANCHOR: check-availability
    let prfProvider = PasskeyProvider()
    if try await prfProvider.isPrfAvailable() {
        // Show passkey as primary option
    } else {
        // Fall back to mnemonic flow
    }
    // ANCHOR_END: check-availability
}

func connectWithPasskey() async throws -> BreezSdk {
    // ANCHOR: connect-with-passkey
    // Use the built-in platform PRF provider (or pass a custom implementation)
    let prfProvider = PasskeyProvider()
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
    let prfProvider = PasskeyProvider()
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
    let prfProvider = PasskeyProvider()
    let relayConfig = NostrRelayConfig(breezApiKey: "<breez api key>")
    let passkey = Passkey(prfProvider: prfProvider, relayConfig: relayConfig)

    // Publish the label to Nostr for later discovery
    try await passkey.storeLabel(label: "personal")
    // ANCHOR_END: store-label
}
