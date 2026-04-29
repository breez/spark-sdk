import AuthenticationServices
import Flutter
import UIKit

/// Flutter plugin for passkey PRF operations on iOS.
///
/// Uses `ASAuthorizationPlatformPublicKeyCredentialProvider` with the PRF extension.
/// Auto-registers a new credential on first use if none exists.
@available(iOS 18.0, *)
public class BreezSdkSparkPasskeyPlugin: NSObject, FlutterPlugin {

    public static func register(with registrar: FlutterPluginRegistrar) {
        let channel = FlutterMethodChannel(
            name: "breez_sdk_spark_passkey",
            binaryMessenger: registrar.messenger()
        )
        let instance = BreezSdkSparkPasskeyPlugin()
        registrar.addMethodCallDelegate(instance, channel: channel)
    }

    public func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
        switch call.method {
        case "derivePrfSeed":
            guard let args = call.arguments as? [String: Any],
                  let salt = args["salt"] as? String,
                  let rpId = args["rpId"] as? String,
                  let rpName = args["rpName"] as? String,
                  let userName = args["userName"] as? String,
                  let userDisplayName = args["userDisplayName"] as? String
            else {
                result(FlutterError(code: "ERR_PASSKEY", message: "Invalid arguments", details: nil))
                return
            }

            guard let saltData = salt.data(using: .utf8) else {
                result(FlutterError(code: "ERR_PASSKEY", message: "Failed to encode salt as UTF-8", details: nil))
                return
            }

            Task { @MainActor in
                do {
                    let prfOutput = try await performDerivation(
                        saltData: saltData, rpId: rpId, rpName: rpName,
                        userName: userName, userDisplayName: userDisplayName
                    )
                    result(prfOutput.base64EncodedString())
                } catch PasskeyError.userCancelled {
                    result(FlutterError(code: "ERR_USER_CANCELLED", message: "User cancelled authentication", details: nil))
                } catch PasskeyError.prfNotSupported {
                    result(FlutterError(code: "ERR_PRF_NOT_SUPPORTED", message: "PRF not supported by authenticator", details: nil))
                } catch PasskeyError.configuration(let msg) {
                    result(FlutterError(code: "ERR_CONFIGURATION", message: msg, details: nil))
                } catch {
                    result(FlutterError(code: "ERR_PASSKEY", message: error.localizedDescription, details: nil))
                }
            }

        case "createPasskey":
            guard let args = call.arguments as? [String: Any],
                  let rpId = args["rpId"] as? String,
                  let rpName = args["rpName"] as? String,
                  let userName = args["userName"] as? String,
                  let userDisplayName = args["userDisplayName"] as? String
            else {
                result(FlutterError(code: "ERR_PASSKEY", message: "Invalid arguments", details: nil))
                return
            }

            var excludeCredentialIds: [Data] = []
            if let rawIds = args["excludeCredentialIds"] as? [FlutterStandardTypedData] {
                excludeCredentialIds = rawIds.map { $0.data }
            } else if let base64Ids = args["excludeCredentialIds"] as? [String] {
                excludeCredentialIds = base64Ids.compactMap { Data(base64Encoded: $0) }
            }

            Task { @MainActor in
                do {
                    let credentialId = try await registerCredential(
                        rpId: rpId, rpName: rpName,
                        userName: userName, userDisplayName: userDisplayName,
                        excludeCredentialIds: excludeCredentialIds
                    )
                    result(credentialId.base64EncodedString())
                } catch PasskeyError.userCancelled {
                    result(FlutterError(code: "ERR_USER_CANCELLED", message: "User cancelled registration", details: nil))
                } catch PasskeyError.prfNotSupported {
                    result(FlutterError(code: "ERR_PRF_NOT_SUPPORTED", message: "PRF not supported by authenticator", details: nil))
                } catch PasskeyError.configuration(let msg) {
                    result(FlutterError(code: "ERR_CONFIGURATION", message: msg, details: nil))
                } catch {
                    result(FlutterError(code: "ERR_PASSKEY", message: error.localizedDescription, details: nil))
                }
            }

        case "isPrfAvailable":
            if #available(iOS 18.0, *) {
                result(true)
            } else {
                result(false)
            }

        default:
            result(FlutterMethodNotImplemented)
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
            do {
                _ = try await registerCredential(
                    rpId: rpId, rpName: rpName,
                    userName: userName, userDisplayName: userDisplayName
                )
            } catch PasskeyError.credentialNotFound {
                // Registration also got notHandled: the entitlement or
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

        PasskeyPRFHelper.setAssertionPRFOn(request, withSalt: saltData)

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
    ) async throws -> Data {
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

        PasskeyPRFHelper.setRegistrationPRFOn(request)

        let delegate = PasskeyDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate

        return try await withCheckedThrowingContinuation { continuation in
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

            guard let prfData = PasskeyPRFHelper.extractPRFOutput(from: credential) else {
                continuation?.resume(throwing: PasskeyError.prfNotSupported)
                return
            }

            continuation?.resume(returning: prfData)
        } else {
            // Registration complete — extract and return the credential ID
            guard let credential = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialRegistration
            else {
                continuation?.resume(throwing: PasskeyError.authenticationFailed("Unexpected credential type"))
                return
            }
            continuation?.resume(returning: credential.credentialID)
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
    case configuration(String)
    case authenticationFailed(String)
}
