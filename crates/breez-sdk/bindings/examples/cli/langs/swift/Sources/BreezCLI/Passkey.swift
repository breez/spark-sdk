import Foundation
import BreezSdkSpark
import CommonCrypto

// MARK: - Passkey provider types

enum PasskeyProviderType: String {
    case platform
    case file
    case yubikey
    case fido2
}

// MARK: - Passkey configuration

struct PasskeyConfig {
    let provider: PasskeyProviderType
    let label: String?
    let listLabels: Bool
    let storeLabel: Bool
    let rpid: String?
}

// MARK: - File-based PRF provider

/// File-based implementation of `PrfProvider`.
///
/// Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
/// randomly on first use and persisted to disk.
class FilePrfProvider: PrfProvider {
    private let secret: Data

    private static let secretFileName = "seedless-restore-secret"

    init(dataDir: String) throws {
        let secretPath = (dataDir as NSString).appendingPathComponent(Self.secretFileName)
        let fm = FileManager.default

        if fm.fileExists(atPath: secretPath) {
            let bytes = try Data(contentsOf: URL(fileURLWithPath: secretPath))
            guard bytes.count == 32 else {
                throw PrfProviderError.Generic( "Invalid secret file: expected 32 bytes, got \(bytes.count)")
            }
            self.secret = bytes
        } else {
            // Generate new random secret
            var randomBytes = [UInt8](repeating: 0, count: 32)
            let status = SecRandomCopyBytes(kSecRandomDefault, 32, &randomBytes)
            guard status == errSecSuccess else {
                throw PrfProviderError.Generic( "Failed to generate random secret")
            }

            // Ensure data directory exists
            try fm.createDirectory(atPath: dataDir, withIntermediateDirectories: true)

            // Save secret to file
            let data = Data(randomBytes)
            try data.write(to: URL(fileURLWithPath: secretPath))
            self.secret = data
        }
    }

    private func hmac(salt: String) -> Data {
        let saltData = Data(salt.utf8)
        var out = [UInt8](repeating: 0, count: Int(CC_SHA256_DIGEST_LENGTH))
        secret.withUnsafeBytes { secretPtr in
            saltData.withUnsafeBytes { saltPtr in
                CCHmac(
                    CCHmacAlgorithm(kCCHmacAlgSHA256),
                    secretPtr.baseAddress, secret.count,
                    saltPtr.baseAddress, saltData.count,
                    &out
                )
            }
        }
        return Data(out)
    }

    func deriveSeeds(request: DeriveSeedsRequest) async throws -> DeriveSeedsOutput {
        let seeds = request.salts.map { hmac(salt: $0) }
        return DeriveSeedsOutput(seeds: seeds, credentialId: nil)
    }

    func isSupported() async throws -> Bool { true }

    func createPasskey(excludeCredentials: [Data]) async throws -> PasskeyCredential {
        throw PrfProviderError.Generic(
            "File-backed PRF provider does not implement create-credential; " +
            "use sign-in by label instead."
        )
    }

    func checkDomainAssociation() async throws -> DomainAssociation {
        .skipped(reason: "FilePrfProvider does not verify domain association")
    }
}

// MARK: - YubiKey PRF provider (skeleton)

/// YubiKey-based PRF provider using HMAC-SHA1 challenge-response.
///
/// This is a skeleton implementation. A full implementation would require
/// a Swift YubiKey library (e.g., YubiKit from Yubico).
class YubiKeyPrfProvider: PrfProvider {
    private func notYet() -> PrfProviderError {
        .Generic(
            "YubiKey PRF provider is not yet supported in the Swift CLI. " +
            "See the Rust CLI for a reference implementation using yubico-manager."
        )
    }

    func deriveSeeds(request: DeriveSeedsRequest) async throws -> DeriveSeedsOutput {
        throw notYet()
    }

    func isSupported() async throws -> Bool { false }

    func createPasskey(excludeCredentials: [Data]) async throws -> PasskeyCredential {
        throw notYet()
    }

    func checkDomainAssociation() async throws -> DomainAssociation {
        .skipped(reason: "YubiKeyPrfProvider does not verify domain association")
    }
}

// MARK: - FIDO2 PRF provider (skeleton)

/// FIDO2/WebAuthn PRF provider using CTAP2 hmac-secret extension.
///
/// This is a skeleton implementation. A full implementation would require
/// a Swift FIDO2/CTAP2 library with HID transport support.
class Fido2PrfProvider: PrfProvider {
    private func notYet() -> PrfProviderError {
        .Generic(
            "FIDO2 PRF provider is not yet supported in the Swift CLI. " +
            "See the Rust CLI for a reference implementation using ctap-hid-fido2."
        )
    }

    func deriveSeeds(request: DeriveSeedsRequest) async throws -> DeriveSeedsOutput {
        throw notYet()
    }

    func isSupported() async throws -> Bool { false }

    func createPasskey(excludeCredentials: [Data]) async throws -> PasskeyCredential {
        throw notYet()
    }

    func checkDomainAssociation() async throws -> DomainAssociation {
        .skipped(reason: "Fido2PrfProvider does not verify domain association")
    }
}

// MARK: - Provider factory

func createPrfProvider(type: PasskeyProviderType, dataDir: String, rpId: String? = nil) throws -> PrfProvider {
    switch type {
    case .platform:
        if #available(iOS 18.0, macOS 15.0, *) {
            return PasskeyProvider(
                options: PasskeyProviderOptions(rpId: rpId ?? "keys.breez.technology", rpName: "Breez SDK")
            )
        } else {
            throw PrfProviderError.Generic(
                "Platform passkey PRF requires iOS 18.0+ or macOS 15.0+"
            )
        }
    case .file:
        return try FilePrfProvider(dataDir: dataDir)
    case .yubikey:
        return YubiKeyPrfProvider()
    case .fido2:
        return Fido2PrfProvider()
    }
}

// MARK: - Passkey seed resolution

func resolvePasskeySeed(
    provider: PrfProvider,
    breezApiKey: String?,
    label: String?,
    listLabels: Bool,
    storeLabel: Bool
) async throws -> Seed {
    let passkey = PasskeyClient(prfProvider: provider, breezApiKey: breezApiKey, config: nil)

    // --list-labels: discovery sign-in (no cached label) returns the
    // published label set; prompt the user to pick one.
    let resolvedName: String?
    if listLabels {
        print("Querying Nostr for available labels...")
        let labels = try await passkey.signIn(request: SignInRequest(label: nil)).labels

        if labels.isEmpty {
            throw PrfProviderError.Generic(
                "No labels found on Nostr for this identity"
            )
        }

        print("Available labels:")
        for (i, name) in labels.enumerated() {
            print("  \(i + 1): \(name)")
        }

        guard let line = readlinePrompt("Select label (1-\(labels.count)): "),
              let idx = Int(line.trimmingCharacters(in: CharacterSet.whitespaces)),
              idx >= 1, idx <= labels.count
        else {
            throw PrfProviderError.Generic( "Invalid selection")
        }

        resolvedName = labels[idx - 1]
    } else {
        resolvedName = label
    }

    // --store-label: publish before signing in so a fresh client can
    // discover the label later.
    if storeLabel, let resolvedName {
        print("Publishing label '\(resolvedName)' to Nostr...")
        try await passkey.labels().store(label: resolvedName)
        print("Label '\(resolvedName)' published successfully.")
    }

    let response = try await passkey.signIn(request: SignInRequest(label: resolvedName))
    return response.wallet.seed
}
