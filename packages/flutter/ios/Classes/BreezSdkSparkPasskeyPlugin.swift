import AuthenticationServices
import Flutter
import UIKit

/// Flutter plugin for passkey PRF operations on iOS.
///
/// Thin MethodChannel bridge over the shared `PasskeyAssertionCore`.
/// Behavioral parity with the upstream Swift `PasskeyProvider` and the
/// React Native module is enforced by routing all ASAuthorizationController
/// orchestration through the same canonical core.
@available(iOS 18.0, *)
public class BreezSdkSparkPasskeyPlugin: NSObject, FlutterPlugin {

    private let core = PasskeyAssertionCore()

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
        case "createPasskey":
            handleCreatePasskey(call: call, result: result)
        case "deriveSeeds":
            handleDeriveSeeds(call: call, result: result)
        case "isSupported":
            if #available(iOS 18.0, *) {
                result(true)
            } else {
                result(false)
            }
        case "checkDomainAssociation":
            handleCheckDomainAssociation(call: call, result: result)
        default:
            result(FlutterMethodNotImplemented)
        }
    }

    // MARK: - Method handlers

    private func handleCreatePasskey(call: FlutterMethodCall, result: @escaping FlutterResult) {
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

        var userIdOverride: Data? = nil
        if let typed = args["userId"] as? FlutterStandardTypedData {
            userIdOverride = typed.data
        } else if let base64UserId = args["userId"] as? String,
                  let decoded = Data(base64Encoded: base64UserId) {
            userIdOverride = decoded
        }

        Task { @MainActor in
            do {
                let registered = try await core.createPasskey(
                    rpId: rpId,
                    rpName: rpName,
                    userName: userName,
                    userDisplayName: userDisplayName,
                    excludeCredentialIds: excludeCredentialIds,
                    userId: userIdOverride
                )
                result([
                    "credentialId": registered.credentialId.base64EncodedString(),
                    "aaguid": registered.aaguid?.base64EncodedString() as Any?,
                    "backupEligible": registered.backupEligible as Any?,
                ])
            } catch let err as PasskeyAssertionError {
                result(Self.flutterError(from: err))
            } catch {
                result(FlutterError(code: "ERR_PASSKEY", message: error.localizedDescription, details: nil))
            }
        }
    }

    private func handleDeriveSeeds(call: FlutterMethodCall, result: @escaping FlutterResult) {
        guard let args = call.arguments as? [String: Any],
              let salts = args["salts"] as? [String],
              let rpId = args["rpId"] as? String,
              let rpName = args["rpName"] as? String,
              let userName = args["userName"] as? String,
              let userDisplayName = args["userDisplayName"] as? String
        else {
            result(FlutterError(code: "ERR_PASSKEY", message: "Invalid arguments", details: nil))
            return
        }

        let autoRegister = (args["autoRegister"] as? Bool) ?? false

        // Encode salts as UTF-8 bytes; the OS uses these directly as
        // PRF eval inputs (saltInput1, saltInput2).
        let saltDatas: [Data] = salts.compactMap { $0.data(using: .utf8) }
        guard saltDatas.count == salts.count else {
            result(FlutterError(code: "ERR_PASSKEY", message: "Failed to encode salts as UTF-8", details: nil))
            return
        }

        var allowCredentialIds: [Data] = []
        if let base64Ids = args["allowCredentialIds"] as? [String] {
            allowCredentialIds = base64Ids.compactMap { Data(base64Encoded: $0) }
        } else if let rawIds = args["allowCredentialIds"] as? [FlutterStandardTypedData] {
            allowCredentialIds = rawIds.map { $0.data }
        }

        Task { @MainActor in
            do {
                let seeds = try await core.performBulkDerivation(
                    salts: saltDatas,
                    rpId: rpId,
                    rpName: rpName,
                    userName: userName,
                    userDisplayName: userDisplayName,
                    autoRegister: autoRegister,
                    explicitAllowCredentialIds: allowCredentialIds
                )
                result(seeds.map { $0.base64EncodedString() })
            } catch let err as PasskeyAssertionError {
                result(Self.flutterError(from: err))
            } catch {
                result(FlutterError(code: "ERR_PASSKEY", message: error.localizedDescription, details: nil))
            }
        }
    }

    private func handleCheckDomainAssociation(call: FlutterMethodCall, result: @escaping FlutterResult) {
        guard let args = call.arguments as? [String: Any],
              let rpId = args["rpId"] as? String
        else {
            result(FlutterError(code: "ERR_PASSKEY", message: "Invalid arguments", details: nil))
            return
        }
        // Optional explicit team ID — most callers leave it nil and let
        // the canonical core auto-detect from the running app's
        // signing info. Useful for unit tests / sandboxed contexts
        // where SecTask / provisioning-profile lookup doesn't work.
        let explicitTeamId = args["teamId"] as? String

        Task { @MainActor in
            let outcome = await core.checkDomainAssociation(
                rpId: rpId,
                explicitTeamId: explicitTeamId
            )
            result(Self.serializeDomainAssociation(outcome))
        }
    }

    /// Serialize `IosDomainAssociation` into the JSON-shaped map the
    /// Dart side expects. Mirrors the typed `DomainAssociation`
    /// sealed class one-to-one.
    private static func serializeDomainAssociation(
        _ outcome: IosDomainAssociation
    ) -> [String: Any] {
        switch outcome {
        case .associated:
            return ["kind": "Associated"]
        case .notAssociated(let source, let reason):
            return ["kind": "NotAssociated", "source": source, "reason": reason]
        case .skipped(let reason):
            return ["kind": "Skipped", "reason": reason]
        }
    }

    // MARK: - Error mapping

    private static func flutterError(from err: PasskeyAssertionError) -> FlutterError {
        switch err {
        case .userCancelled:
            return FlutterError(code: "ERR_USER_CANCELLED", message: "User cancelled authentication", details: nil)
        case .userTimedOut:
            return FlutterError(code: "ERR_USER_TIMED_OUT", message: "Authenticator timed out", details: nil)
        case .credentialNotFound:
            return FlutterError(code: "ERR_NO_CREDENTIAL", message: "No matching passkey credential found", details: nil)
        case .credentialAlreadyExists(let msg):
            return FlutterError(code: "ERR_CREDENTIAL_ALREADY_EXISTS", message: msg, details: nil)
        case .prfNotSupported:
            return FlutterError(code: "ERR_PRF_NOT_SUPPORTED", message: "PRF not supported by authenticator", details: nil)
        case .prfEvaluationFailed(let msg):
            return FlutterError(code: "ERR_PRF_NOT_SUPPORTED", message: msg, details: nil)
        case .configuration(let msg):
            return FlutterError(code: "ERR_CONFIGURATION", message: msg, details: nil)
        case .authenticationFailed(let msg):
            return FlutterError(code: "ERR_PASSKEY", message: msg, details: nil)
        case .generic(let msg):
            return FlutterError(code: "ERR_PASSKEY", message: msg, details: nil)
        }
    }
}
