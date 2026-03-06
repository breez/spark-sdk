import BreezSdkSpark
import Foundation

// ANCHOR: implement-prf-provider
// In practice, implement using platform-specific passkey APIs.
class ExamplePasskeyPrfProvider: PasskeyPrfProvider {
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
    let prfProvider = ExamplePasskeyPrfProvider()
    let passkey = Passkey(prfProvider: prfProvider, relayConfig: nil)

    // Derive the wallet from the passkey (pass nil for the default wallet)
    let wallet = try await passkey.getWallet(walletName: "personal")

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

func listWalletNames() async throws -> [String] {
    // ANCHOR: list-wallet-names
    let prfProvider = ExamplePasskeyPrfProvider()
    let relayConfig = NostrRelayConfig(breezApiKey: "<breez api key>")
    let passkey = Passkey(prfProvider: prfProvider, relayConfig: relayConfig)

    // Query Nostr for wallet names associated with this passkey
    let walletNames = try await passkey.listWalletNames()

    for walletName in walletNames {
        print("Found wallet: \(walletName)")
    }
    // ANCHOR_END: list-wallet-names
    return walletNames
}

func storeWalletName() async throws {
    // ANCHOR: store-wallet-name
    let prfProvider = ExamplePasskeyPrfProvider()
    let relayConfig = NostrRelayConfig(breezApiKey: "<breez api key>")
    let passkey = Passkey(prfProvider: prfProvider, relayConfig: relayConfig)

    // Publish the wallet name to Nostr for later discovery
    try await passkey.storeWalletName(walletName: "personal")
    // ANCHOR_END: store-wallet-name
}
