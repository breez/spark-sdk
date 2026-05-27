import AuthenticationServices
import Foundation
import os.log
import PasskeyPRFHelperObjC
import Security
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

// MARK: - Credential registry

/// App-side persistent store of credential IDs registered for an RP.
/// The SDK does not ship a built-in implementation: bring your own via
/// Keychain, Block Store, localStorage, or a custom backend. See the
/// reference implementations in the passkey guide.
///
/// All methods are called from the SDK as best-effort optimizations:
/// failures and timeouts (3s) are swallowed and surfaced via
/// `onRegistryError`; they never block the WebAuthn ceremony.
@available(iOS 18.0, macOS 15.0, *)
public protocol CredentialRegistry: Sendable {
    func read(rpId: String) async throws -> [Data]
    func add(rpId: String, credentialId: Data) async throws
    func remove(rpId: String, credentialId: Data) async throws
    func clear(rpId: String) async throws
}

/// Discriminator for [`CredentialRegistry`] callbacks. Identifies which
/// registry method failed when the SDK swallows the underlying error.
@available(iOS 18.0, macOS 15.0, *)
public enum RegistryOperation: Sendable {
    case read
    case add
    case remove
    case clear
}

/// 3 second timeout applied to every registry call. Not configurable.
@available(iOS 18.0, macOS 15.0, *)
private let registryTimeout: TimeInterval = 3.0

@available(iOS 18.0, macOS 15.0, *)
private struct RegistryTimeoutError: Error {
    let operation: RegistryOperation
}

/// Run `body` with a `registryTimeout` deadline. Throws
/// `RegistryTimeoutError` on timeout. `cancelAll()` only requests
/// cancellation cooperatively, so a non-cooperative registry backend
/// that ignores cancellation keeps running in the background until it
/// finishes; its result is simply discarded. Acceptable because
/// registry writes are advisory, but a pathological backend can leak a
/// background task.
@available(iOS 18.0, macOS 15.0, *)
private func withRegistryTimeout<T: Sendable>(
    operation: RegistryOperation,
    body: @Sendable @escaping () async throws -> T
) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { try await body() }
        group.addTask {
            try await Task.sleep(nanoseconds: UInt64(registryTimeout * 1_000_000_000))
            throw RegistryTimeoutError(operation: operation)
        }
        let result = try await group.next()!
        group.cancelAll()
        return result
    }
}

@available(iOS 18.0, macOS 15.0, *)
private let registryLog = OSLog(
    subsystem: "technology.breez.spark.passkey", category: "CredentialRegistry"
)

/// Best-effort registry read. On timeout / throw: log + invoke
/// `onRegistryError`, return `[]`.
@available(iOS 18.0, macOS 15.0, *)
private func registryReadBestEffort(
    registry: CredentialRegistry,
    rpId: String,
    onRegistryError: ((RegistryOperation, Error) -> Void)?
) async -> [Data] {
    do {
        return try await withRegistryTimeout(operation: .read) {
            try await registry.read(rpId: rpId)
        }
    } catch {
        os_log("CredentialRegistry.read failed: %{public}@",
               log: registryLog, type: .info,
               String(describing: error))
        onRegistryError?(.read, error)
        return []
    }
}

/// Best-effort registry write. On timeout / throw: log + invoke
/// `onRegistryError`, swallow.
@available(iOS 18.0, macOS 15.0, *)
private func registryAddBestEffort(
    registry: CredentialRegistry,
    rpId: String,
    credentialId: Data,
    onRegistryError: ((RegistryOperation, Error) -> Void)?
) async {
    do {
        try await withRegistryTimeout(operation: .add) {
            try await registry.add(rpId: rpId, credentialId: credentialId)
        }
    } catch {
        os_log("CredentialRegistry.add failed: %{public}@",
               log: registryLog, type: .info,
               String(describing: error))
        onRegistryError?(.add, error)
    }
}

/// Canonical iOS/macOS passkey PRF logic shared between the upstream
/// Swift `PrfProvider`, the Flutter MethodChannel plugin, and the
/// React Native `RCT_EXTERN_MODULE` bridge. All three wrap a single
/// `PasskeyAssertionCore` instance and translate `PasskeyAssertionError`
/// to whatever error type their layer expects.
///
/// Mirrors the Android shared `CredentialManagerPrfCore.kt`. Synced via
/// `cargo xtask sync-passkey-core`.

// MARK: - Post-create grace

/// A newly-registered passkey is sometimes not yet ready for the
/// immediate post-create assertion. On Apple Passwords this manifests
/// as a dual-salt PRF assertion returning `prf.first` but with
/// `prf.second == nil`, forcing us to fall back to a second single-salt
/// assertion (= 2 prompts instead of 1). On GPM the credential is
/// briefly invisible to the picker entirely. Holding the next derive
/// for up to 800ms after a successful create lets the OS finish
/// indexing. Mirrors the Capacitor plugin's `PostCreateGraceTracker`.
@available(iOS 18.0, macOS 15.0, *)
public actor PostCreateGraceTracker {
    public static let defaultTotal: TimeInterval = 0.8

    private var deadline: Date?

    public init() {}

    public func arm(after interval: TimeInterval = PostCreateGraceTracker.defaultTotal) {
        deadline = Date().addingTimeInterval(interval)
    }

    public func consume() async {
        guard let d = deadline else { return }
        deadline = nil
        let remaining = d.timeIntervalSinceNow
        if remaining > 0 {
            try? await Task.sleep(nanoseconds: UInt64(remaining * 1_000_000_000))
        }
    }
}

// MARK: - Error type

/// Layer-neutral error surface produced by `PasskeyAssertionCore`.
/// Wrappers translate to their own typed errors (UniFFI `PrfProviderError`,
/// `FlutterError`, RCT promise reject codes).
@available(iOS 18.0, macOS 15.0, *)
public enum PasskeyAssertionError: Error {
    case userCancelled
    /// The OS biometric prompt timed out without user interaction
    /// (~55s+ inactivity on iOS). Distinct from `userCancelled`, which
    /// means the user actively dismissed the prompt.
    case userTimedOut
    case credentialNotFound(String)
    case credentialAlreadyExists(String)
    case prfNotSupported
    case prfEvaluationFailed(String)
    case configuration(String)
    case authenticationFailed(String)
    case generic(String)
}

// MARK: - Registered credential

/// Result of a successful registration. Named `Ios*` so it does not
/// collide with the UniFFI-generated `RegisteredCredential` in the
/// upstream Swift wrapper; that wrapper translates to the FFI type.
///
/// `userId` is the WebAuthn user handle the core minted for this
/// credential. Always populated; the SDK never lets hosts supply one.
@available(iOS 18.0, macOS 15.0, *)
public struct IosRegisteredCredential {
    public let credentialId: Data
    public let userId: Data
    public let aaguid: Data?
    public let backupEligible: Bool?

    public init(credentialId: Data, userId: Data, aaguid: Data?, backupEligible: Bool?) {
        self.credentialId = credentialId
        self.userId = userId
        self.aaguid = aaguid
        self.backupEligible = backupEligible
    }
}

// MARK: - Domain association

/// Result of an Apple-app-site-association probe against the AASA CDN.
/// Layer-neutral; wrappers translate to UniFFI's `DomainAssociation`,
/// MethodChannel maps, or RCT-friendly dictionaries.
///
/// `Skipped` is the catch-all for verification-level failures: missing
/// team ID, missing bundle ID, network errors, malformed JSON. Callers
/// treat `Skipped` as advisory ("we couldn't tell") and proceed with
/// the WebAuthn ceremony: the SDK never blocks on it.
@available(iOS 18.0, macOS 15.0, *)
public enum IosDomainAssociation {
    case associated
    case notAssociated(source: String, reason: String)
    case skipped(reason: String)
}

// MARK: - Team ID detection

/// Auto-detect the Apple Developer Team ID from the running app's
/// signing information. Two different mechanisms per platform, both
/// yielding the same 10-character team ID:
///
/// - **macOS**: read the `application-identifier` entitlement via
///   `SecTaskCopyValueForEntitlement`. Entitlement format is
///   `<TEAM_ID>.<BUNDLE_ID>`; split on the first dot.
///
/// - **iOS**: parse `embedded.mobileprovision` from the app bundle.
///   Provisioning profiles are PKCS#7-wrapped plists. The plist bytes
///   are plain-text inside the CMS envelope, so we locate the
///   `<?xml>...</plist>` span and deserialize. The profile declares
///   `TeamIdentifier` (array of one entry).
///
/// Simulator and unsigned builds don't ship a provisioning profile,
/// so this returns nil and `checkDomainAssociation` reports `.skipped`.
///
/// Cached at first read: the team ID is stable for the lifetime of
/// the installed binary and detection is non-trivial.
@available(iOS 18.0, macOS 15.0, *)
public enum PasskeyTeamIdDetector {
    private static let cached: String? = {
        #if os(macOS)
        return detectFromSecTask()
        #elseif os(iOS)
        return detectFromProvisioningProfile()
        #else
        return nil
        #endif
    }()

    public static func detect() -> String? { cached }

    #if os(macOS)
    private static func detectFromSecTask() -> String? {
        guard let task = SecTaskCreateFromSelf(nil) else { return nil }
        let key = "application-identifier" as CFString
        var error: Unmanaged<CFError>?
        guard let value = SecTaskCopyValueForEntitlement(task, key, &error)
                as? String
        else { return nil }
        return parseFromApplicationIdentifier(value)
    }
    #endif

    #if os(iOS)
    private static func detectFromProvisioningProfile() -> String? {
        // Release / ad-hoc / TestFlight / Enterprise builds embed
        // `embedded.mobileprovision` at the bundle root.
        guard let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision"),
              let data = try? Data(contentsOf: url)
        else { return nil }
        // The PKCS#7 CMS envelope has binary DER bytes > 127. `.ascii`
        // rejects those; `.isoLatin1` is a 1:1 byte-to-codepoint map
        // that always succeeds. We only use the string to locate the
        // plist span; actual parsing uses the raw Data slice.
        guard let raw = String(data: data, encoding: .isoLatin1),
              let startRange = raw.range(of: "<?xml"),
              let endRange = raw.range(of: "</plist>")
        else { return nil }
        let startByteOffset = raw.utf16.distance(
            from: raw.utf16.startIndex,
            to: startRange.lowerBound.samePosition(in: raw.utf16) ?? raw.utf16.startIndex
        )
        let endByteOffset = raw.utf16.distance(
            from: raw.utf16.startIndex,
            to: endRange.upperBound.samePosition(in: raw.utf16) ?? raw.utf16.endIndex
        )
        guard startByteOffset < endByteOffset, endByteOffset <= data.count
        else { return nil }
        let plistData = data.subdata(in: startByteOffset..<endByteOffset)
        guard let plist = try? PropertyListSerialization.propertyList(
            from: plistData, format: nil
        ) as? [String: Any]
        else { return nil }
        if let team = (plist["TeamIdentifier"] as? [String])?.first {
            return validate(team)
        }
        if let prefix = (plist["ApplicationIdentifierPrefix"] as? [String])?.first {
            return validate(prefix)
        }
        return nil
    }
    #endif

    private static func parseFromApplicationIdentifier(_ value: String) -> String? {
        guard let firstDot = value.firstIndex(of: ".") else { return nil }
        return validate(String(value[..<firstDot]))
    }

    private static func validate(_ candidate: String) -> String? {
        guard candidate.count == 10,
              candidate.allSatisfy({ $0.isLetter || $0.isNumber })
        else { return nil }
        return candidate
    }
}

// MARK: - Presentation anchor

/// Layer-neutral presentation anchor protocol. Wrappers can supply a
/// custom anchor (e.g. SceneDelegate-aware) or fall back to the
/// platform default.
@available(iOS 18.0, macOS 15.0, *)
public protocol PasskeyPresentationAnchorProvider: AnyObject {
    func presentationAnchor() -> ASPresentationAnchor
}

@available(iOS 18.0, macOS 15.0, *)
public final class DefaultPasskeyPresentationAnchorProvider: PasskeyPresentationAnchorProvider {
    public init() {}

    public func presentationAnchor() -> ASPresentationAnchor {
        #if os(iOS)
        if let scene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first(where: { $0.activationState == .foregroundActive }),
            let window = scene.windows.first(where: { $0.isKeyWindow }) {
            return window
        }
        if let window = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .flatMap({ $0.windows })
            .first {
            return window
        }
        return ASPresentationAnchor()
        #elseif os(macOS)
        return NSApplication.shared.keyWindow ?? ASPresentationAnchor()
        #else
        return ASPresentationAnchor()
        #endif
    }
}

// MARK: - Core

/// Reusable WebAuthn PRF logic. Holds no per-request state. Methods
/// take an `rpId` plus credential metadata and return a typed result.
///
/// All ASAuthorizationController orchestration lives here:
/// - one assertion ceremony for 1-2 salts (`assertPrf`)
/// - bulk derivation walking salts in pairs (`deriveSeeds`)
/// - registration (`register`)
/// - opt-in `CredentialRegistry` auto-merge / auto-capture
/// - `preferImmediatelyAvailableCredentials` for fast-fail on no-cred
/// - AAGUID + BE flag extraction from attestation CBOR
/// - post-create grace tracker
@available(iOS 18.0, macOS 15.0, *)
public final class PasskeyAssertionCore {
    private let rpId: String
    private let rpName: String
    private let userName: String
    private let userDisplayName: String
    private let credentialRegistry: CredentialRegistry?
    private let onRegistryError: (@Sendable (RegistryOperation, Error) -> Void)?
    private let explicitTeamId: String?
    private let urlSession: URLSession
    private let anchor: PasskeyPresentationAnchorProvider
    private let graceTracker: PostCreateGraceTracker
    private let postCreateGraceTotal: TimeInterval

    public init(
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        credentialRegistry: CredentialRegistry? = nil,
        onRegistryError: (@Sendable (RegistryOperation, Error) -> Void)? = nil,
        explicitTeamId: String? = nil,
        urlSession: URLSession = .shared,
        anchorProvider: PasskeyPresentationAnchorProvider? = nil,
        graceTracker: PostCreateGraceTracker = PostCreateGraceTracker(),
        postCreateGraceTotal: TimeInterval = PostCreateGraceTracker.defaultTotal
    ) {
        self.rpId = rpId
        self.rpName = rpName
        self.userName = userName
        self.userDisplayName = userDisplayName
        self.credentialRegistry = credentialRegistry
        self.onRegistryError = onRegistryError
        self.explicitTeamId = explicitTeamId
        self.urlSession = urlSession
        self.anchor = anchorProvider ?? DefaultPasskeyPresentationAnchorProvider()
        self.graceTracker = graceTracker
        self.postCreateGraceTotal = postCreateGraceTotal
    }

    // MARK: Public entry points

    /// Derive one 32-byte PRF output per salt in as few authenticator
    /// ceremonies as the platform supports: salts are walked in pairs
    /// (one dual-salt assertion each via `prf.eval.first`/`.second`),
    /// and an authenticator that drops `second` is recovered with a
    /// single-salt re-assert. When no credential exists yet and
    /// [autoRegister] is set, the first miss registers a passkey and
    /// retries. Output ordering matches input ordering.
    public func deriveSeeds(
        salts: [Data],
        autoRegister: Bool,
        allowCredentials: [Data] = [],
        preferImmediatelyAvailableCredentials: Bool = true
    ) async throws -> (seeds: [Data], credentialId: Data?) {
        // Union the caller's allow-list with the registry's stored IDs.
        var allow = allowCredentials
        if let reg = credentialRegistry {
            var seen = Set(allow)
            for id in await registryReadBestEffort(registry: reg, rpId: rpId, onRegistryError: onRegistryError)
            where seen.insert(id).inserted {
                allow.append(id)
            }
        }
        // Wait out the post-create grace before any assertion so the
        // immediate setup_wallet derive doesn't race the credential's
        // PRF-readiness window (dual-salt would drop `second`, forcing
        // a second prompt).
        await graceTracker.consume()
        if salts.isEmpty { return (seeds: [], credentialId: nil) }

        // One assertion for 1-2 salts, registering + retrying once when
        // the device holds no credential. Returns (first, second?,
        // credentialId); `second` is nil when the authenticator dropped
        // saltInput2. `credentialId` is the asserted credential, observed
        // inline (same value across every chunk of one derive call).
        func assertChunk(_ salt1: Data, _ salt2: Data?) async throws -> (Data, Data?, Data) {
            do {
                return try await assertPrf(
                    salt1: salt1, salt2: salt2, allowCredentials: allow,
                    preferImmediatelyAvailableCredentials: preferImmediatelyAvailableCredentials
                )
            } catch PasskeyAssertionError.credentialNotFound(_) {
                guard autoRegister else {
                    throw augmentCredentialNotFound(
                        explicitAllowCredentials: allow, credentialRegistry: credentialRegistry
                    )
                }
                do {
                    _ = try await register()
                } catch PasskeyAssertionError.credentialNotFound(_) {
                    throw PasskeyAssertionError.configuration(
                        "Associated Domains entitlement not configured. "
                        + "Add 'webcredentials:\(rpId)' to your app's entitlements "
                        + "and ensure a valid provisioning profile."
                    )
                }
                // Retry once. A second miss (e.g. the user deleted the
                // pinned credential from Settings) escapes as
                // credentialNotFound for hosts to treat as deletion
                // recovery.
                return try await assertPrf(
                    salt1: salt1, salt2: salt2, allowCredentials: allow,
                    preferImmediatelyAvailableCredentials: preferImmediatelyAvailableCredentials
                )
            }
        }

        var out: [Data] = []
        // The asserted credential ID, captured inline from each ceremony
        // (identical across chunks) and returned so the binding layer can
        // surface it on `SignInResponse.credential_id` without a separate
        // read-and-clear call.
        var observedCredentialId: Data?
        var i = 0
        while i < salts.count {
            if i + 1 < salts.count {
                let (first, second, credId) = try await assertChunk(salts[i], salts[i + 1])
                observedCredentialId = credId
                out.append(first)
                if let second = second {
                    out.append(second)
                } else {
                    // Authenticator dropped `second`: single-salt recover.
                    let (recovered, _, recoveredCredId) = try await assertChunk(salts[i + 1], nil)
                    out.append(recovered)
                    observedCredentialId = recoveredCredId
                }
                i += 2
            } else {
                let (single, _, credId) = try await assertChunk(salts[i], nil)
                out.append(single)
                observedCredentialId = credId
                i += 1
            }
        }
        return (seeds: out, credentialId: observedCredentialId)
    }

    /// Verify the app's bundle identifier is listed in the
    /// `webcredentials` section of the AASA file served for `rpId`,
    /// via Apple's app-site-association CDN
    /// (`https://app-site-association.cdn-apple.com/a/v1/<rpId>`).
    ///
    /// The CDN is the same source the OS uses to validate Associated
    /// Domains, but exposed as a public HTTP endpoint so we can
    /// proactively surface misconfiguration ("Bundle ID `<team>.<bid>`
    /// is not in `webcredentials.apps` for `rpId`") to integrators
    /// before a WebAuthn ceremony fires and fails opaquely.
    ///
    /// Resolves the team ID from `explicitTeamId` (when set) or
    /// auto-detects from the running app's signing info via
    /// `PasskeyTeamIdDetector`. If both fail (unsigned test builds),
    /// returns `.skipped`.
    ///
    /// Never throws: verification-level failures (no bundle ID, no
    /// team ID, network errors, malformed JSON) all map to `.skipped`
    /// so callers have one surface to handle.
    public func checkDomainAssociation() async -> IosDomainAssociation {
        let bundleId = Bundle.main.bundleIdentifier ?? ""
        guard !bundleId.isEmpty else {
            return .skipped(
                reason: "Bundle.main.bundleIdentifier is empty (unsigned / test context?)"
            )
        }

        guard let teamId = explicitTeamId ?? PasskeyTeamIdDetector.detect() else {
            return .skipped(
                reason:
                    "Could not resolve Apple Developer Team ID "
                    + "(no explicit teamId and SecTaskCopyValueForEntitlement "
                    + "lookup failed)"
            )
        }

        let fullAppId = "\(teamId).\(bundleId)"
        let cdnUrl = "https://app-site-association.cdn-apple.com/a/v1/\(rpId)"
        guard let url = URL(string: cdnUrl) else {
            return .skipped(reason: "Invalid AASA CDN URL: \(cdnUrl)")
        }

        var request = URLRequest(url: url)
        request.timeoutInterval = 3.0
        request.httpMethod = "GET"

        do {
            let (data, response) = try await urlSession.data(for: request)
            guard let httpResponse = response as? HTTPURLResponse else {
                return .skipped(reason: "AASA CDN returned non-HTTP response")
            }
            guard httpResponse.statusCode == 200 else {
                return .skipped(
                    reason: "AASA CDN returned HTTP \(httpResponse.statusCode)"
                )
            }
            guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let webcredentials = json["webcredentials"] as? [String: Any],
                  let apps = webcredentials["apps"] as? [String]
            else {
                return .skipped(
                    reason:
                        "AASA CDN returned unparseable JSON or missing "
                        + "webcredentials.apps for \(rpId)"
                )
            }

            if apps.contains(fullAppId) {
                return .associated
            } else {
                return .notAssociated(
                    source: "Apple app-site-association CDN",
                    reason:
                        "Bundle ID \(fullAppId) not in webcredentials.apps "
                        + "for \(rpId). CDN listed: [\(apps.joined(separator: ", "))]"
                )
            }
        } catch {
            return .skipped(
                reason: "AASA CDN fetch failed: \(error.localizedDescription)"
            )
        }
    }

    // MARK: Private helpers

    /// Build an assertion request with rpId, challenge, allow-credentials,
    /// and the caller-supplied PRF setup. Shared by single- and dual-salt.
    private func makeAssertionRequest(
        rpId: String,
        explicitAllowCredentials: [Data],
        configurePrf: (ASAuthorizationPlatformPublicKeyCredentialAssertionRequest) -> Void
    ) -> ASAuthorizationPlatformPublicKeyCredentialAssertionRequest {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let request = provider.createCredentialAssertionRequest(challenge: Self.randomBytes(count: 32))
        applyAllowedCredentials(
            to: request,
            explicitAllowCredentials: explicitAllowCredentials
        )
        configurePrf(request)
        return request
    }

    /// Pin the assertion to caller-supplied credential IDs. With this
    /// set + `preferImmediatelyAvailableCredentials`, iOS auto-routes
    /// to a single matching credential. Empty means fully discoverable.
    private func applyAllowedCredentials(
        to request: ASAuthorizationPlatformPublicKeyCredentialAssertionRequest,
        explicitAllowCredentials: [Data]
    ) {
        if !explicitAllowCredentials.isEmpty {
            request.allowedCredentials = explicitAllowCredentials.map {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
            }
        }
    }

    /// Run one assertion ceremony for `salt1` (+ optional `salt2`) and
    /// return `(first, second?)`. `second` is nil when only one salt
    /// was requested or the authenticator dropped `saltInput2`. The
    /// ObjC helper treats a nil `salt2` as a single-salt evaluation, so
    /// this is the only assertion entry point for both cases.
    private func assertPrf(
        salt1: Data,
        salt2: Data?,
        allowCredentials: [Data],
        preferImmediatelyAvailableCredentials: Bool = true
    ) async throws -> (Data, Data?, Data) {
        let request = makeAssertionRequest(
            rpId: rpId,
            explicitAllowCredentials: allowCredentials
        ) { req in
            // PRF types are NS_REFINED_FOR_SWIFT with no accessible Swift
            // initializers; the ObjC helper sets them via runtime KVC.
            PasskeyPRFHelper.setAssertionPRFOn(req, withSalt1: salt1, salt2: salt2)
        }

        let delegate = PasskeyDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate
        delegate.anchor = anchor.presentationAnchor()

        let preferImmediate = preferImmediatelyAvailableCredentials
        let result = try await withCheckedThrowingContinuation { continuation in
            delegate.assertionContinuation = continuation
            delegate.extractPrf = true
            delegate.ceremonyStartedAt = Date()
            DispatchQueue.main.async {
                Self.performAssertionRequest(
                    controller,
                    preferImmediatelyAvailableCredentials: preferImmediate
                )
            }
        }

        // Best-effort registry seed with the asserted credential ID, so a
        // returning user's pre-tracking credential is captured on first
        // assertion and subsequent registrations hit the platform-level
        // "already exists" guard via `excludeCredentials`. Detached +
        // timeout-bounded so it neither blocks the seed return nor
        // inherits ceremony cancellation. The credential ID itself is
        // returned inline (third tuple element), so there is no separate
        // delegate callback.
        if let reg = credentialRegistry {
            let rpId = self.rpId
            let onRegistryError = self.onRegistryError
            let credentialId = result.2
            Task.detached {
                await registryAddBestEffort(
                    registry: reg,
                    rpId: rpId,
                    credentialId: credentialId,
                    onRegistryError: onRegistryError
                )
            }
        }
        return result
    }

    /// Wraps `controller.performRequests` for assertion paths and
    /// suppresses the OS's hybrid (cross-device QR) sign-in option.
    /// Wallet-style integrators target only local credentials, so a
    /// fast `.canceled` failure is preferable to a confusing QR sheet
    /// when no passkey is on the device.
    private static func performAssertionRequest(
        _ controller: ASAuthorizationController,
        preferImmediatelyAvailableCredentials: Bool = true
    ) {
        if #available(iOS 16.0, macOS 13.0, *), preferImmediatelyAvailableCredentials {
            controller.performRequests(options: .preferImmediatelyAvailableCredentials)
        } else {
            controller.performRequests()
        }
    }

    /// Register a new passkey with PRF support (one platform prompt, no
    /// seed derivation). The configured registry's stored IDs are
    /// auto-merged into [excludeCredentials] so the platform refuses to
    /// create a duplicate even after a reinstall; the new credential ID
    /// is auto-added on success, and the post-create grace tracker is
    /// armed. `userId` is never host-supplied: the core mints a fresh
    /// random 16-byte value and surfaces it via the returned
    /// `IosRegisteredCredential.userId`.
    @discardableResult
    public func register(
        excludeCredentials: [Data] = []
    ) async throws -> IosRegisteredCredential {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = Self.randomBytes(count: 32)
        let resolvedUserId = Self.randomBytes(count: 16)
        // `ASAuthorizationPlatformPublicKeyCredentialProvider` only
        // exposes a 3-arg overload (challenge:name:userID:) on the
        // current iOS / macOS SDKs; the 4-arg displayName overload is
        // security-key-only. Password managers (Apple Passwords, GPM)
        // surface `user.name` as the primary label here, so we pass
        // userDisplayName when callers want that label set, otherwise
        // we fall back to userName.
        _ = userDisplayName // accepted for parity with caller signatures; not consumed by the platform overload
        let request = provider.createCredentialRegistrationRequest(
            challenge: challenge,
            name: userName,
            userID: resolvedUserId
        )

        // Auto-merge previously-registered credential IDs from the
        // opt-in registry so the platform refuses to create a duplicate
        // even after a reinstall (when the registry survives).
        var allExclusions = excludeCredentials
        var seen = Set(excludeCredentials)
        if let reg = credentialRegistry {
            let known = await registryReadBestEffort(
                registry: reg, rpId: rpId, onRegistryError: onRegistryError
            )
            for id in known where seen.insert(id).inserted {
                allExclusions.append(id)
            }
        }
        if !allExclusions.isEmpty {
            request.excludedCredentials = allExclusions.map {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
            }
        }

        // Request PRF support during registration via ObjC helper
        PasskeyPRFHelper.setRegistrationPRFOn(request)

        let delegate = PasskeyDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate
        delegate.anchor = anchor.presentationAnchor()
        // Hand the freshly-minted user handle to the delegate so it can
        // attach it to the returned `IosRegisteredCredential`. The
        // platform never echoes `user.id` back through the credential
        // object, so we surface the value we chose ourselves.
        delegate.registrationUserId = resolvedUserId

        let credential: IosRegisteredCredential = try await withCheckedThrowingContinuation { continuation in
            delegate.registrationContinuation = continuation
            delegate.extractPrf = false
            delegate.ceremonyStartedAt = Date()
            DispatchQueue.main.async {
                controller.performRequests()
            }
        }

        // Record the new credential so subsequent registrations on
        // this device auto-exclude it. Best-effort, fire-and-forget.
        // `detached` so the write isn't cancelled when this function
        // returns and doesn't inherit caller actor isolation; fields are
        // copied to locals so the task captures values, not `self`.
        if let reg = credentialRegistry {
            let credentialId = credential.credentialId
            let rpId = self.rpId
            let onRegistryError = self.onRegistryError
            Task.detached {
                await registryAddBestEffort(
                    registry: reg,
                    rpId: rpId,
                    credentialId: credentialId,
                    onRegistryError: onRegistryError
                )
            }
        }
        // Arm the post-create grace so the SDK's immediate
        // setup_wallet derive doesn't race the credential's
        // PRF-readiness window. Without this, on Apple Passwords
        // the dual-salt assertion drops `prf.second` and we fall
        // back to a second prompt.
        await graceTracker.arm(after: postCreateGraceTotal)
        return credential
    }

    public static func randomBytes(count: Int) -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        _ = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        return Data(bytes)
    }

    /// Whether a `.credentialNotFound` should carry the registry help
    /// suffix when the binding layer maps it to its own error type:
    /// the host had no allow-list and no registry, so the SDK had no
    /// way to populate `allowCredentials` itself.
    public static func shouldAugmentCredentialNotFound(
        explicitAllowCredentials: [Data],
        credentialRegistry: CredentialRegistry?
    ) -> Bool {
        explicitAllowCredentials.isEmpty && credentialRegistry == nil
    }
}

/// Module-level wrapper used inside the core. Returns
/// `.credentialNotFound(message)` with an augmented help suffix when
/// the host had no allow-list and no registry, plain message
/// otherwise. The String payload travels untouched through the
/// binding-layer mappers so every host surface (UniFFI Swift,
/// React Native, Flutter) shows the same diagnostic without
/// per-binding string manipulation.
@available(iOS 18.0, macOS 15.0, *)
fileprivate func augmentCredentialNotFound(
    explicitAllowCredentials: [Data],
    credentialRegistry: CredentialRegistry?
) -> PasskeyAssertionError {
    let base = "No matching credential on this device"
    let augment = PasskeyAssertionCore.shouldAugmentCredentialNotFound(
        explicitAllowCredentials: explicitAllowCredentials,
        credentialRegistry: credentialRegistry
    )
    return .credentialNotFound(augment ? base + credentialRegistryHelpSuffix : base)
}

/// Suffix appended to `CredentialNotFound` errors when the host had no
/// `allowCredentials` and no `CredentialRegistry`, pointing at the
/// docs section that explains the opt-in auto-discovery path.
@available(iOS 18.0, macOS 15.0, *)
public let credentialRegistryHelpSuffix: String =
    " (No CredentialRegistry was supplied to PasskeyProvider; "
    + "if you expect the SDK to auto-discover known credentials, see "
    + "https://sdk-doc-spark.breez.technology/guide/passkey.html#credentialregistry)"

// MARK: - Registered credential metadata

/// Extract AAGUID + BE flag from the attestation object's authenticator
/// data via byte-pattern search for the "authData" CBOR key. Returns nil
/// when the pattern isn't found or the byte string is too short.
///
/// authData layout when AT flag is set (always on a successful create):
///   [32]      flags (UP=0, UV=2, BE=3, BS=4, AT=6)
///   [37..53)  AAGUID (16 bytes)
@available(iOS 18.0, macOS 15.0, *)
public func extractRegistrationMetadata(from attestation: Data) -> (aaguid: Data, backupEligible: Bool)? {
    let bytes = [UInt8](attestation)
    // CBOR text key "authData": 0x68 = major type 3 (text) + length 8.
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

    // Parse CBOR byte string (major type 2) at keyEnd.
    let header = bytes[keyEnd]
    guard header >> 5 == 2 else { return nil }
    let minor = Int(header & 0x1f)
    let length: Int
    let dataStart: Int
    switch minor {
    case 0..<24:
        length = minor
        dataStart = keyEnd + 1
    case 24:
        guard keyEnd + 1 < bytes.count else { return nil }
        length = Int(bytes[keyEnd + 1])
        dataStart = keyEnd + 2
    case 25:
        guard keyEnd + 2 < bytes.count else { return nil }
        length = (Int(bytes[keyEnd + 1]) << 8) | Int(bytes[keyEnd + 2])
        dataStart = keyEnd + 3
    case 26:
        guard keyEnd + 4 < bytes.count else { return nil }
        length = (Int(bytes[keyEnd + 1]) << 24) | (Int(bytes[keyEnd + 2]) << 16)
            | (Int(bytes[keyEnd + 3]) << 8) | Int(bytes[keyEnd + 4])
        dataStart = keyEnd + 5
    default:
        return nil
    }
    guard dataStart + length <= bytes.count, length >= 53 else { return nil }
    let flags = bytes[dataStart + 32]
    guard flags & 0x40 != 0 else { return nil }
    let backupEligible = flags & 0x08 != 0
    let aaguid = Data(bytes[(dataStart + 37)..<(dataStart + 53)])
    return (aaguid: aaguid, backupEligible: backupEligible)
}

// MARK: - Authorization Delegate

/// Delegate handling both the assertion and registration ceremonies,
/// selected at the call site by which continuation is set
/// (`assertionContinuation` or `registrationContinuation`).
@available(iOS 18.0, macOS 15.0, *)
private final class PasskeyDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    /// Resolves `(first, second?, credentialId)`; `second` is nil for a
    /// single-salt assertion or when the authenticator dropped
    /// `saltInput2`. `credentialId` is the asserted credential, carried
    /// inline so the core can return it without a separate slot.
    var assertionContinuation: CheckedContinuation<(Data, Data?, Data), Error>?
    var registrationContinuation: CheckedContinuation<IosRegisteredCredential, Error>?
    /// User handle the core chose for the in-flight registration. The
    /// platform never echoes `user.id` back through the credential
    /// object, so the delegate copies this into the returned
    /// `IosRegisteredCredential.userId` field.
    var registrationUserId: Data = Data()
    var anchor: ASPresentationAnchor = ASPresentationAnchor()
    /// `true` for assertion ceremonies, `false` for registration.
    var extractPrf = true
    /// Wall-clock timestamp at the moment `controller.performRequests()`
    /// fires. Used by `mapPasskeyError` to discriminate the OS biometric
    /// inactivity timeout (~55s+, surfaced as `.canceled`) from a
    /// user-dismissed prompt. Set on every ceremony before the controller
    /// is started.
    var ceremonyStartedAt: Date?

    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        return anchor
    }

    func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        if extractPrf {
            guard let credential = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialAssertion
            else {
                assertionContinuation?.resume(
                    throwing: PasskeyAssertionError.authenticationFailed("Unexpected credential type"))
                return
            }

            guard let prfFirst = PasskeyPRFHelper.extractPRFOutput(from: credential) else {
                assertionContinuation?.resume(throwing: PasskeyAssertionError.prfNotSupported)
                return
            }

            // Resolve with the seeds plus the asserted credential ID.
            // `second` is nil for single-salt ceremonies or when the
            // authenticator dropped saltInput2. The caller seeds the
            // registry from the returned credential ID; no separate
            // callback is needed.
            assertionContinuation?.resume(
                returning: (
                    prfFirst,
                    PasskeyPRFHelper.extractSecondPRFOutput(from: credential),
                    credential.credentialID
                ))
        } else {
            guard let credential = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialRegistration
            else {
                registrationContinuation?.resume(
                    throwing: PasskeyAssertionError.authenticationFailed("Unexpected credential type"))
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
                returning: IosRegisteredCredential(
                    credentialId: credential.credentialID,
                    userId: registrationUserId,
                    aaguid: aaguid,
                    backupEligible: backupEligible
                ))
        }
    }

    func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        let elapsedMs: Double? = ceremonyStartedAt
            .map { Date().timeIntervalSince($0) * 1000.0 }
        let mapped = mapPasskeyError(error, elapsedMs: elapsedMs)
        assertionContinuation?.resume(throwing: mapped)
        registrationContinuation?.resume(throwing: mapped)
    }
}

/// Map an `ASAuthorizationError` to `PasskeyAssertionError`.
///
/// `.canceled` covers three distinct cases that the OS collapses into
/// the same error code (`preferImmediatelyAvailableCredentials`
/// suppresses the QR / hybrid sheet, so there is no in-process signal
/// to disambiguate):
///
///   1. No matching credential available: fast-fail before any UI is
///      shown. Resolves in well under 300ms.
///   2. User dismissed the visible prompt: anywhere from a fraction of
///      a second up to a few tens of seconds, depending on user.
///   3. OS biometric inactivity timeout: the prompt was up but the user
///      neither approved nor dismissed. iOS tears the sheet down at
///      ~55s and reports the same `.canceled`.
///
/// The wall-clock time between starting the ceremony and receiving the
/// error tells (1) from (2 / 3), and (2) from (3). Thresholds:
///   - `< 300ms`         → `.credentialNotFound`
///   - `>= 55_000ms`     → `.userTimedOut`
///   - in between        → `.userCancelled`
///
/// Hosts can branch on the typed variant instead of timing the call
/// themselves.
@available(iOS 18.0, macOS 15.0, *)
public func mapPasskeyError(
    _ error: Error,
    elapsedMs: Double? = nil
) -> PasskeyAssertionError {
    let nsError = error as NSError
    if nsError.domain == ASAuthorizationError.errorDomain {
        switch ASAuthorizationError.Code(rawValue: nsError.code) {
        case .canceled:
            return classifyCanceled(elapsedMs: elapsedMs)
        case .unknown:
            if nsError.localizedDescription.contains("no credential")
                || nsError.localizedDescription.contains("No credentials")
            {
                return .credentialNotFound(nsError.localizedDescription)
            }
            return .authenticationFailed(nsError.localizedDescription)
        case .invalidResponse:
            return .prfEvaluationFailed(nsError.localizedDescription)
        case .notHandled:
            return .credentialNotFound(nsError.localizedDescription)
        case .failed:
            return .authenticationFailed(nsError.localizedDescription)
        case .notInteractive:
            return .authenticationFailed("User interaction required")
        case .matchedExcludedCredential:
            return .credentialAlreadyExists("Credential already registered")
        default:
            return .generic(nsError.localizedDescription)
        }
    }
    return .generic(error.localizedDescription)
}

/// Wall-clock thresholds used to discriminate the three flavors of
/// `ASAuthorizationError.canceled`. See `mapPasskeyError` for the
/// rationale behind each cutoff.
@available(iOS 18.0, macOS 15.0, *)
private func classifyCanceled(elapsedMs: Double?) -> PasskeyAssertionError {
    guard let elapsed = elapsedMs else {
        // No timing context: preserve the historical mapping.
        return .userCancelled
    }
    if elapsed < 300 {
        return .credentialNotFound("Credential not found")
    }
    if elapsed >= 55_000 {
        return .userTimedOut
    }
    return .userCancelled
}
