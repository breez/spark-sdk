import BreezSdkSpark
import Foundation

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs (hardware key, FIDO2 transport,
// air-gapped backup file, etc.). Single API surface: deriveSeeds for
// derivation, createPasskey for registration, isSupported /
// checkDomainAssociation for diagnostics. Single-salt derivation is the
// trivial 1-element bulk case.
class CustomPrfProvider: PrfProvider {
    func deriveSeeds(salts: [String]) async throws -> [Data] {
        // Call platform passkey API with PRF extension. Use the dual-salt
        // ceremony when the authenticator supports it (one OS prompt for
        // N salts) and fall back to per-salt assertions otherwise.
        // Returns one 32-byte PRF output per salt in input order.
        fatalError("Implement using WebAuthn or native passkey APIs")
    }

    func isSupported() async throws -> Bool {
        // Check if a PRF-capable authenticator is reachable from this
        // platform / device.
        fatalError("Check platform passkey availability")
    }

    func createPasskey(request: CreatePasskeyRequest) async throws -> RegisteredCredential {
        // Register a new credential and return its ID + AAGUID + BE flag.
        fatalError("Implement registration via WebAuthn create() / native API")
    }

    func checkDomainAssociation() async throws -> DomainAssociation {
        // Optional: verify the app's identity against the platform's
        // domain verification source (e.g., iOS AASA CDN, Android
        // assetlinks). Built-in providers do this automatically; custom
        // providers without a verification source return `.skipped`,
        // which tells callers "proceed with WebAuthn as normal".
        return .skipped(reason: "CustomPrfProvider does not verify domain association")
    }
}
// ANCHOR_END: implement-prf-provider

func checkAvailability() async throws {
    // ANCHOR: check-availability
    let prfProvider = PasskeyProvider()
    if try await prfProvider.isSupported() {
        // Show passkey as primary option
    } else {
        // Fall back to mnemonic flow
    }
    // ANCHOR_END: check-availability
}

func connectWithPasskey() async throws -> BreezSdk {
    // ANCHOR: connect-with-passkey
    // Use the built-in platform PRF provider (or pass a custom implementation).
    let prfProvider = PasskeyProvider()
    let passkey = PasskeyClient(prfProvider: prfProvider, relayConfig: nil)

    // signIn derives the wallet seed for an existing credential. With
    // bulk PRF on iOS+Android this is a single OS prompt that derives
    // master + label seeds in one ceremony.
    let response = try await passkey.signIn(
        request: SignInRequest(label: "personal", extraSalts: [])
    )

    let config = defaultConfig(network: .mainnet)
    let sdk = try await connect(
        request: ConnectRequest(
            config: config,
            seed: response.wallet.seed,
            storageDir: "./.data"
        ))
    // ANCHOR_END: connect-with-passkey
    return sdk
}

func registerNewPasskey() async throws -> BreezSdk {
    // ANCHOR: register-passkey
    // For a brand-new user with no existing passkey: register() creates
    // the credential AND derives the wallet seed in one orchestrated
    // call. On iOS+Android this is 2 OS prompts total (1 create + 1
    // dual-salt assert) thanks to the SDK's bulk-PRF setup_wallet path.
    let prfProvider = PasskeyProvider()
    let passkey = PasskeyClient(prfProvider: prfProvider, relayConfig: nil)

    let response = try await passkey.register(
        request: RegisterRequest(
            label: "personal",
            extraSalts: [],
            excludeCredentialIds: []
        )
    )

    let config = defaultConfig(network: .mainnet)
    let sdk = try await connect(
        request: ConnectRequest(
            config: config,
            seed: response.wallet.seed,
            storageDir: "./.data"
        ))
    // ANCHOR_END: register-passkey
    return sdk
}

func listLabels() async throws -> [String] {
    // ANCHOR: list-labels
    let prfProvider = PasskeyProvider()
    let relayConfig = NostrRelayConfig(breezApiKey: "<breez api key>")
    let passkey = PasskeyClient(prfProvider: prfProvider, relayConfig: relayConfig)

    // signIn with no label runs in discovery mode: it derives the
    // master seed AND lists labels in the same ceremony, so a follow-up
    // listLabels() reads from the cached identity for free.
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
    let passkey = PasskeyClient(prfProvider: prfProvider, relayConfig: relayConfig)

    // For a new label on an existing identity, call signIn(newLabel)
    // first to seed the SDK's identity cache via setup_wallet, THEN
    // storeLabel uses the cached identity for free (1 OS prompt total).
    try await passkey.storeLabel(label: "personal")
    // ANCHOR_END: store-label
}
