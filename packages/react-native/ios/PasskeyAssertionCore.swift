import AuthenticationServices
import Foundation
import Security
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

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
    case credentialNotFound
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
@available(iOS 18.0, macOS 15.0, *)
public struct IosRegisteredCredential {
    public let credentialId: Data
    public let aaguid: Data?
    public let backupEligible: Bool?

    public init(credentialId: Data, aaguid: Data?, backupEligible: Bool?) {
        self.credentialId = credentialId
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
/// the WebAuthn ceremony — the SDK never blocks on it.
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
/// - single-salt assertion (`assertionWithPrf`)
/// - dual-salt assertion (`dualSaltAssertion`)
/// - bulk derivation walking salts in pairs (`performBulkDerivation`)
/// - registration with iOS 17.4+ displayName overload (`registerCredential`)
/// - KnownCredentialsStore auto-merge for `excludeCredentialIds` and
///   `allowedCredentials`
/// - `preferImmediatelyAvailableCredentials` for fast-fail on no-cred
/// - `matchedExcludedCredential` -> `credentialAlreadyExists`
/// - AAGUID + BE flag extraction from attestation CBOR
/// - 800ms post-create grace tracker
@available(iOS 18.0, macOS 15.0, *)
public final class PasskeyAssertionCore {
    private let anchor: PasskeyPresentationAnchorProvider
    private let graceTracker: PostCreateGraceTracker
    private let postCreateGraceTotal: TimeInterval

    /// Optional callback fired with the credential ID returned by every
    /// successful WebAuthn assertion (sign-in path). Hosts can set this
    /// to record which credential was just used so they can populate
    /// `excludeCredentialIds` and `allowedCredentialIds` on subsequent
    /// requests.
    public var onAssertionCredentialId: ((Data) -> Void)?

    public init(
        anchorProvider: PasskeyPresentationAnchorProvider? = nil,
        graceTracker: PostCreateGraceTracker = PostCreateGraceTracker(),
        postCreateGraceTotal: TimeInterval = PostCreateGraceTracker.defaultTotal
    ) {
        self.anchor = anchorProvider ?? DefaultPasskeyPresentationAnchorProvider()
        self.graceTracker = graceTracker
        self.postCreateGraceTotal = postCreateGraceTotal
    }

    // MARK: Public entry points

    /// Single-salt PRF derivation with auto-register fallback. Used
    /// internally by `performBulkDerivation` for the `salts.count == 1`
    /// case. Public callers always go through `performBulkDerivation`;
    /// the bulk path short-circuits to this helper for single-element
    /// inputs so a single-salt derive still costs one prompt.
    private func performDerivation(
        saltData: Data,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        autoRegister: Bool,
        explicitAllowCredentialIds: [Data] = []
    ) async throws -> Data {
        await graceTracker.consume()
        do {
            return try await assertionWithPrf(
                saltData: saltData,
                rpId: rpId,
                explicitAllowCredentialIds: explicitAllowCredentialIds
            )
        } catch PasskeyAssertionError.credentialNotFound {
            guard autoRegister else { throw PasskeyAssertionError.credentialNotFound }
            do {
                _ = try await registerCredential(
                    rpId: rpId,
                    rpName: rpName,
                    userName: userName,
                    userDisplayName: userDisplayName
                )
            } catch PasskeyAssertionError.credentialNotFound {
                throw PasskeyAssertionError.configuration(
                    "Associated Domains entitlement not configured. "
                    + "Add 'webcredentials:\(rpId)' to your app's entitlements "
                    + "and ensure a valid provisioning profile."
                )
            }
            return try await assertionWithPrf(
                saltData: saltData,
                rpId: rpId,
                explicitAllowCredentialIds: explicitAllowCredentialIds
            )
        }
    }

    /// Bulk PRF derivation. Walks `salts` two-at-a-time, attempting a
    /// dual-salt single-ceremony assertion per pair; if the authenticator
    /// drops `saltInput2` we fall back to a single-salt re-assert for that
    /// element. Odd-count tail is a single-salt assertion. Mirrors the
    /// upstream Swift `dualSaltAssertion` flow used by glow-app.
    public func performBulkDerivation(
        salts: [Data],
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        autoRegister: Bool,
        explicitAllowCredentialIds: [Data] = []
    ) async throws -> [Data] {
        // Wait out post-create grace before any assertion in the batch.
        // Without this, the immediate setup_wallet derive after register
        // races the credential's PRF-readiness window: dual-salt comes
        // back with `prf.first` set and `prf.second == nil`, forcing a
        // second single-salt prompt.
        await graceTracker.consume()
        if salts.isEmpty {
            return []
        }
        if salts.count == 1 {
            return [try await performDerivation(
                saltData: salts[0],
                rpId: rpId,
                rpName: rpName,
                userName: userName,
                userDisplayName: userDisplayName,
                autoRegister: autoRegister,
                explicitAllowCredentialIds: explicitAllowCredentialIds
            )]
        }
        var out: [Data] = []
        var i = 0
        while i < salts.count {
            let salt1 = salts[i]
            let salt2: Data? = (i + 1 < salts.count) ? salts[i + 1] : nil
            do {
                let pair = try await dualSaltAssertion(
                    salt1: salt1,
                    salt2: salt2,
                    rpId: rpId,
                    explicitAllowCredentialIds: explicitAllowCredentialIds
                )
                out.append(pair.0)
                if let second = pair.1 {
                    out.append(second)
                    i += 2
                } else if let salt2 = salt2 {
                    // Authenticator dropped saltInput2: fall back to a
                    // separate single-salt assertion for the second salt.
                    let recovered = try await assertionWithPrf(
                        saltData: salt2,
                        rpId: rpId,
                        explicitAllowCredentialIds: explicitAllowCredentialIds
                    )
                    out.append(recovered)
                    i += 2
                } else {
                    i += 1
                }
            } catch PasskeyAssertionError.credentialNotFound {
                // No credential on the device. Single-salt path can fall
                // back to register; the bulk path defers to single-salt.
                guard autoRegister else { throw PasskeyAssertionError.credentialNotFound }
                _ = try await registerCredential(
                    rpId: rpId,
                    rpName: rpName,
                    userName: userName,
                    userDisplayName: userDisplayName
                )
                // Retry the same pair after registration.
                continue
            }
        }
        return out
    }

    /// Create a new passkey with PRF support. Auto-merges previously-
    /// registered credential IDs from `KnownCredentialsStore` into the
    /// final exclude list so the platform refuses to create a duplicate
    /// even after a reinstall (the store is iCloud-synced). Records the
    /// new credential ID after a successful create and arms the
    /// post-create grace tracker.
    @discardableResult
    public func createPasskey(
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        excludeCredentialIds: [Data] = [],
        userId: Data? = nil
    ) async throws -> IosRegisteredCredential {
        let credential = try await registerCredential(
            rpId: rpId,
            rpName: rpName,
            userName: userName,
            userDisplayName: userDisplayName,
            excludeCredentialIds: excludeCredentialIds,
            userId: userId
        )
        return credential
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
    public func checkDomainAssociation(
        rpId: String,
        explicitTeamId: String? = nil,
        urlSession: URLSession = .shared
    ) async -> IosDomainAssociation {
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
        explicitAllowCredentialIds: [Data],
        configurePrf: (ASAuthorizationPlatformPublicKeyCredentialAssertionRequest) -> Void
    ) -> ASAuthorizationPlatformPublicKeyCredentialAssertionRequest {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let request = provider.createCredentialAssertionRequest(challenge: Self.randomBytes(count: 32))
        applyAllowedCredentials(
            to: request,
            rpId: rpId,
            explicitAllowCredentialIds: explicitAllowCredentialIds
        )
        configurePrf(request)
        return request
    }

    /// Pin the assertion to credentials we've registered for this rpId
    /// (read from the iCloud-synced KnownCredentialsStore). With this
    /// set + `preferImmediatelyAvailableCredentials`, iOS auto-routes
    /// to a single matching credential. No "select your passkey"
    /// picker between register and the post-register PRF assertion.
    /// Empty / no known creds means fully discoverable (initial sign-in
    /// before the user has registered anything on this device).
    private func applyAllowedCredentials(
        to request: ASAuthorizationPlatformPublicKeyCredentialAssertionRequest,
        rpId: String,
        explicitAllowCredentialIds: [Data]
    ) {
        // Caller-supplied IDs win when present (upstream PrfProvider
        // exposes this via constructor for tests / explicit pinning).
        if !explicitAllowCredentialIds.isEmpty {
            request.allowedCredentials = explicitAllowCredentialIds.map {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
            }
            return
        }
        let known: [Data] = KnownCredentialsStore.read(rpId: rpId).compactMap {
            Data(base64Encoded: $0)
        }
        if !known.isEmpty {
            request.allowedCredentials = known.map {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
            }
        }
    }

    private func assertionWithPrf(
        saltData: Data,
        rpId: String,
        explicitAllowCredentialIds: [Data]
    ) async throws -> Data {
        let request = makeAssertionRequest(
            rpId: rpId,
            explicitAllowCredentialIds: explicitAllowCredentialIds
        ) { req in
            // PRF types are NS_REFINED_FOR_SWIFT with no accessible Swift
            // initializers; the ObjC helper sets them via runtime KVC.
            PasskeyPRFHelper.setAssertionPRFOn(req, withSalt: saltData)
        }

        let delegate = PasskeyDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate
        delegate.anchor = anchor.presentationAnchor()
        delegate.onAssertionCredentialId = makeCaptureCallback(rpId: rpId)

        return try await withCheckedThrowingContinuation { continuation in
            delegate.continuation = continuation
            delegate.extractPrf = true
            delegate.ceremonyStartedAt = Date()
            DispatchQueue.main.async {
                Self.performAssertionRequest(controller)
            }
        }
    }

    private func dualSaltAssertion(
        salt1: Data,
        salt2: Data?,
        rpId: String,
        explicitAllowCredentialIds: [Data]
    ) async throws -> (Data, Data?) {
        let request = makeAssertionRequest(
            rpId: rpId,
            explicitAllowCredentialIds: explicitAllowCredentialIds
        ) { req in
            PasskeyPRFHelper.setAssertionPRFOn(req, withSalt1: salt1, salt2: salt2)
        }

        let delegate = PasskeyDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate
        delegate.anchor = anchor.presentationAnchor()
        delegate.onAssertionCredentialId = makeCaptureCallback(rpId: rpId)

        return try await withCheckedThrowingContinuation { continuation in
            delegate.dualSaltContinuation = continuation
            delegate.extractPrf = true
            delegate.extractSecondPrf = true
            delegate.ceremonyStartedAt = Date()
            DispatchQueue.main.async {
                Self.performAssertionRequest(controller)
            }
        }
    }

    /// Build the per-assertion credential-ID callback. Wraps two
    /// concerns into one closure passed to the delegate:
    ///
    /// 1. Capture-on-sign-in: every successful assertion auto-adds the
    ///    credential ID to the iCloud-synced `KnownCredentialsStore`.
    ///    `add` is idempotent. This migrates users whose passkey
    ///    predates our tracking — first assertion seeds the store, so
    ///    subsequent registration attempts correctly hit the
    ///    platform-level "already exists" guard via `excludedCredentials`.
    ///    Without this, the store stays empty until a fresh `createPasskey`
    ///    runs, and a returning user with a pre-tracking credential
    ///    can accidentally register a duplicate.
    /// 2. Host opt-in: forwards to the public `onAssertionCredentialId`
    ///    callback for any host-side bookkeeping (per-cred metadata,
    ///    last-seen timestamps, etc.). Always invoked, regardless of
    ///    whether the cred was already in the store.
    ///
    /// Capture happens BEFORE the host callback fires so a host that
    /// reads the store inside its callback observes the just-seen cred.
    private func makeCaptureCallback(rpId: String) -> (Data) -> Void {
        let captured = onAssertionCredentialId
        return { credentialId in
            KnownCredentialsStore.add(
                credentialId: credentialId.base64EncodedString(),
                rpId: rpId
            )
            captured?(credentialId)
        }
    }

    /// Wraps `controller.performRequests` for assertion paths and
    /// suppresses the OS's hybrid (cross-device QR) sign-in option.
    /// Wallet-style integrators target only local credentials, so a
    /// fast `.canceled` failure is preferable to a confusing QR sheet
    /// when no passkey is on the device.
    private static func performAssertionRequest(_ controller: ASAuthorizationController) {
        if #available(iOS 16.0, macOS 13.0, *) {
            controller.performRequests(options: .preferImmediatelyAvailableCredentials)
        } else {
            controller.performRequests()
        }
    }

    @discardableResult
    private func registerCredential(
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        excludeCredentialIds: [Data] = [],
        userId: Data? = nil
    ) async throws -> IosRegisteredCredential {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = Self.randomBytes(count: 32)
        let resolvedUserId = userId ?? Self.randomBytes(count: 16)
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
        // iCloud-synced KnownCredentialsStore so the platform refuses
        // to create a duplicate even after a reinstall.
        var allExclusions = excludeCredentialIds
        var seen = Set(excludeCredentialIds)
        for known in KnownCredentialsStore.read(rpId: rpId).compactMap({ Data(base64Encoded: $0) }) {
            if seen.insert(known).inserted {
                allExclusions.append(known)
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

        let credential: IosRegisteredCredential = try await withCheckedThrowingContinuation { continuation in
            delegate.registrationContinuation = continuation
            delegate.extractPrf = false
            delegate.ceremonyStartedAt = Date()
            DispatchQueue.main.async {
                controller.performRequests()
            }
        }

        // Record the new credential so subsequent registrations on
        // this device (or a reinstall) auto-exclude it.
        KnownCredentialsStore.add(
            credentialId: credential.credentialId.base64EncodedString(),
            rpId: rpId
        )
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
}

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

/// Unified delegate handling both single-salt and dual-salt assertion
/// paths plus registration. Selected at the call site by which
/// continuation is set (`continuation`, `dualSaltContinuation`, or
/// `registrationContinuation`).
@available(iOS 18.0, macOS 15.0, *)
private final class PasskeyDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    var continuation: CheckedContinuation<Data, Error>?
    var dualSaltContinuation: CheckedContinuation<(Data, Data?), Error>?
    var registrationContinuation: CheckedContinuation<IosRegisteredCredential, Error>?
    var anchor: ASPresentationAnchor = ASPresentationAnchor()
    var extractPrf = true
    var extractSecondPrf = false
    /// Invoked with the credential ID from a successful assertion. Set
    /// by the core so hosts can record which credential was used. No-op
    /// on registration (the credential ID flows out via the
    /// registrationContinuation).
    var onAssertionCredentialId: ((Data) -> Void)?
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
                let err = PasskeyAssertionError.authenticationFailed("Unexpected credential type")
                continuation?.resume(throwing: err)
                dualSaltContinuation?.resume(throwing: err)
                return
            }

            guard let prfFirst = PasskeyPRFHelper.extractPRFOutput(from: credential) else {
                let err = PasskeyAssertionError.prfNotSupported
                continuation?.resume(throwing: err)
                dualSaltContinuation?.resume(throwing: err)
                return
            }

            // Surface the credential ID before resolving so hosts can
            // record it. Failures here are best-effort and must not
            // block the seed return.
            onAssertionCredentialId?(credential.credentialID)

            if let dualCont = dualSaltContinuation {
                let prfSecond = extractSecondPrf
                    ? PasskeyPRFHelper.extractSecondPRFOutput(from: credential)
                    : nil
                dualCont.resume(returning: (prfFirst, prfSecond))
                return
            }

            continuation?.resume(returning: prfFirst)
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
        continuation?.resume(throwing: mapped)
        dualSaltContinuation?.resume(throwing: mapped)
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
                return .credentialNotFound
            }
            return .authenticationFailed(nsError.localizedDescription)
        case .invalidResponse:
            return .prfEvaluationFailed(nsError.localizedDescription)
        case .notHandled:
            return .credentialNotFound
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
        return .credentialNotFound
    }
    if elapsed >= 55_000 {
        return .userTimedOut
    }
    return .userCancelled
}
