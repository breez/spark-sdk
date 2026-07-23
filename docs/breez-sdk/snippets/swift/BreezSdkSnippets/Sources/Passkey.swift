import BreezSdkSpark
import Foundation

// ANCHOR: implement-prf-provider
// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
// file-backed). Only deriveSeeds and isSupported are required.
class CustomPrfProvider: PrfProvider {
    func deriveSeeds(request: DeriveSeedsRequest) async throws -> DeriveSeedsOutput {
        // Return one 32-byte PRF output per salt, in input order.
        fatalError("Implement using WebAuthn or native passkey APIs")
    }

    func isSupported() async throws -> Bool {
        fatalError("Check platform passkey availability")
    }

    func createPasskey(excludeCredentials: [Data]) async throws -> PasskeyCredential {
        // Register a credential and return its ID plus attestation.
        fatalError("Implement registration via WebAuthn create() / native API")
    }

    func checkDomainAssociation() async throws -> DomainAssociation {
        return .skipped(reason: "CustomPrfProvider does not verify domain association")
    }
}
// ANCHOR_END: implement-prf-provider

func checkAvailability() async throws {
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    // ANCHOR: check-availability
    switch try await passkey.checkAvailability() {
    case .available:
        // Show passkey as primary option.
        break
    case .prfUnsupported:
        // Fall back to mnemonic flow.
        break
    case .notAssociated(let source, let reason):
        print("Domain association failed (source=\(source)): \(reason)")
    case .skipped:
        // No verification source on this platform; proceed normally.
        break
    }
    // ANCHOR_END: check-availability
}

func setupPasskeyClient() -> PasskeyClient {
    // ANCHOR: setup-client
    let passkey = PasskeyClient(
        breezApiKey: "<breez api key>",
        config: PasskeyConfig(
            providerOptions: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
        )
    )
    // ANCHOR_END: setup-client
    return passkey
}

func connectWithPasskey() async throws -> BreezSdk {
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    // ANCHOR: connect-with-passkey
    // Single-CTA onboarding: silent sign-in, fall through to register.
    var config = defaultConfig(network: .mainnet)
    config.apiKey = "<breez api key>"

    let response = try await passkey.connectWithPasskey(
        request: ConnectWithPasskeyRequest()
    )

    if response.labels.count > 1 {
        // Returning multi-wallet user: let them pick a label, then sign in to it.
        // let chosen = promptForLabel(response.labels)
        // return try await passkey.signIn(request: SignInRequest(label: chosen))
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
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    // ANCHOR: sign-in
    // Returning-user sign-in. No fall-through to register.
    return try await passkey.signIn(request: SignInRequest(label: "personal"))
    // ANCHOR_END: sign-in
}

func registerNewPasskey() async throws -> BreezSdk {
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    // ANCHOR: register-passkey
    var config = defaultConfig(network: .mainnet)
    config.apiKey = "<breez api key>"

    let response = try await passkey.register(
        request: RegisterRequest(label: "personal")
    )

    let sdk = try await connect(
        request: ConnectRequest(
            config: config,
            seed: response.wallet.seed,
            storageDir: "./.data"
        ))
    // ANCHOR_END: register-passkey
    return sdk
}

func credentialMetadata() async throws {
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: nil, config: nil)

    // ANCHOR: credential-metadata
    let response = try await passkey.register(
        request: RegisterRequest(label: "personal")
    )

    if let credential = response.credential {
        // Persist to reopen the same wallet on sign-in
        print(credential.credentialId)
        // Authenticator model (display hint, unverified)
        print(credential.aaguid)
        // Whether the passkey syncs across devices
        print(credential.backupEligible)
    }

    // Pin the stored credential ID so the OS can't substitute a sibling
    // credential, which would derive a different wallet.
    let signInResponse = try await passkey.signIn(
        request: SignInRequest(
            label: "personal",
            allowCredentials: [
                // stored credentialId bytes
            ]
        )
    )
    // Pass to connect() to open the wallet
    print(signInResponse.wallet.seed)
    // Label this wallet was derived from
    print(signInResponse.wallet.label)
    // This passkey's labels (populated on discovery sign-in)
    print(signInResponse.labels)
    // Credential signed in with (credential_id only)
    print(signInResponse.credential)
    // ANCHOR_END: credential-metadata
}

func listLabels() async throws -> [String] {
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
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
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)
    // ANCHOR: store-label
    try await passkey.labels().store(label: "personal")
    // ANCHOR_END: store-label
}

func checkDomain() async throws {
    // ANCHOR: domain-association
    // Lower-level provider call. Most hosts use `checkAvailability` instead.
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
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
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    // ANCHOR: recover-already-exists
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
        // A matching credential already exists; sign in instead.
        let response = try await passkey.signIn(
            request: SignInRequest(label: "personal")
        )
        return response.wallet
    }
    // ANCHOR_END: recover-already-exists
}

func handleTimeout() async throws -> SignInResponse {
    let prfProvider = PasskeyProvider(
        options: PasskeyProviderOptions(rpId: "<your-rp-domain>", rpName: "Your App")
    )
    let passkey = PasskeyClient(prfProvider: prfProvider, breezApiKey: "<breez api key>", config: nil)

    // ANCHOR: handle-timeout
    do {
        return try await passkey.signIn(
            request: SignInRequest(label: "personal")
        )
    } catch PrfProviderError.UserTimedOut {
        // Show a retry UI. Do NOT auto-retry without user input.
        print("Sign-in timed out: show \"Try Again\" UI.")
        throw PrfProviderError.UserTimedOut
    }
    // ANCHOR_END: handle-timeout
}
