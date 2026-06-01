import AuthenticationServices
import Foundation
import PasskeyPRFHelperObjC
import Security
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// Canonical iOS/macOS passkey PRF logic. The upstream Swift
/// `PrfProvider`, the Flutter MethodChannel plugin, and the React Native
/// bridge each wrap one `PasskeyAssertionCore` and translate
/// `PasskeyAssertionError` to their own error type.
///
/// Mirrors Android's `CredentialManagerPrfCore.kt`. Synced via
/// `cargo xtask sync-passkey-core`.

// MARK: - Post-create grace

/// A newly-registered passkey is briefly not ready for the immediate
/// post-create assertion: Apple Passwords drops `prf.second` from a
/// dual-salt assertion (forcing a second single-salt prompt), and GPM
/// hides the credential from the picker entirely. Holding the next
/// derive up to 800ms lets the OS finish indexing.
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

/// Layer-neutral error surface. Wrappers translate to their own typed
/// errors (UniFFI `PrfProviderError`, `FlutterError`, RCT reject codes).
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

// MARK: - Passkey credential

/// A passkey credential from a register or sign-in ceremony. Named `Ios*`
/// to avoid colliding with the UniFFI-generated `PasskeyCredential` the
/// Swift wrapper translates to.
///
/// `credentialId` is always set. The attestation fields (`userId`,
/// `aaguid`, `backupEligible`) are populated on registration and `nil` on
/// sign-in, where an assertion carries no attestation. `userId` is the
/// core-minted WebAuthn user handle, never host-supplied.
@available(iOS 18.0, macOS 15.0, *)
public struct IosPasskeyCredential {
    public let credentialId: Data
    public let userId: Data?
    public let aaguid: Data?
    public let backupEligible: Bool?

    public init(credentialId: Data, userId: Data?, aaguid: Data?, backupEligible: Bool?) {
        self.credentialId = credentialId
        self.userId = userId
        self.aaguid = aaguid
        self.backupEligible = backupEligible
    }
}

/// Result of `deriveSeeds`: one 32-byte PRF output per salt (input
/// order) plus the asserted credential ID. `credentialId` is `nil` when
/// no assertion ran (empty `salts`).
public struct PrfDerivation {
    public let seeds: [Data]
    public let credentialId: Data?

    public init(seeds: [Data], credentialId: Data?) {
        self.seeds = seeds
        self.credentialId = credentialId
    }
}

// MARK: - Domain association

/// Result of an Apple-app-site-association probe against the AASA CDN.
/// Layer-neutral; wrappers translate to their own representation.
///
/// `Skipped` is the catch-all for verification-level failures (missing
/// team/bundle ID, network error, malformed JSON). It is advisory: the
/// SDK never blocks the WebAuthn ceremony on it.
@available(iOS 18.0, macOS 15.0, *)
public enum IosDomainAssociation {
    case associated
    case notAssociated(source: String, reason: String)
    case skipped(reason: String)
}

// MARK: - Team ID detection

/// Auto-detect the 10-character Apple Developer Team ID from the running
/// app's signing info, by platform:
///
/// - **macOS**: the `application-identifier` entitlement
///   (`<TEAM_ID>.<BUNDLE_ID>`, split on the first dot) via
///   `SecTaskCopyValueForEntitlement`.
/// - **iOS**: `embedded.mobileprovision`, a PKCS#7-wrapped plist. The
///   plist bytes are plain-text inside the CMS envelope, so locate the
///   `<?xml>...</plist>` span and deserialize `TeamIdentifier`.
///
/// Returns nil on simulator / unsigned builds (no provisioning profile),
/// where `checkDomainAssociation` then reports `.skipped`. Cached at
/// first read: the team ID is stable for the binary's lifetime.
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
        guard let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision"),
              let data = try? Data(contentsOf: url)
        else { return nil }
        // `.isoLatin1` (not `.ascii`) because the PKCS#7 envelope has
        // binary DER bytes > 127; it maps 1:1 and always succeeds. The
        // string only locates the plist span; parsing uses the raw slice.
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

/// Reusable WebAuthn PRF logic; holds no per-request state. Owns all
/// ASAuthorizationController orchestration (assertion, bulk derivation,
/// registration) plus the post-create grace tracker.
@available(iOS 18.0, macOS 15.0, *)
public final class PasskeyAssertionCore {
    private let rpId: String
    private let rpName: String
    private let userName: String
    private let userDisplayName: String
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
    ) async throws -> PrfDerivation {
        var allow = allowCredentials
        // Wait out the post-create grace so the immediate derive doesn't
        // race the credential's PRF-readiness window (see grace tracker).
        await graceTracker.consume()
        if salts.isEmpty { return PrfDerivation(seeds: [], credentialId: nil) }

        // One assertion for 1-2 salts, registering + retrying once on no
        // credential. Returns (first, second?, credentialId); `second` is
        // nil when the authenticator dropped saltInput2. After the first
        // chunk the caller pins `allow` to its credential, so every chunk
        // resolves to the same one.
        func assertChunk(_ salt1: Data, _ salt2: Data?) async throws -> (Data, Data?, Data) {
            do {
                return try await assertPrf(
                    salt1: salt1, salt2: salt2, allowCredentials: allow,
                    preferImmediatelyAvailableCredentials: preferImmediatelyAvailableCredentials
                )
            } catch PasskeyAssertionError.credentialNotFound(_) {
                guard autoRegister else {
                    throw PasskeyAssertionError.credentialNotFound(
                        "No matching credential on this device"
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
                // Retry once. A second miss (e.g. user deleted the pinned
                // credential in Settings) escapes as credentialNotFound for
                // hosts to treat as deletion recovery.
                return try await assertPrf(
                    salt1: salt1, salt2: salt2, allowCredentials: allow,
                    preferImmediatelyAvailableCredentials: preferImmediatelyAvailableCredentials
                )
            }
        }

        var out: [Data] = []
        // Asserted credential ID, returned inline so the binding layer can
        // surface it on `SignInResponse.credential_id` without a separate
        // read-and-clear call.
        var observedCredentialId: Data?
        var i = 0
        while i < salts.count {
            if i + 1 < salts.count {
                let (first, second, credId) = try await assertChunk(salts[i], salts[i + 1])
                observedCredentialId = credId
                // Pin every later assertion in this call to the credential the
                // first one resolved to, so all salts derive from one passkey
                // even when a chunk splits (dropped `second`, or 3+ salts).
                allow = [credId]
                out.append(first)
                if let second = second {
                    out.append(second)
                } else {
                    // Authenticator dropped `second`: single-salt recover,
                    // pinned to the same credential as `first`.
                    let (recovered, _, _) = try await assertChunk(salts[i + 1], nil)
                    out.append(recovered)
                }
                i += 2
            } else {
                let (single, _, credId) = try await assertChunk(salts[i], nil)
                observedCredentialId = credId
                allow = [credId]
                out.append(single)
                i += 1
            }
        }
        return PrfDerivation(seeds: out, credentialId: observedCredentialId)
    }

    /// Verify the app's bundle ID is listed in `webcredentials.apps` of
    /// the AASA file for `rpId`, via Apple's app-site-association CDN
    /// (`https://app-site-association.cdn-apple.com/a/v1/<rpId>`). This is
    /// the same source the OS uses for Associated Domains, queried up
    /// front so integrators see misconfiguration before a WebAuthn
    /// ceremony fails opaquely.
    ///
    /// Team ID comes from `explicitTeamId` or `PasskeyTeamIdDetector`.
    /// Never throws: every verification-level failure (no bundle/team ID,
    /// network error, malformed JSON) maps to `.skipped`.
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

    /// Run one assertion ceremony for `salt1` (+ optional `salt2`),
    /// returning `(first, second?, credentialId)`. `second` is nil for a
    /// single salt or when the authenticator dropped `saltInput2`. The
    /// ObjC helper treats nil `salt2` as single-salt, so this serves both.
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
            DispatchQueue.main.async {
                // Capture start time inside the closure so the wait for
                // the main-thread dispatch doesn't count as ceremony time
                // (a busy main thread can push a sub-300ms fast-fail past
                // the no-credential threshold and misclassify it).
                delegate.ceremonyStartedAt = Date()
                Self.performAssertionRequest(
                    controller,
                    preferImmediatelyAvailableCredentials: preferImmediate
                )
            }
        }

        return result
    }

    /// Wraps `controller.performRequests`, suppressing the hybrid
    /// (cross-device QR) sign-in option. Wallet-style integrators target
    /// only local credentials, so a fast `.canceled` beats a confusing QR
    /// sheet when no passkey is on the device.
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
    /// seed derivation). `excludeCredentials` lists already-registered IDs
    /// the platform must refuse as duplicates. The post-create grace
    /// tracker is armed on success. `userId` is never host-supplied: the
    /// core mints a fresh random 16-byte value and returns it on
    /// `IosPasskeyCredential.userId`.
    @discardableResult
    public func register(
        excludeCredentials: [Data] = []
    ) async throws -> IosPasskeyCredential {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = Self.randomBytes(count: 32)
        let resolvedUserId = Self.randomBytes(count: 16)
        // The platform provider only exposes the 3-arg
        // challenge:name:userID: overload on current SDKs; the 4-arg
        // displayName overload is security-key-only. Password managers
        // (Apple Passwords, GPM) show `user.name` as the primary label.
        _ = userDisplayName // accepted for parity with caller signatures; not consumed by the platform overload
        let request = provider.createCredentialRegistrationRequest(
            challenge: challenge,
            name: userName,
            userID: resolvedUserId
        )

        if !excludeCredentials.isEmpty {
            request.excludedCredentials = excludeCredentials.map {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
            }
        }

        PasskeyPRFHelper.setRegistrationPRFOn(request)

        let delegate = PasskeyDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate
        delegate.anchor = anchor.presentationAnchor()
        // The platform never echoes `user.id` back, so hand the delegate
        // our minted handle to attach to the returned credential.
        delegate.registrationUserId = resolvedUserId

        let credential: IosPasskeyCredential = try await withCheckedThrowingContinuation { continuation in
            delegate.registrationContinuation = continuation
            delegate.extractPrf = false
            DispatchQueue.main.async {
                // Capture start time inside the closure so the wait for
                // the main-thread dispatch doesn't count as ceremony time.
                delegate.ceremonyStartedAt = Date()
                controller.performRequests()
            }
        }

        // Arm the post-create grace so the immediate derive doesn't race
        // the credential's PRF-readiness window (see grace tracker).
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
/// when not found or too short.
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
    /// single salt or a dropped `saltInput2`.
    var assertionContinuation: CheckedContinuation<(Data, Data?, Data), Error>?
    var registrationContinuation: CheckedContinuation<IosPasskeyCredential, Error>?
    /// User handle the core minted for the in-flight registration; copied
    /// into the returned `IosPasskeyCredential.userId` (the platform
    /// never echoes `user.id` back).
    var registrationUserId: Data = Data()
    var anchor: ASPresentationAnchor = ASPresentationAnchor()
    /// `true` for assertion ceremonies, `false` for registration.
    var extractPrf = true
    /// Set when `performRequests()` fires; `mapPasskeyError` uses the
    /// elapsed time to tell the biometric inactivity timeout from a
    /// user-dismissed prompt (both arrive as `.canceled`).
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
                returning: IosPasskeyCredential(
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
/// The OS collapses three distinct `.canceled` cases into one code (the
/// suppressed QR sheet leaves no in-process signal to disambiguate): no
/// matching credential (fast-fail before any UI), user-dismissed prompt,
/// and the biometric inactivity timeout (sheet torn down at ~55s).
/// Elapsed wall-clock time separates them:
///   - `< 300ms`     -> `.credentialNotFound`
///   - `>= 55_000ms` -> `.userTimedOut`
///   - in between    -> `.userCancelled`
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

/// Apply the `.canceled` timing thresholds (see `mapPasskeyError`).
@available(iOS 18.0, macOS 15.0, *)
private func classifyCanceled(elapsedMs: Double?) -> PasskeyAssertionError {
    guard let elapsed = elapsedMs else {
        // No timing context: default to userCancelled.
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
