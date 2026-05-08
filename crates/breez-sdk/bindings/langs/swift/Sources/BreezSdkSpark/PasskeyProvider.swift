import AuthenticationServices
import Foundation
import PasskeyPRFHelperObjC
import Security

/// Built-in passkey-based PRF provider for iOS/macOS using the
/// AuthenticationServices framework.
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
/// let prfProvider = PasskeyProvider()
/// let passkey = Passkey(prfProvider: prfProvider, relayConfig: nil)
/// let wallet = try await passkey.getWallet(walletName: "personal")
/// ```
@available(iOS 18.0, macOS 15.0, *)
public class PasskeyProvider: PrfProvider {
    private let rpId: String
    private let rpName: String
    private let userName: String
    private let userDisplayName: String
    private let autoRegister: Bool
    private let allowCredentialIds: [Data]
    private let explicitTeamId: String?
    private let urlSession: URLSession
    private let core: PasskeyAssertionCore

    /// Optional callback fired with the credential ID returned by every
    /// successful WebAuthn assertion (sign-in path). Hosts can set this
    /// to record which credential was just used so they can populate
    /// `excludeCredentialIds` and `allowCredentialIds` on subsequent
    /// requests.
    ///
    /// Useful for migrating users whose passkey predates the host's
    /// own credential-ID tracking: the first successful sign-in surfaces
    /// the credential ID, after which the host's records are correct
    /// and the platform-level "already exists" check can fire on
    /// future create attempts.
    ///
    /// Must be set before calling `deriveSeed`. Not invoked on
    /// registration (see `createPasskey`'s return value for that).
    public var onAssertionCredentialId: ((Data) -> Void)? {
        didSet { core.onAssertionCredentialId = onAssertionCredentialId }
    }

    /// Protocol for providing a presentation anchor for the authorization controller.
    public typealias PresentationAnchorProvider = PasskeyPresentationAnchorProvider

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
    ///   - teamId: Optional explicit Apple Developer Team ID (10-character
    ///     alphanumeric, e.g. "F7R2LZH3W5"). Used only by
    ///     `checkDomainAssociation` to verify the app's identity against
    ///     Apple's AASA CDN. If nil, the team ID is auto-detected at call
    ///     time from the running app's code signature via
    ///     `SecTaskCopyValueForEntitlement(application-identifier)`. Auto-
    ///     detection works in virtually all real deployments; provide an
    ///     explicit value only for unit tests or sandboxed contexts where
    ///     the entitlement lookup fails.
    ///   - urlSession: Optional custom URLSession for the AASA CDN fetch.
    ///     Defaults to `.shared`. Override in tests to mock the HTTP layer.
    ///   - autoRegister: When `true`, `deriveSeed` automatically creates
    ///     a new passkey if none exists, then retries the assertion.
    ///     When `false` (default), throws `PrfProviderError.CredentialNotFound`
    ///     and the caller drives registration via `createPasskey()`.
    ///   - allowCredentialIds: When non-empty, restricts assertion (sign-in)
    ///     to one of the listed credential IDs. iOS will refuse any other
    ///     credential for this RP. Use this to bind sign-in to a specific
    ///     passkey the caller has registered, instead of letting iOS pick
    ///     any sibling credential that happens to share the RP. Critical
    ///     for deterministic seed derivation when multiple credentials
    ///     might exist for the same RP (e.g. test artifacts, or a user
    ///     who registered separately on multiple devices). When empty
    ///     (default), iOS picks any credential matching the RP.
    public init(
        rpId: String = "keys.breez.technology",
        rpName: String = "Breez SDK",
        userName: String? = nil,
        userDisplayName: String? = nil,
        anchorProvider: PresentationAnchorProvider? = nil,
        teamId: String? = nil,
        urlSession: URLSession = .shared,
        autoRegister: Bool = false,
        allowCredentialIds: [Data] = []
    ) {
        self.rpId = rpId
        self.rpName = rpName
        self.userName = userName ?? rpName
        self.userDisplayName = userDisplayName ?? (userName ?? rpName)
        self.autoRegister = autoRegister
        self.allowCredentialIds = allowCredentialIds
        self.explicitTeamId = teamId
        self.urlSession = urlSession
        self.core = PasskeyAssertionCore(anchorProvider: anchorProvider)
    }

    /// Derive multiple PRF outputs in as few authenticator ceremonies as
    /// possible. Uses the iOS 18+
    /// `ASAuthorizationPublicKeyCredentialPRFAssertionInputValues`
    /// dual-salt path: 2 derivations in a single user prompt.
    ///
    /// Salt count semantics:
    /// - 0 salts: returns empty without prompting.
    /// - 1 salt: equivalent to `deriveSeed`.
    /// - 2 salts: one ceremony.
    /// - 3+ salts: pairs are batched (N+1)/2 ceremonies; a trailing odd
    ///   salt uses the single-salt path.
    ///
    /// - Parameter salts: Salt strings in order.
    /// - Returns: One 32-byte output per salt, in input order.
    /// - Throws: `PrfProviderError` if any underlying ceremony fails. The
    ///   first failing ceremony aborts the rest.
    public func deriveSeeds(salts: [String]) async throws -> [Data] {
        let saltDatas: [Data] = salts.compactMap { $0.data(using: .utf8) }
        guard saltDatas.count == salts.count else {
            throw PrfProviderError.Generic("Failed to encode salts as UTF-8")
        }
        do {
            return try await core.performBulkDerivation(
                salts: saltDatas,
                rpId: rpId,
                rpName: rpName,
                userName: userName,
                userDisplayName: userDisplayName,
                autoRegister: autoRegister,
                explicitAllowCredentialIds: allowCredentialIds
            )
        } catch let err as PasskeyAssertionError {
            throw Self.toPrfProviderError(err)
        }
    }

    /// Create a new passkey with PRF support. Single platform prompt;
    /// separates credential creation from derivation in multi-step
    /// onboarding flows. Per-call overrides on `request` (userId,
    /// userName, userDisplayName) fall back to the constructor values.
    ///
    /// Auto-merges previously-registered credential IDs from
    /// [`KnownCredentialsStore`] into `request.excludeCredentialIds`
    /// so the platform refuses to create a duplicate even after a
    /// reinstall (the store is iCloud-synced). Records the new
    /// credential ID after a successful create.
    @discardableResult
    public func createPasskey(request: CreatePasskeyRequest) async throws -> RegisteredCredential {
        do {
            let credential = try await core.createPasskey(
                rpId: rpId,
                rpName: rpName,
                userName: request.userName ?? userName,
                userDisplayName: request.userDisplayName ?? userDisplayName,
                excludeCredentialIds: request.excludeCredentialIds,
                userId: request.userId
            )
            return RegisteredCredential(
                credentialId: credential.credentialId,
                aaguid: credential.aaguid,
                backupEligible: credential.backupEligible
            )
        } catch let err as PasskeyAssertionError {
            throw Self.toPrfProviderError(err)
        }
    }

    /// Check if this device's OS version supports the passkey PRF extension.
    ///
    /// This is an **API availability** check, not a readiness check:
    /// - Returns `true` whenever the OS exposes
    ///   `ASAuthorizationPlatformPublicKeyCredentialPRFAssertionInput`
    ///   (iOS 18.0+ / macOS 15.0+). Because this class is itself gated on
    ///   those versions via `@available`, any instance that can be
    ///   constructed will return `true`.
    /// - Does **not** verify that the user has Face ID / Touch ID /
    ///   a device passcode enrolled, or that iCloud Keychain / a third-party
    ///   credential provider is configured. Those states are handled by the
    ///   system at call time: when `deriveSeed` runs, the OS surfaces
    ///   its own "set up biometrics / pick a credential provider" prompts
    ///   and the call either succeeds or fails with a `PrfProviderError`
    ///   (e.g. `.userCancelled`, `.authenticationFailed`).
    ///
    /// Callers that need a stronger "ready to derive" signal should try a
    /// real `deriveSeed` and handle the error, rather than pre-checking.
    ///
    /// - Returns: `true` on supported OS versions.
    public func isSupported() async throws -> Bool {
        return true
    }

    /// Verify the app's bundle identifier is listed in the `webcredentials`
    /// section of the Apple app-site-association file served for `rpId`,
    /// via Apple's CDN (`app-site-association.cdn-apple.com`).
    ///
    /// # Why this check exists
    ///
    /// On iOS, Associated Domains verification is run by Apple's
    /// infrastructure at app install time and cached on-device. When the
    /// AASA file doesn't list your bundle ID (because it was never added,
    /// or because your app update shipped before Apple's CDN picked up a
    /// newly-deployed AASA), subsequent `ASAuthorizationController`
    /// requests fail with `ASAuthorizationError.notHandled` or `.failed`
    /// (errors that are **indistinguishable** from "no credential found"
    /// or "user cancelled" at the error-code layer).
    ///
    /// By proactively hitting the same CDN iOS consults
    /// (`app-site-association.cdn-apple.com/a/v1/<rpId>`), callers can
    /// detect this condition before the first WebAuthn ceremony and show
    /// a dedicated error state rather than falling through to the generic
    /// "passkey failed" handler.
    ///
    /// # Detection asymmetry
    ///
    /// - CDN lists this bundle: device will also list it (CDN is the
    ///   upstream; propagation is monotonic). Return `.associated`.
    /// - CDN does **not** list this bundle: device on this region almost
    ///   certainly does not either. Return `.notAssociated`.
    /// - CDN unreachable / timed out / returned invalid JSON: the check
    ///   itself couldn't complete; return `.skipped` and let the caller
    ///   proceed with the WebAuthn ceremony normally.
    ///
    /// # Team ID resolution
    ///
    /// AASA matches on the full `<TEAM_ID>.<BUNDLE_ID>` identity. The team
    /// ID comes from:
    /// 1. The `teamId` constructor parameter, if explicitly provided.
    /// 2. Otherwise, auto-detected from the running app's
    ///    `application-identifier` entitlement via
    ///    `SecTaskCopyValueForEntitlement`. The entitlement value is
    ///    `<TEAM_ID>.<BUNDLE_ID>`; the team ID is the prefix before the
    ///    first dot.
    ///
    /// If both sources fail (no explicit team ID AND entitlement lookup
    /// unavailable, e.g. unsigned test builds), returns `.skipped`.
    ///
    /// - Returns: A [`DomainAssociation`] describing the verification
    ///   outcome. Never throws; uses `.skipped` for verification-level
    ///   failures so callers have a single surface to handle.
    public func checkDomainAssociation() async throws -> DomainAssociation {
        // Delegate to the canonical core. Translate from the layer-
        // neutral `IosDomainAssociation` to UniFFI's `DomainAssociation`.
        let result = await core.checkDomainAssociation(
            rpId: rpId,
            explicitTeamId: explicitTeamId,
            urlSession: urlSession
        )
        switch result {
        case .associated:
            return .associated
        case .notAssociated(let source, let reason):
            return .notAssociated(source: source, reason: reason)
        case .skipped(let reason):
            return .skipped(reason: reason)
        }
    }

    /// Auto-detect the Apple Developer Team ID from the running app's
    /// signing information.
    ///
    // Team-ID detection lives in `PasskeyAssertionCore`'s
    // `PasskeyTeamIdDetector` so Flutter / RN plugins can use the same
    // logic. `checkDomainAssociation` above delegates to the core which
    // calls into the detector when no explicit team ID is supplied.

    // MARK: - PasskeyAssertionError -> PrfProviderError mapping

    private static func toPrfProviderError(_ err: PasskeyAssertionError) -> PrfProviderError {
        switch err {
        case .userCancelled:
            return .UserCancelled
        case .userTimedOut:
            return .UserTimedOut
        case .credentialNotFound:
            return .CredentialNotFound
        case .credentialAlreadyExists(let msg):
            return .CredentialAlreadyExists(msg)
        case .prfNotSupported:
            return .PrfNotSupported
        case .prfEvaluationFailed(let msg):
            return .PrfEvaluationFailed(msg)
        case .configuration(let msg):
            return .Configuration(msg)
        case .authenticationFailed(let msg):
            return .AuthenticationFailed(msg)
        case .generic(let msg):
            return .Generic(msg)
        }
    }
}

// MARK: - Error Extension

@available(iOS 18.0, macOS 15.0, *)
extension PrfProviderError {
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
