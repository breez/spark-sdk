import Foundation
import BreezSdkSpark
import CommonCrypto

// MARK: - Passkey provider types

enum PasskeyProviderType: String {
    case file
    case yubikey
    case fido2
}

// MARK: - Passkey configuration

struct PasskeyConfig {
    let provider: PasskeyProviderType
    let walletName: String?
    let listWalletNames: Bool
    let storeWalletName: Bool
    let rpid: String?
}

// MARK: - File-based PRF provider

/// File-based implementation of `PasskeyPrfProvider`.
///
/// Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
/// randomly on first use and persisted to disk.
class FilePrfProvider: PasskeyPrfProvider {
    private let secret: Data

    private static let secretFileName = "seedless-restore-secret"

    init(dataDir: String) throws {
        let secretPath = (dataDir as NSString).appendingPathComponent(Self.secretFileName)
        let fm = FileManager.default

        if fm.fileExists(atPath: secretPath) {
            let bytes = try Data(contentsOf: URL(fileURLWithPath: secretPath))
            guard bytes.count == 32 else {
                throw PasskeyPrfError.Generic( "Invalid secret file: expected 32 bytes, got \(bytes.count)")
            }
            self.secret = bytes
        } else {
            // Generate new random secret
            var randomBytes = [UInt8](repeating: 0, count: 32)
            let status = SecRandomCopyBytes(kSecRandomDefault, 32, &randomBytes)
            guard status == errSecSuccess else {
                throw PasskeyPrfError.Generic( "Failed to generate random secret")
            }

            // Ensure data directory exists
            try fm.createDirectory(atPath: dataDir, withIntermediateDirectories: true)

            // Save secret to file
            let data = Data(randomBytes)
            try data.write(to: URL(fileURLWithPath: secretPath))
            self.secret = data
        }
    }

    func derivePrfSeed(salt: String) async throws -> Data {
        // HMAC-SHA256(secret, salt)
        let saltData = Data(salt.utf8)
        var hmac = [UInt8](repeating: 0, count: Int(CC_SHA256_DIGEST_LENGTH))
        secret.withUnsafeBytes { secretPtr in
            saltData.withUnsafeBytes { saltPtr in
                CCHmac(
                    CCHmacAlgorithm(kCCHmacAlgSHA256),
                    secretPtr.baseAddress, secret.count,
                    saltPtr.baseAddress, saltData.count,
                    &hmac
                )
            }
        }
        return Data(hmac)
    }

    func isPrfAvailable() async throws -> Bool {
        true
    }
}

// MARK: - YubiKey PRF provider (skeleton)

/// YubiKey-based PRF provider using HMAC-SHA1 challenge-response.
///
/// This is a skeleton implementation. A full implementation would require
/// a Swift YubiKey library (e.g., YubiKit from Yubico).
class YubiKeyPrfProvider: PasskeyPrfProvider {
    func derivePrfSeed(salt: String) async throws -> Data {
        throw PasskeyPrfError.Generic(
            "YubiKey PRF provider is not yet supported in the Swift CLI. " +
                "See the Rust CLI for a reference implementation using yubico-manager."
        )
    }

    func isPrfAvailable() async throws -> Bool {
        false
    }
}

// MARK: - FIDO2 PRF provider (skeleton)

/// FIDO2/WebAuthn PRF provider using CTAP2 hmac-secret extension.
///
/// This is a skeleton implementation. A full implementation would require
/// a Swift FIDO2/CTAP2 library with HID transport support.
class Fido2PrfProvider: PasskeyPrfProvider {
    func derivePrfSeed(salt: String) async throws -> Data {
        throw PasskeyPrfError.Generic(
            "FIDO2 PRF provider is not yet supported in the Swift CLI. " +
                "See the Rust CLI for a reference implementation using ctap-hid-fido2."
        )
    }

    func isPrfAvailable() async throws -> Bool {
        false
    }
}

// MARK: - Provider factory

func createPrfProvider(type: PasskeyProviderType, dataDir: String) throws -> PasskeyPrfProvider {
    switch type {
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
    provider: PasskeyPrfProvider,
    breezApiKey: String?,
    walletName: String?,
    listWalletNames: Bool,
    storeWalletName: Bool
) async throws -> Seed {
    let relayConfig = NostrRelayConfig(breezApiKey: breezApiKey)
    let passkey = Passkey(prfProvider: provider, relayConfig: relayConfig)

    // --store-wallet-name: publish the wallet name to Nostr
    if storeWalletName, let walletName {
        print("Publishing wallet name '\(walletName)' to Nostr...")
        try await passkey.storeWalletName(walletName: walletName)
        print("Wallet name '\(walletName)' published successfully.")
    }

    // --list-wallet-names: query Nostr and prompt user to select
    let resolvedName: String?
    if listWalletNames {
        print("Querying Nostr for available wallet names...")
        let walletNames = try await passkey.listWalletNames()

        if walletNames.isEmpty {
            throw PasskeyPrfError.Generic(
                "No wallet names found on Nostr for this identity"
            )
        }

        print("Available wallet names:")
        for (i, name) in walletNames.enumerated() {
            print("  \(i + 1): \(name)")
        }

        guard let line = readlinePrompt("Select wallet name (1-\(walletNames.count)): "),
              let idx = Int(line.trimmingCharacters(in: .whitespaces)),
              idx >= 1, idx <= walletNames.count
        else {
            throw PasskeyPrfError.Generic( "Invalid selection")
        }

        resolvedName = walletNames[idx - 1]
    } else {
        resolvedName = walletName
    }

    let wallet = try await passkey.getWallet(walletName: resolvedName)
    return wallet.seed
}
