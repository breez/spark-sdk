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

    /// Shared across the per-call cores so the post-create PRF-readiness
    /// grace armed by `createPasskey` survives to an immediately-following
    /// `deriveSeeds` (sparing a second prompt). The registrar retains this
    /// plugin instance, so it persists across calls; mirrors the Android plugin.
    private let graceTracker = PostCreateGraceTracker()

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

        var excludeCredentials: [Data] = []
        if let rawIds = args["excludeCredentials"] as? [FlutterStandardTypedData] {
            excludeCredentials = rawIds.map { $0.data }
        } else if let base64Ids = args["excludeCredentials"] as? [String] {
            excludeCredentials = base64Ids.compactMap { Data(base64Encoded: $0) }
        }

        let core = PasskeyAssertionCore(
            rpId: rpId, rpName: rpName, userName: userName, userDisplayName: userDisplayName,
            graceTracker: graceTracker
        )
        Task { @MainActor in
            do {
                let registered = try await core.register(excludeCredentials: excludeCredentials)
                result([
                    "credentialId": registered.credentialId.base64EncodedString(),
                    "userId": registered.userId.base64EncodedString(),
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

        var allowCredentials: [Data] = []
        if let base64Ids = args["allowCredentials"] as? [String] {
            allowCredentials = base64Ids.compactMap { Data(base64Encoded: $0) }
        } else if let rawIds = args["allowCredentials"] as? [FlutterStandardTypedData] {
            allowCredentials = rawIds.map { $0.data }
        }
        let preferImmediate = args["preferImmediatelyAvailableCredentials"] as? Bool

        let core = PasskeyAssertionCore(
            rpId: rpId, rpName: rpName, userName: userName, userDisplayName: userDisplayName,
            graceTracker: graceTracker
        )
        Task { @MainActor in
            do {
                let derivation = try await core.deriveSeeds(
                    salts: saltDatas,
                    autoRegister: autoRegister,
                    allowCredentials: allowCredentials,
                    preferImmediatelyAvailableCredentials: preferImmediate ?? true
                )
                result([
                    "seeds": derivation.seeds.map { $0.base64EncodedString() },
                    "credentialId": derivation.credentialId.map { $0.base64EncodedString() } ?? NSNull(),
                ])
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
        // Optional explicit team ID: most callers leave it nil and let
        // the canonical core auto-detect from the running app's
        // signing info. Useful for unit tests / sandboxed contexts
        // where SecTask / provisioning-profile lookup doesn't work.
        let explicitTeamId = args["teamId"] as? String

        // Branding fields are unused by the domain check; pass rpId as
        // a placeholder since this is a check-only core.
        let core = PasskeyAssertionCore(
            rpId: rpId, rpName: rpId, userName: rpId, userDisplayName: rpId,
            explicitTeamId: explicitTeamId
        )
        Task { @MainActor in
            let outcome = await core.checkDomainAssociation()
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
        case .credentialNotFound(let msg):
            return FlutterError(code: "ERR_NO_CREDENTIAL", message: msg, details: nil)
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
