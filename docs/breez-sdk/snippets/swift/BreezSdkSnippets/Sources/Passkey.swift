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

func singleCtaOnboarding() async throws -> Wallet {
    // ANCHOR: signin-fallback-register
    // Single-CTA onboarding: try silent signIn first, fall through to
    // register on CredentialNotFound. The OS shows ONE prompt for a
    // returning user (silent assertion succeeds), TWO for a new user
    // (silent assertion fast-fails, then create + dual-salt assert).
    let prfProvider = PasskeyProvider()
    let passkey = PasskeyClient(prfProvider: prfProvider, relayConfig: nil)

    do {
        // Discovery mode (label = nil): derives master + DEFAULT label
        // in a single ceremony. The fresh-device user fast-fails in
        // <300ms with no UI shown.
        let response = try await passkey.signIn(
            request: SignInRequest(label: nil, extraSalts: [])
        )
        return response.wallet
    } catch PrfProviderError.CredentialNotFound {
        // CredentialNotFound is the SDK's classification for "no matching
        // credential on this device", including iOS's <300ms fast-fail
        // case where the platform conflates no-cred with user-cancel.
        let response = try await passkey.register(
            request: RegisterRequest(
                label: "personal",
                extraSalts: [],
                excludeCredentialIds: []
            )
        )
        return response.wallet
    }
    // ANCHOR_END: signin-fallback-register
}

func checkDomain() async throws {
    // ANCHOR: domain-association
    // Verify Apple AASA / Android Asset Links / Web Related Origins
    // before the first WebAuthn ceremony. Diagnostic only: never blocks.
    let prfProvider = PasskeyProvider()
    let result = try await prfProvider.checkDomainAssociation()

    switch result {
    case .associated:
        // Safe to proceed.
        break
    case .notAssociated(let source, let reason):
        // Configuration is wrong (entitlement missing, AASA stale,
        // assetlinks malformed). Surface a developer-facing error.
        print("Domain association failed (source=\(source)): \(reason)")
        return
    case .skipped:
        // Verification could not be performed (offline, endpoint
        // timeout, no public-suffix match). Proceed normally: this
        // is NOT a negative signal.
        break
    }
    // ANCHOR_END: domain-association
}

func recoverFromAlreadyExists() async throws -> Wallet {
    // ANCHOR: recover-already-exists
    // The OS rejected register because the user's password manager
    // already holds a credential matching `excludeCredentialIds`.
    // Route the user to the sign-in path: the OS picker will surface
    // the existing credential and the SDK's identity cache will warm
    // up on the assertion.
    let prfProvider = PasskeyProvider()
    let passkey = PasskeyClient(prfProvider: prfProvider, relayConfig: nil)

    do {
        let response = try await passkey.register(
            request: RegisterRequest(
                label: "personal",
                extraSalts: [],
                excludeCredentialIds: [
                    // app-persisted credential IDs from prior registrations
                ]
            )
        )
        return response.wallet
    } catch PrfProviderError.CredentialAlreadyExists {
        // Flip to sign-in. The existing credential's PRF output is
        // the same wallet seed the host would have minted on register.
        let response = try await passkey.signIn(
            request: SignInRequest(label: "personal", extraSalts: [])
        )
        return response.wallet
    }
    // ANCHOR_END: recover-already-exists
}

func handleTimeout() async throws -> SignInResponse {
    // ANCHOR: handle-timeout
    // The OS biometric inactivity timeout (~55s+) tore down the prompt
    // without user intent. Distinct from a real cancel: hosts may
    // surface a re-prompt UI without treating it as the user opting
    // out. The SDK fires PrfProviderError.UserTimedOut when assertion
    // or register elapsed time crosses 55_000 ms.
    let prfProvider = PasskeyProvider()
    let passkey = PasskeyClient(prfProvider: prfProvider, relayConfig: nil)

    do {
        return try await passkey.signIn(
            request: SignInRequest(label: "personal", extraSalts: [])
        )
    } catch PrfProviderError.UserTimedOut {
        // Show a sticky retry screen with timeout-specific copy.
        // Do NOT auto-retry without user input.
        print("Sign-in timed out: show \"Try Again\" UI.")
        throw PrfProviderError.UserTimedOut
    }
    // ANCHOR_END: handle-timeout
}
