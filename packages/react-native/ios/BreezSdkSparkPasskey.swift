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
        autoRegister: Bool,
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
                    userName: userName, userDisplayName: userDisplayName,
                    autoRegister: autoRegister
                )
                resolve(result.base64EncodedString())
            } catch PasskeyError.userCancelled {
                reject("ERR_USER_CANCELLED", "User cancelled authentication", nil)
            } catch PasskeyError.prfNotSupported {
                reject("ERR_PRF_NOT_SUPPORTED", "PRF not supported by authenticator", nil)
            } catch PasskeyError.configuration(let msg) {
                reject("ERR_CONFIGURATION", msg, nil)
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
        excludeCredentialIds: [String],
        resolve: @escaping RCTPromiseResolveBlock,
        reject: @escaping RCTPromiseRejectBlock
    ) {
        let excludeIds: [Data] = excludeCredentialIds.compactMap { Data(base64Encoded: $0) }

        Task { @MainActor in
            do {
                let registered = try await registerCredential(
                    rpId: rpId, rpName: rpName,
                    userName: userName, userDisplayName: userDisplayName,
                    excludeCredentialIds: excludeIds
                )
                resolve([
                    "credentialId": registered.credentialId.base64EncodedString(),
                    "aaguid": registered.aaguid?.base64EncodedString() as Any?,
                    "backupEligible": registered.backupEligible as Any?,
                ])
            } catch PasskeyError.userCancelled {
                reject("ERR_USER_CANCELLED", "User cancelled registration", nil)
            } catch PasskeyError.prfNotSupported {
                reject("ERR_PRF_NOT_SUPPORTED", "PRF not supported by authenticator", nil)
            } catch PasskeyError.configuration(let msg) {
                reject("ERR_CONFIGURATION", msg, nil)
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
        userName: String, userDisplayName: String,
        autoRegister: Bool
    ) async throws -> Data {
        do {
            return try await assertionWithPrf(saltData: saltData, rpId: rpId)
        } catch PasskeyError.credentialNotFound {
            guard autoRegister else { throw PasskeyError.credentialNotFound }

            do {
                _ = try await registerCredential(
                    rpId: rpId, rpName: rpName,
                    userName: userName, userDisplayName: userDisplayName
                )
            } catch PasskeyError.credentialNotFound {
                // Registration also got notHandled: entitlement or
                // domain association is misconfigured, not a missing credential.
                throw PasskeyError.configuration(
                    "Associated Domains entitlement not configured. "
                    + "Add 'webcredentials:\(rpId)' to your app's entitlements "
                    + "and ensure a valid provisioning profile."
                )
            }
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

    @discardableResult
    private func registerCredential(
        rpId: String, rpName: String,
        userName: String, userDisplayName: String,
        excludeCredentialIds: [Data] = []
    ) async throws -> RegisteredCredential {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = randomBytes(count: 32)
        let userId = randomBytes(count: 16)
        let request = provider.createCredentialRegistrationRequest(
            challenge: challenge,
            name: userName,
            userID: userId
        )

        if !excludeCredentialIds.isEmpty {
            request.excludedCredentials = excludeCredentialIds.map {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
            }
        }

        let prfInput = ASAuthorizationPublicKeyCredentialPRFRegistrationInput()
        request.prf = prfInput

        let delegate = PasskeyDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate

        return try await withCheckedThrowingContinuation { continuation in
            delegate.registrationContinuation = continuation
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

// MARK: - Registered credential metadata

fileprivate struct RegisteredCredential {
    let credentialId: Data
    let aaguid: Data?
    let backupEligible: Bool?
}

/// Extract AAGUID + BE flag from the attestation object's authenticator
/// data via byte-pattern search for the "authData" CBOR key.
fileprivate func extractRegistrationMetadata(from attestation: Data) -> (aaguid: Data, backupEligible: Bool)? {
    let bytes = [UInt8](attestation)
    let key: [UInt8] = [0x68, 0x61, 0x75, 0x74, 0x68, 0x44, 0x61, 0x74, 0x61]
    guard bytes.count >= key.count else { return nil }
    var keyEnd = -1
    for i in 0...(bytes.count - key.count) {
        var match = true
        for j in 0..<key.count where bytes[i + j] != key[j] {
            match = false
            break
        }
        if match { keyEnd = i + key.count; break }
    }
    guard keyEnd >= 0 && keyEnd < bytes.count else { return nil }
    let header = bytes[keyEnd]
    guard header >> 5 == 2 else { return nil }
    let minor = Int(header & 0x1f)
    let length: Int
    let dataStart: Int
    switch minor {
    case 0..<24: length = minor; dataStart = keyEnd + 1
    case 24:
        guard keyEnd + 1 < bytes.count else { return nil }
        length = Int(bytes[keyEnd + 1]); dataStart = keyEnd + 2
    case 25:
        guard keyEnd + 2 < bytes.count else { return nil }
        length = (Int(bytes[keyEnd + 1]) << 8) | Int(bytes[keyEnd + 2])
        dataStart = keyEnd + 3
    case 26:
        guard keyEnd + 4 < bytes.count else { return nil }
        length = (Int(bytes[keyEnd + 1]) << 24) | (Int(bytes[keyEnd + 2]) << 16)
            | (Int(bytes[keyEnd + 3]) << 8) | Int(bytes[keyEnd + 4])
        dataStart = keyEnd + 5
    default: return nil
    }
    guard dataStart + length <= bytes.count, length >= 53 else { return nil }
    let flags = bytes[dataStart + 32]
    guard flags & 0x40 != 0 else { return nil }
    let backupEligible = flags & 0x08 != 0
    let aaguid = Data(bytes[(dataStart + 37)..<(dataStart + 53)])
    return (aaguid: aaguid, backupEligible: backupEligible)
}

// MARK: - Passkey Delegate

@available(iOS 18.0, *)
private class PasskeyDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    var continuation: CheckedContinuation<Data, Error>?
    var registrationContinuation: CheckedContinuation<RegisteredCredential, Error>?
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
            guard let credential = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialRegistration
            else {
                registrationContinuation?.resume(
                    throwing: PasskeyError.authenticationFailed("Unexpected credential type"))
                return
            }
            if let prfOutput = credential.prf, !prfOutput.isSupported {
                registrationContinuation?.resume(throwing: PasskeyError.prfNotSupported)
                return
            }
            var aaguid: Data? = nil
            var backupEligible: Bool? = nil
            if let attestation = credential.rawAttestationObject,
               let meta = extractRegistrationMetadata(from: attestation)
            {
                aaguid = meta.aaguid
                backupEligible = meta.backupEligible
            }
            registrationContinuation?.resume(
                returning: RegisteredCredential(
                    credentialId: credential.credentialID,
                    aaguid: aaguid,
                    backupEligible: backupEligible
                ))
        }
    }

    func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        let nsError = error as NSError
        let mapped: PasskeyError
        if nsError.domain == ASAuthorizationError.errorDomain {
            switch ASAuthorizationError.Code(rawValue: nsError.code) {
            case .canceled: mapped = .userCancelled
            case .notHandled: mapped = .credentialNotFound
            default: mapped = .authenticationFailed(nsError.localizedDescription)
            }
        } else {
            mapped = .authenticationFailed(error.localizedDescription)
        }
        continuation?.resume(throwing: mapped)
        registrationContinuation?.resume(throwing: mapped)
    }
}

// MARK: - Error Types

private enum PasskeyError: Error {
    case userCancelled
    case credentialNotFound
    case prfNotSupported
    case configuration(String)
    case authenticationFailed(String)
}
