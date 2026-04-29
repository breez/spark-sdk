import AuthenticationServices
import Foundation
import React

/// React Native native module for passkey PRF operations on iOS.
///
/// Uses `ASAuthorizationPlatformPublicKeyCredentialProvider` with the PRF extension.
/// Auto-registers a new credential on first use if none exists.
@available(iOS 18.0, *)
@objc(BreezSdkSparkPasskey)
class BreezSdkSparkPasskey: NSObject {

    @objc
    static func requiresMainQueueSetup() -> Bool {
        return false
    }

    /// Derive a 32-byte PRF seed from a passkey assertion.
    ///
    /// - Parameters:
    ///   - salt: The salt string for PRF evaluation.
    ///   - rpId: The Relying Party ID.
    ///   - rpName: The RP display name (used during registration).
    ///   - userName: User name for credential registration.
    ///   - userDisplayName: User display name for credential registration.
    ///   - resolve: Resolves with a base64-encoded 32-byte PRF output.
    ///   - reject: Rejects with an error code and message.
    @objc
    func derivePrfSeed(
        _ salt: String,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        resolve: @escaping RCTPromiseResolveBlock,
        reject: @escaping RCTPromiseRejectBlock
    ) {
        guard let saltData = salt.data(using: .utf8) else {
            reject("ERR_PASSKEY", "Failed to encode salt as UTF-8", nil)
            return
        }

        Task { @MainActor in
            do {
                let result = try await performDerivation(
                    saltData: saltData, rpId: rpId, rpName: rpName,
                    userName: userName, userDisplayName: userDisplayName
                )
                resolve(result.base64EncodedString())
            } catch PasskeyError.userCancelled {
                reject("ERR_USER_CANCELLED", "User cancelled authentication", nil)
            } catch PasskeyError.prfNotSupported {
                reject("ERR_PRF_NOT_SUPPORTED", "PRF not supported by authenticator", nil)
            } catch {
                reject("ERR_PASSKEY", error.localizedDescription, nil)
            }
        }
    }

    /// Create a new passkey with PRF support.
    ///
    /// Only registers the credential — no seed derivation. Triggers exactly
    /// 1 platform prompt. Use for multi-step onboarding flows.
    @objc
    func createPasskey(
        _ rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        resolve: @escaping RCTPromiseResolveBlock,
        reject: @escaping RCTPromiseRejectBlock
    ) {
        Task { @MainActor in
            do {
                try await registerCredential(
                    rpId: rpId, rpName: rpName,
                    userName: userName, userDisplayName: userDisplayName
                )
                resolve(nil)
            } catch PasskeyError.userCancelled {
                reject("ERR_USER_CANCELLED", "User cancelled registration", nil)
            } catch PasskeyError.prfNotSupported {
                reject("ERR_PRF_NOT_SUPPORTED", "PRF not supported by authenticator", nil)
            } catch {
                reject("ERR_PASSKEY", error.localizedDescription, nil)
            }
        }
    }

    /// Check if PRF-capable passkeys are available on this device.
    @objc
    func isPrfAvailable(
        _ resolve: @escaping RCTPromiseResolveBlock,
        reject: @escaping RCTPromiseRejectBlock
    ) {
        if #available(iOS 18.0, *) {
            resolve(true)
        } else {
            resolve(false)
        }
    }

    // MARK: - Private

    private func performDerivation(
        saltData: Data, rpId: String, rpName: String,
        userName: String, userDisplayName: String
    ) async throws -> Data {
        do {
            return try await assertionWithPrf(saltData: saltData, rpId: rpId)
        } catch PasskeyError.credentialNotFound {
            try await registerCredential(
                rpId: rpId, rpName: rpName,
                userName: userName, userDisplayName: userDisplayName
            )
            return try await assertionWithPrf(saltData: saltData, rpId: rpId)
        }
    }

    private func assertionWithPrf(saltData: Data, rpId: String) async throws -> Data {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = randomBytes(count: 32)
        let request = provider.createCredentialAssertionRequest(challenge: challenge)

        let prfInput = ASAuthorizationPublicKeyCredentialPRFAssertionInput(
            inputValues: ASAuthorizationPublicKeyCredentialPRFValues(saltInput1: saltData)
        )
        request.prf = prfInput

        let delegate = PasskeyDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate

        return try await withCheckedThrowingContinuation { continuation in
            delegate.continuation = continuation
            delegate.extractPrf = true
            DispatchQueue.main.async {
                controller.performRequests()
            }
        }
    }

    private func registerCredential(
        rpId: String, rpName: String,
        userName: String, userDisplayName: String
    ) async throws {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = randomBytes(count: 32)
        let userId = randomBytes(count: 16)
        let request = provider.createCredentialRegistrationRequest(
            challenge: challenge,
            name: userName,
            userID: userId
        )

        let prfInput = ASAuthorizationPublicKeyCredentialPRFRegistrationInput()
        request.prf = prfInput

        let delegate = PasskeyDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate

        let _: Data = try await withCheckedThrowingContinuation { continuation in
            delegate.continuation = continuation
            delegate.extractPrf = false
            DispatchQueue.main.async {
                controller.performRequests()
            }
        }
    }

    private func randomBytes(count: Int) -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        _ = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        return Data(bytes)
    }
}

// MARK: - Passkey Delegate

@available(iOS 18.0, *)
private class PasskeyDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    var continuation: CheckedContinuation<Data, Error>?
    var extractPrf = true

    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        if let scene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first(where: { $0.activationState == .foregroundActive }),
            let window = scene.windows.first(where: { $0.isKeyWindow }) {
            return window
        }
        return ASPresentationAnchor()
    }

    func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        if extractPrf {
            guard let credential = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialAssertion
            else {
                continuation?.resume(throwing: PasskeyError.authenticationFailed("Unexpected credential type"))
                return
            }

            guard let prfOutput = credential.prf, let first = prfOutput.first else {
                continuation?.resume(throwing: PasskeyError.prfNotSupported)
                return
            }

            continuation?.resume(returning: first)
        } else {
            if let credential = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialRegistration
            {
                if let prfOutput = credential.prf, !prfOutput.isSupported {
                    continuation?.resume(throwing: PasskeyError.prfNotSupported)
                    return
                }
            }
            continuation?.resume(returning: Data())
        }
    }

    func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        let nsError = error as NSError
        if nsError.domain == ASAuthorizationError.errorDomain {
            switch ASAuthorizationError.Code(rawValue: nsError.code) {
            case .canceled:
                continuation?.resume(throwing: PasskeyError.userCancelled)
            case .notHandled:
                continuation?.resume(throwing: PasskeyError.credentialNotFound)
            default:
                continuation?.resume(
                    throwing: PasskeyError.authenticationFailed(nsError.localizedDescription))
            }
        } else {
            continuation?.resume(
                throwing: PasskeyError.authenticationFailed(error.localizedDescription))
        }
    }
}

// MARK: - Error Types

private enum PasskeyError: Error {
    case userCancelled
    case credentialNotFound
    case prfNotSupported
    case authenticationFailed(String)
}
