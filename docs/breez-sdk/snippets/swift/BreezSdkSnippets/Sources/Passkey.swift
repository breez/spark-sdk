import BreezSdkSpark
import Foundation

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs (hardware key, FIDO2 transport,
// air-gapped backup file, etc.). Three required methods: deriveSeeds for
// derivation, isSupported for the capability probe; createPasskey for
// registration is optional.
class CustomPrfProvider: PrfProvider {
    func deriveSeeds(request: DeriveSeedsRequest) async throws -> DeriveSeedsOutput {
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

    func createPasskey(excludeCredentials: [Data]) async throws -> PasskeyCredential {
        // Register a new credential and return its ID, the WebAuthn
        // user.id the platform recorded (returned for host-side
        // correlation, never host-supplied), AAGUID, and BE flag.
        fatalError("Implement registration via WebAuthn create() / native API")
    }

    func checkDomainAssociation() async throws -> DomainAssociation {
        // Optional: verify the app's identity against the platform's
        // domain verification source (e.g., iOS AASA CDN, Android
        // assetlinks). Custom providers without a verification source
        // return `.skipped`, which tells callers "proceed with WebAuthn
        // as normal".
        return .skipped(reason: "CustomPrfProvider does not verify domain association")
    }
}
// ANCHOR_END: implement-prf-provider

func checkAvailability() async throws {
    // ANCHOR: check-availability
    // Pass `PasskeyProvider.BREEZ_RP_ID` instead of "<your-rp-domain>" if your
    // app is Breez-registered (shares credentials with other Breez apps).
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    switch try await passkey.checkAvailability() {
    case .available:
        break // Show passkey as primary option.
    case .prfUnsupported:
        break // Fall back to mnemonic flow.
    case .notAssociated(let source, let reason):
        print("Domain association failed (source=\(source)): \(reason)")
    case .skipped:
        break // No verification source on this platform; proceed normally.
    }
    // ANCHOR_END: check-availability
}

func setupPasskeyClient() -> PasskeyClient {
    // ANCHOR: setup-client
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)
    // ANCHOR_END: setup-client
    return passkey
}

func connectWithPasskey() async throws -> BreezSdk {
    // ANCHOR: connect-with-passkey
    // Single-CTA onboarding: silent sign-in, fall through to register.
    var config = defaultConfig(network: .mainnet)
    config.apiKey = "<breez api key>"
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: config.apiKey, config: nil)

    let response = try await passkey.connectWithPasskey(
        request: ConnectWithPasskeyRequest(label: "personal")
    )

    // `credential` is the path discriminator (nil on sign-in).
    if let credential = response.credential {
        let _ = credential.credentialId
    }

    let sdk = try await connect(
        request: ConnectRequest(
            config: config,
            seed: response.wallet.seed,
            storageDir: "./.data"
        ))
    // ANCHOR_END: connect-with-passkey
    return sdk
}

func signInExistingUser() async throws -> SignInResponse {
    // ANCHOR: sign-in
    // Returning-user-only sign-in. No fall-through to register: use
    // `connectWithPasskey` when you also want the new-user path.
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    return try await passkey.signIn(request: SignInRequest(label: "personal"))
    // ANCHOR_END: sign-in
}

func registerNewPasskey() async throws -> BreezSdk {
    // ANCHOR: register-passkey
    var config = defaultConfig(network: .mainnet)
    config.apiKey = "<breez api key>"
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: config.apiKey, config: nil)

    let response = try await passkey.register(
        request: RegisterRequest(label: "personal")
    )

    // Persist credentialId for future excludeCredentials.
    let _ = (response.credential?.credentialId, response.credential?.userId)

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
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)
    // ANCHOR: list-labels
    let labels = try await passkey.labels().list()
    for label in labels {
        print("Found label: \(label)")
    }
    // ANCHOR_END: list-labels
    return labels
}

func storeLabel() async throws {
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)
    // ANCHOR: store-label
    try await passkey.labels().store(label: "personal")
    // ANCHOR_END: store-label
}

func checkDomain() async throws {
    // ANCHOR: domain-association
    // Lower-level provider call. Most hosts use `checkAvailability` instead.
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let result = try await prfProvider.checkDomainAssociation()

    switch result {
    case .associated:
        break
    case .notAssociated(let source, let reason):
        print("Domain association failed (source=\(source)): \(reason)")
        return
    case .skipped:
        break
    }
    // ANCHOR_END: domain-association
}

func recoverFromAlreadyExists() async throws -> Wallet {
    // ANCHOR: recover-already-exists
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    do {
        let response = try await passkey.register(
            request: RegisterRequest(
                label: "personal",
                excludeCredentials: [
                    // app-persisted credential IDs from prior registrations
                ]
            )
        )
        return response.wallet
    } catch PrfProviderError.CredentialAlreadyExists {
        let response = try await passkey.signIn(
            request: SignInRequest(label: "personal")
        )
        return response.wallet
    }
    // ANCHOR_END: recover-already-exists
}

func handleTimeout() async throws -> SignInResponse {
    // ANCHOR: handle-timeout
    let prfProvider = PasskeyProvider(rpId: "<your-rp-domain>", rpName: "Your App")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    do {
        return try await passkey.signIn(
            request: SignInRequest(label: "personal")
        )
    } catch PrfProviderError.UserTimedOut {
        print("Sign-in timed out: show \"Try Again\" UI.")
        throw PrfProviderError.UserTimedOut
    }
    // ANCHOR_END: handle-timeout
}
