import AuthenticationServices
import Foundation
import React

/// React Native native module for passkey PRF operations on iOS.
///
/// Thin RCT_EXTERN_MODULE bridge over the shared `PasskeyAssertionCore`.
/// Behavioral parity with the upstream Swift `PasskeyProvider` and the
/// Flutter plugin is enforced by routing all ASAuthorizationController
/// orchestration through the same canonical core.
@available(iOS 18.0, *)
@objc(BreezSdkSparkPasskey)
class BreezSdkSparkPasskey: NSObject {

    private let core = PasskeyAssertionCore()

    @objc
    static func requiresMainQueueSetup() -> Bool {
        return false
    }

    /// Derive multiple PRF seeds in a single ceremony when supported.
    ///
    /// Walks `salts` two-at-a-time using a dual-salt assertion. Falls back
    /// to per-salt single-salt assertion if the authenticator drops the
    /// second salt. The `salts.count == 1` case short-circuits to a
    /// single-salt assertion under the hood (one prompt).
    @objc
    func deriveSeeds(
        _ salts: [String],
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        autoRegister: Bool,
        allowCredentialIds: [String],
        preferImmediatelyAvailableCredentials: NSNumber?,
        resolve: @escaping RCTPromiseResolveBlock,
        reject: @escaping RCTPromiseRejectBlock
    ) {
        // Encode salts as UTF-8 bytes; the OS uses these directly as
        // PRF eval inputs (saltInput1, saltInput2).
        let saltDatas: [Data] = salts.compactMap { $0.data(using: .utf8) }
        guard saltDatas.count == salts.count else {
            reject("ERR_PASSKEY", "Failed to encode salts as UTF-8", nil)
            return
        }

        let allowIds: [Data] = allowCredentialIds.compactMap { Data(base64Encoded: $0) }
        let preferImmediate = preferImmediatelyAvailableCredentials?.boolValue

        Task { @MainActor in
            do {
                let seeds = try await core.performBulkDerivation(
                    salts: saltDatas,
                    rpId: rpId,
                    rpName: rpName,
                    userName: userName,
                    userDisplayName: userDisplayName,
                    autoRegister: autoRegister,
                    options: DeriveSeedsOptions(
                        allowCredentialIds: allowIds,
                        preferImmediatelyAvailableCredentials: preferImmediate
                    )
                )
                resolve(seeds.map { $0.base64EncodedString() })
            } catch let err as PasskeyAssertionError {
                Self.reject(err, reject: reject, defaultMessage: "User cancelled authentication")
            } catch {
                reject("ERR_PASSKEY", error.localizedDescription, nil)
            }
        }
    }

    /// Create a new passkey with PRF support.
    ///
    /// Only registers the credential, no seed derivation. Triggers exactly
    /// 1 platform prompt. Use for multi-step onboarding flows. Per-call
    /// overrides on userId, userName, userDisplayName fall back to the
    /// constructor values when null.
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
                let registered = try await core.createPasskey(
                    rpId: rpId,
                    rpName: rpName,
                    userName: userName,
                    userDisplayName: userDisplayName,
                    excludeCredentialIds: excludeIds
                )
                resolve([
                    "credentialId": registered.credentialId.base64EncodedString(),
                    "userId": registered.userId.base64EncodedString(),
                    "aaguid": registered.aaguid?.base64EncodedString() as Any?,
                    "backupEligible": registered.backupEligible as Any?,
                ])
            } catch let err as PasskeyAssertionError {
                Self.reject(err, reject: reject, defaultMessage: "User cancelled registration")
            } catch {
                reject("ERR_PASSKEY", error.localizedDescription, nil)
            }
        }
    }

    /// Check if PRF-capable passkeys are available on this device.
    @objc
    func isSupported(
        _ resolve: @escaping RCTPromiseResolveBlock,
        reject: @escaping RCTPromiseRejectBlock
    ) {
        if #available(iOS 18.0, *) {
            resolve(true)
        } else {
            resolve(false)
        }
    }

    /// Domain association check. Delegates to the canonical
    /// `PasskeyAssertionCore` which probes Apple's AASA CDN
    /// (`app-site-association.cdn-apple.com`) for the bundle ID's
    /// `webcredentials` listing. Verification-level failures (no
    /// team ID, network errors, malformed JSON) all map to `Skipped`
    /// — the SDK never blocks on this check, it's diagnostic.
    @objc
    func checkDomainAssociation(
        _ rpId: String,
        teamId: String?,
        resolve: @escaping RCTPromiseResolveBlock,
        reject: @escaping RCTPromiseRejectBlock
    ) {
        Task { @MainActor in
            let outcome = await core.checkDomainAssociation(
                rpId: rpId,
                explicitTeamId: teamId
            )
            switch outcome {
            case .associated:
                resolve(["kind": "Associated"])
            case .notAssociated(let source, let reason):
                resolve(["kind": "NotAssociated", "source": source, "reason": reason])
            case .skipped(let reason):
                resolve(["kind": "Skipped", "reason": reason])
            }
        }
    }

    // MARK: - Error mapping

    private static func reject(
        _ err: PasskeyAssertionError,
        reject: @escaping RCTPromiseRejectBlock,
        defaultMessage: String
    ) {
        switch err {
        case .userCancelled:
            reject("ERR_USER_CANCELLED", defaultMessage, nil)
        case .userTimedOut:
            reject("ERR_USER_TIMED_OUT", "Authenticator timed out", nil)
        case .credentialNotFound(let msg):
            reject("ERR_NO_CREDENTIAL", msg, nil)
        case .credentialAlreadyExists(let msg):
            reject("ERR_CREDENTIAL_ALREADY_EXISTS", msg, nil)
        case .prfNotSupported:
            reject("ERR_PRF_NOT_SUPPORTED", "PRF not supported by authenticator", nil)
        case .prfEvaluationFailed(let msg):
            reject("ERR_PRF_NOT_SUPPORTED", msg, nil)
        case .configuration(let msg):
            reject("ERR_CONFIGURATION", msg, nil)
        case .authenticationFailed(let msg):
            reject("ERR_PASSKEY", msg, nil)
        case .generic(let msg):
            reject("ERR_PASSKEY", msg, nil)
        }
    }
}
