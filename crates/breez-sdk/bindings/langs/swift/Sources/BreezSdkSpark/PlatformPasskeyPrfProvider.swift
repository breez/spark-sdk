import AuthenticationServices
import Foundation
import PasskeyPRFHelperObjC

/// Platform passkey PRF provider for iOS/macOS using the AuthenticationServices framework.
///
/// Uses `ASAuthorizationPlatformPublicKeyCredentialProvider` with the PRF extension
/// to derive deterministic 32-byte seeds from passkeys.
///
/// On first use, if no credential exists for the RP ID, a new passkey is
/// automatically created (registered), then the assertion is retried.
///
/// Requirements:
/// - iOS 18.0+ / macOS 15.0+
/// - Associated Domains entitlement: `webcredentials:<rpId>`
/// - The domain's `apple-app-site-association` must list your app
///
/// Example:
/// ```swift
/// let prfProvider = PlatformPasskeyPrfProvider()
/// let passkey = Passkey(prfProvider: prfProvider, relayConfig: nil)
/// let wallet = try await passkey.getWallet(walletName: "personal")
/// ```
@available(iOS 18.0, macOS 15.0, *)
public class PlatformPasskeyPrfProvider: PasskeyPrfProvider {
    private let rpId: String
    private let rpName: String
    private let userName: String
    private let userDisplayName: String
    private let anchor: PresentationAnchorProvider

    /// Protocol for providing a presentation anchor for the authorization controller.
    public protocol PresentationAnchorProvider {
        func presentationAnchor() -> ASPresentationAnchor
    }

    /// Default anchor provider that uses the key window from connected scenes.
    private class DefaultAnchorProvider: PresentationAnchorProvider {
        func presentationAnchor() -> ASPresentationAnchor {
            #if os(iOS)
            if let scene = UIApplication.shared.connectedScenes
                .compactMap({ $0 as? UIWindowScene })
                .first(where: { $0.activationState == .foregroundActive }),
                let window = scene.windows.first(where: { $0.isKeyWindow }) {
                return window
            }
            // Fallback: first available window
            if let window = UIApplication.shared.connectedScenes
                .compactMap({ $0 as? UIWindowScene })
                .flatMap({ $0.windows })
                .first {
                return window
            }
            return ASPresentationAnchor()
            #elseif os(macOS)
            return NSApplication.shared.keyWindow ?? ASPresentationAnchor()
            #endif
        }
    }

    /// Create a new platform passkey PRF provider.
    ///
    /// - Parameters:
    ///   - rpId: Relying Party ID (default: "keys.breez.technology").
    ///     Must match the domain configured for cross-platform credential sharing.
    ///     Changing this after users have registered passkeys will make their existing
    ///     credentials undiscoverable.
    ///   - rpName: Display name for the RP (default: "Breez SDK").
    ///     Shown to the user during credential registration. Only used when creating
    ///     new passkeys; changing it does not affect existing credentials.
    ///   - userName: User name stored with the credential. Defaults to rpName. Only used
    ///     during registration; changing it does not affect existing credentials.
    ///   - userDisplayName: User display name shown in the passkey picker. Defaults to
    ///     userName. Only used during registration; changing it does not affect existing credentials.
    ///   - anchorProvider: Custom presentation anchor provider. If nil, uses the key window.
    public init(
        rpId: String = "keys.breez.technology",
        rpName: String = "Breez SDK",
        userName: String? = nil,
        userDisplayName: String? = nil,
        anchorProvider: PresentationAnchorProvider? = nil
    ) {
        self.rpId = rpId
        self.rpName = rpName
        self.userName = userName ?? rpName
        self.userDisplayName = userDisplayName ?? (userName ?? rpName)
        self.anchor = anchorProvider ?? DefaultAnchorProvider()
    }

    /// Derive a 32-byte seed from passkey PRF with the given salt.
    ///
    /// Authenticates the user via a platform passkey and evaluates the PRF extension.
    /// If no credential exists for this RP ID, a new passkey is created automatically.
    ///
    /// - Parameter salt: The salt string to use for PRF evaluation.
    /// - Returns: The 32-byte PRF output.
    /// - Throws: `PasskeyPrfError` if authentication fails, PRF is not supported, or the user cancels.
    public func derivePrfSeed(salt: String) async throws -> Data {
        guard let saltData = salt.data(using: .utf8) else {
            throw PasskeyPrfError.Generic("Failed to encode salt as UTF-8")
        }

        // Try assertion first (existing credential)
        do {
            return try await performAssertionWithPrf(saltData: saltData)
        } catch let error as PasskeyPrfError where error.isCredentialNotFound {
            // No credential found — register a new one and retry
            try await registerCredential()
            return try await performAssertionWithPrf(saltData: saltData)
        }
    }

    /// Create a new passkey with PRF support.
    ///
    /// Only registers the credential — no seed derivation. Triggers exactly
    /// 1 platform prompt. Use this to separate credential creation from
    /// derivation in multi-step onboarding flows.
    ///
    /// - Throws: `PasskeyPrfError` if the user cancels or PRF is not supported by the authenticator.
    public func createPasskey() async throws {
        try await registerCredential()
    }

    /// Check if a PRF-capable passkey is available on this device.
    ///
    /// - Returns: `true` if the platform supports passkeys with the PRF extension.
    public func isPrfAvailable() async throws -> Bool {
        return true // iOS 18+ always supports platform passkeys with PRF
    }

    // MARK: - Private

    private func performAssertionWithPrf(saltData: Data) async throws -> Data {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = randomBytes(count: 32)
        let assertionRequest = provider.createCredentialAssertionRequest(challenge: challenge)

        // Configure PRF extension via ObjC helper
        // (PRF types are NS_REFINED_FOR_SWIFT with no accessible Swift initializers)
        PasskeyPRFHelper.setAssertionPRFOn(assertionRequest, withSalt: saltData)

        let delegate = AuthorizationDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [assertionRequest])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate
        delegate.anchor = anchor.presentationAnchor()

        return try await withCheckedThrowingContinuation { continuation in
            delegate.continuation = continuation
            delegate.extractPrf = true
            controller.performRequests()
        }
    }

    private func registerCredential() async throws {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = randomBytes(count: 32)
        let userId = randomBytes(count: 16)
        let registrationRequest = provider.createCredentialRegistrationRequest(
            challenge: challenge,
            name: userName,
            userID: userId
        )

        // Request PRF support during registration via ObjC helper
        PasskeyPRFHelper.setRegistrationPRFOn(registrationRequest)

        let delegate = AuthorizationDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [registrationRequest])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate
        delegate.anchor = anchor.presentationAnchor()

        let _: Data = try await withCheckedThrowingContinuation { continuation in
            delegate.continuation = continuation
            delegate.extractPrf = false
            controller.performRequests()
        }
    }

    private func randomBytes(count: Int) -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        _ = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        return Data(bytes)
    }
}

// MARK: - Authorization Delegate

@available(iOS 18.0, macOS 15.0, *)
private class AuthorizationDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    var continuation: CheckedContinuation<Data, Error>?
    var anchor: ASPresentationAnchor = ASPresentationAnchor()
    var extractPrf = true

    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        return anchor
    }

    func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        if extractPrf {
            // Assertion — extract PRF output
            guard let credential = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialAssertion
            else {
                continuation?.resume(
                    throwing: PasskeyPrfError.AuthenticationFailed(
                        "Unexpected credential type"))
                return
            }

            guard let prfData = PasskeyPRFHelper.extractPRFOutput(from: credential) else {
                continuation?.resume(throwing: PasskeyPrfError.PrfNotSupported)
                return
            }

            continuation?.resume(returning: prfData)
        } else {
            // Registration complete — PRF support is implicit on iOS 18+ / macOS 15+
            continuation?.resume(returning: Data())
        }
    }

    func authorizationController(
        controller: ASAuthorizationController, didCompleteWithError error: Error
    ) {
        let mapped = mapAuthorizationError(error)
        continuation?.resume(throwing: mapped)
    }

    private func mapAuthorizationError(_ error: Error) -> PasskeyPrfError {
        let nsError = error as NSError

        if nsError.domain == ASAuthorizationError.errorDomain {
            switch ASAuthorizationError.Code(rawValue: nsError.code) {
            case .canceled:
                return .UserCancelled
            case .unknown:
                if nsError.localizedDescription.contains("no credential")
                    || nsError.localizedDescription.contains("No credentials")
                {
                    return .CredentialNotFound
                }
                return .AuthenticationFailed(nsError.localizedDescription)
            case .invalidResponse:
                return .PrfEvaluationFailed(nsError.localizedDescription)
            case .notHandled:
                return .CredentialNotFound
            case .failed:
                return .AuthenticationFailed(nsError.localizedDescription)
            case .notInteractive:
                return .AuthenticationFailed("User interaction required")
            case .matchedExcludedCredential:
                return .AuthenticationFailed("Credential already registered")
            default:
                return .Generic(nsError.localizedDescription)
            }
        }

        return .Generic(error.localizedDescription)
    }
}

// MARK: - Error Extension

@available(iOS 18.0, macOS 15.0, *)
extension PasskeyPrfError {
    /// Whether this error indicates no credential was found (recoverable by registration).
    var isCredentialNotFound: Bool {
        switch self {
        case .CredentialNotFound:
            return true
        default:
            return false
        }
    }
}
