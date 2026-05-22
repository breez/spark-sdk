import AuthenticationServices
import Foundation
import PasskeyPRFHelperObjC
import Security

/// iOS / macOS platform-specific options for [`PasskeyProvider`].
///
/// Bundles the three iOS-only knobs (team ID, URLSession, presentation
/// anchor) that integrators rarely need to set. Defaults work for any
/// signed App Store / TestFlight build.
@available(iOS 18.0, macOS 15.0, *)
public struct IOSOptions {
    /// Apple Developer Team ID (10-character alphanumeric). Used by
    /// `checkDomainAssociation` to verify the app against Apple's
    /// AASA CDN. `nil` auto-detects from the signing entitlement;
    /// override only when entitlement lookup is unavailable (unit
    /// tests, sandboxed contexts).
    public let teamId: String?

    /// Custom URLSession for the AASA CDN fetch. Defaults to
    /// `.shared`. Override in tests to mock the HTTP layer.
    public let urlSession: URLSession

    /// Custom presentation anchor provider for the
    /// `ASAuthorizationController`. `nil` uses the foreground key
    /// window (correct for ~every iOS app).
    public let anchorProvider: PasskeyPresentationAnchorProvider?

    public init(
        teamId: String? = nil,
        urlSession: URLSession = .shared,
        anchorProvider: PasskeyPresentationAnchorProvider? = nil
    ) {
        self.teamId = teamId
        self.urlSession = urlSession
        self.anchorProvider = anchorProvider
    }
}

/// Built-in passkey-based PRF provider for iOS / macOS using the
/// AuthenticationServices framework.
///
/// Uses `ASAuthorizationPlatformPublicKeyCredentialProvider` with the
/// PRF extension to derive deterministic 32-byte seeds from passkeys.
///
/// Requirements:
/// - iOS 18.0+ / macOS 15.0+
/// - Associated Domains entitlement: `webcredentials:<rpId>`
/// - The domain's `apple-app-site-association` must list your app
@available(iOS 18.0, macOS 15.0, *)
public class PasskeyProvider: PrfProvider {
    /// Constant identifying Breez's shared `keys.breez.technology` RP.
    /// Pass as `rpId` when opting into the Breez-managed Relying Party
    /// (only valid for apps registered with Breez). Apps with their
    /// own RP domain pass their own string.
    public static let BREEZ_RP_ID: String = "keys.breez.technology"

    private let rpId: String
    private let core: PasskeyAssertionCore
    private let credentialRegistry: CredentialRegistry?
    private let onRegistryError: (@Sendable (RegistryOperation, Error) -> Void)?

    /// Take ownership of the credential ID captured by the most recent
    /// successful assertion. Returns nil if no assertion has completed
    /// since the last call. Used by binding-layer code that wants to
    /// surface the credential ID to a higher-level response type.
    public func takeLastObservedCredentialId() -> Data? {
        core.takeLastObservedCredentialId()
    }

    /// Protocol for providing a presentation anchor for the authorization controller.
    public typealias PresentationAnchorProvider = PasskeyPresentationAnchorProvider

    /// Create a new platform passkey PRF provider.
    ///
    /// - Parameters:
    ///   - rpId: **Required.** Relying Party ID. Pass your app's domain,
    ///     or `PasskeyProvider.BREEZ_RP_ID` to opt into Breez's shared
    ///     `keys.breez.technology` RP (only valid for Breez-registered apps).
    ///     Changing this after users have registered passkeys will make
    ///     their existing credentials undiscoverable.
    ///   - rpName: **Required.** Maps to the WebAuthn `rp.name`.
    ///     Deprecated in WebAuthn L3 but still required by current
    ///     iOS / macOS prompts. Surfaces in some credential-management
    ///     UIs (iCloud Keychain, Google Password Manager, 1Password);
    ///     platform UIs increasingly ignore it. Only used at
    ///     credential registration; changing it does not affect
    ///     existing credentials.
    ///   - userName: Maps to the WebAuthn `user.name`. Treated as the
    ///     user's unique identifier for the credential and shown in
    ///     Apple's account picker during sign-in. Pass a stable
    ///     per-user value if each registration should surface as a
    ///     distinct entry (iCloud Keychain dedupes by
    ///     `(rpId, user.name)`). Defaults to `rpName`. Only used at
    ///     registration; changing it does not affect existing
    ///     credentials.
    ///   - userDisplayName: Maps to the WebAuthn `user.displayName`.
    ///     The user-friendly label the OS MAY (but is not required to)
    ///     show in the picker. Defaults to `userName`. Only used at
    ///     registration; changing it does not affect existing
    ///     credentials.
    ///   - credentialRegistry: Opt-in app-side store of known
    ///     credential IDs. When supplied, the SDK auto-merges stored
    ///     IDs into `allowCredentials` / `excludeCredentials` and
    ///     writes new IDs back after success.
    ///   - onRegistryError: Best-effort callback for registry failures;
    ///     never blocks the WebAuthn ceremony.
    ///   - iosOptions: iOS / macOS-specific knobs (team ID for AASA CDN,
    ///     custom URLSession, presentation anchor). Defaults work for
    ///     every signed App Store / TestFlight build; override only for
    ///     unit tests or unusual presentation setups.
    public init(
        rpId: String,
        rpName: String,
        userName: String? = nil,
        userDisplayName: String? = nil,
        credentialRegistry: CredentialRegistry? = nil,
        onRegistryError: (@Sendable (RegistryOperation, Error) -> Void)? = nil,
        iosOptions: IOSOptions? = nil
    ) {
        self.rpId = rpId
        self.credentialRegistry = credentialRegistry
        self.onRegistryError = onRegistryError
        self.core = PasskeyAssertionCore(
            rpId: rpId,
            rpName: rpName,
            userName: userName ?? rpName,
            userDisplayName: userDisplayName ?? (userName ?? rpName),
            credentialRegistry: credentialRegistry,
            onRegistryError: onRegistryError,
            explicitTeamId: iosOptions?.teamId,
            urlSession: iosOptions?.urlSession ?? .shared,
            anchorProvider: iosOptions?.anchorProvider
        )
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
    ///
    /// Passes `autoRegister: false`: this provider never implicitly
    /// creates a credential during derivation. Sign-up and sign-in stay
    /// explicit (the host calls `createPasskey` to register), so a
    /// missing credential surfaces as `.credentialNotFound` rather than
    /// silently minting a new passkey. (The core defaults `autoRegister`
    /// to true for direct callers; the provider opts out.)
    public func deriveSeeds(request: DeriveSeedsRequest) async throws -> [Data] {
        // Map (not compactMap) so a salt that somehow can't UTF-8 encode
        // fails loudly with its position rather than being dropped and
        // detected after the fact by a count mismatch.
        let saltDatas = try request.salts.map { salt -> Data in
            guard let data = salt.data(using: .utf8) else {
                throw PrfProviderError.Generic("Failed to encode salt as UTF-8: \(salt)")
            }
            return data
        }
        do {
            return try await core.deriveSeeds(
                salts: saltDatas,
                autoRegister: false,
                allowCredentials: request.allowCredentials.map { Data($0) },
                preferImmediatelyAvailableCredentials:
                    request.preferImmediatelyAvailableCredentials ?? true
            )
        } catch let err as PasskeyAssertionError {
            throw Self.toPrfProviderError(err)
        }
    }

    /// Create a new passkey with PRF support. Single platform prompt;
    /// separates credential creation from derivation in multi-step
    /// onboarding flows.
    ///
    /// `user.id` is never host-supplied: the core mints a fresh random
    /// 16-byte handle per call and surfaces it via
    /// `RegisteredCredential.userId`. Branding fields (`userName`,
    /// `userDisplayName`) live on this provider's constructor.
    ///
    /// Auto-merges previously-registered credential IDs from the
    /// optional `CredentialRegistry` into `excludeCredentials` so the
    /// platform refuses to create a duplicate even after a reinstall
    /// (the registry is iCloud-synced when host-backed). Records the
    /// new credential ID after a successful create.
    @discardableResult
    public func createPasskey(excludeCredentials: [Data]) async throws -> RegisteredCredential {
        do {
            let credential = try await core.register(excludeCredentials: excludeCredentials)
            return RegisteredCredential(
                credentialId: credential.credentialId,
                userId: credential.userId,
                aaguid: credential.aaguid,
                backupEligible: credential.backupEligible
            )
        } catch let err as PasskeyAssertionError {
            throw Self.toPrfProviderError(err)
        }
    }

    public func getKnownCredentialIds() async throws -> [Data] {
        guard let reg = credentialRegistry else { return [] }
        do {
            return try await reg.read(rpId: rpId)
        } catch {
            onRegistryError?(.read, error)
            return []
        }
    }

    public func removeKnownCredentialId(id: Data) async throws {
        guard let reg = credentialRegistry else { return }
        do {
            try await reg.remove(rpId: rpId, credentialId: id)
        } catch {
            onRegistryError?(.remove, error)
        }
    }

    public func clearKnownCredentialIds() async throws {
        guard let reg = credentialRegistry else { return }
        do {
            try await reg.clear(rpId: rpId)
        } catch {
            onRegistryError?(.clear, error)
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
        let result = await core.checkDomainAssociation()
        switch result {
        case .associated:
            return .associated
        case .notAssociated(let source, let reason):
            return .notAssociated(source: source, reason: reason)
        case .skipped(let reason):
            return .skipped(reason: reason)
        }
    }

    // MARK: - PasskeyAssertionError -> PrfProviderError mapping

    private static func toPrfProviderError(
        _ err: PasskeyAssertionError
    ) -> PrfProviderError {
        switch err {
        case .userCancelled:
            return .UserCancelled
        case .userTimedOut:
            return .UserTimedOut
        case .credentialNotFound(let msg):
            // The core embeds any registry help suffix into the message
            // when relevant. Pass through unchanged so every host
            // surface (UniFFI, Flutter, RN) sees the same diagnostic.
            return .CredentialNotFound(msg)
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

