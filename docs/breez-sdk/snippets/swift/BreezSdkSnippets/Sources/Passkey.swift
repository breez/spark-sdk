import BreezSdkSpark
import Foundation

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs (hardware key, FIDO2 transport,
// air-gapped backup file, etc.). Three required methods: deriveSeeds for
// derivation, isSupported for the capability probe; createPasskey for
// registration is optional.
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

    func createPasskey(excludeCredentialIds: [Data]) async throws -> RegisteredCredential {
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
    // `rpId` is required. Pass your app's domain, or
    // `PasskeyProvider.BREEZ_RP_ID` if your app is Breez-registered.
    let prfProvider = PasskeyProvider(rpId: "my-app.com")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: nil, config: nil)

    // checkAvailability collapses isSupported + checkDomainAssociation
    // into a single tagged value. Branch on the variant the host needs.
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

func connectWithPasskey() async throws -> BreezSdk {
    // ANCHOR: connect-with-passkey
    // Single-CTA onboarding: silent sign-in for a returning user,
    // fall-through to register on a fresh device. Internally pins
    // `preferImmediatelyAvailableCredentials = true` so the silent
    // attempt fast-fails (no UI) when no local credential exists;
    // only `CredentialNotFound` flips to register, all other errors
    // (cancel / timeout / configuration) propagate unchanged.
    let prfProvider = PasskeyProvider(rpId: "my-app.com")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: nil, config: nil)

    let response = try await passkey.connectWithPasskey(
        request: ConnectWithPasskeyRequest(label: "personal")
    )

    // Branch on `flow` to know which path ran. Hosts maintaining a
    // CredentialRegistry typically persist the new credential ID on
    // `.registered`; `.signedIn` surfaces the asserted ID when the
    // provider supports it.
    switch response.flow {
    case .signedIn: break  // returning user
    case .registered: break  // new user
    }

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
    // dual-salt assert) thanks to the SDK's bulk-PRF path.
    let prfProvider = PasskeyProvider(rpId: "my-app.com")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: nil, config: nil)

    let response = try await passkey.register(
        request: RegisterRequest(label: "personal")
    )

    // Hosts SHOULD persist credential.credentialId (for excludeCredentialIds
    // bookkeeping) and credential.userId (for server-side correlation).
    // The SDK generates userId; it is never host-supplied.
    let _ = (response.credential.credentialId, response.credential.userId)

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
    let prfProvider = PasskeyProvider(rpId: "my-app.com")
    let config = PasskeyConfig(
        // Optional: override the default wallet label used when
        // register / signIn receive `label = nil`. Falls back to the
        // SDK's internal "Default" when unset.
        defaultLabel: "personal"
    )
    // breezApiKey enables authenticated (NIP-42) Breez relay access
    // for label sync; pass nil for public-relay-only.
    let passkey = PasskeyClient(
        prfProvider: prfProvider,
        breezApiKey: "<breez api key>",
        config: config
    )

    let labels = try await passkey.labels().list()
    for label in labels {
        print("Found label: \(label)")
    }
    // ANCHOR_END: list-labels
    return labels
}

func storeLabel() async throws {
    // ANCHOR: store-label
    let prfProvider = PasskeyProvider(rpId: "my-app.com")
    let passkey = PasskeyClient(
        prfProvider: prfProvider,
        breezApiKey: "<breez api key>",
        config: nil
    )

    try await passkey.labels().store(label: "personal")
    // ANCHOR_END: store-label
}

func checkDomain() async throws {
    // ANCHOR: domain-association
    let prfProvider = PasskeyProvider(rpId: "my-app.com")
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
    let prfProvider = PasskeyProvider(rpId: "my-app.com")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: nil, config: nil)

    do {
        let response = try await passkey.register(
            request: RegisterRequest(
                label: "personal",
                excludeCredentialIds: [
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
    let prfProvider = PasskeyProvider(rpId: "my-app.com")
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: nil, config: nil)

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
