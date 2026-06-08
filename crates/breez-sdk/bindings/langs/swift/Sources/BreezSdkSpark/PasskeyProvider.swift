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
    /// Apple Developer Team ID (10-char). Used by `checkDomainAssociation`
    /// to verify the app against Apple's AASA CDN. `nil` auto-detects from
    /// the signing entitlement; override only when that's unavailable
    /// (unit tests, sandboxed contexts).
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

    /// Default Relying Party name used by the zero-config
    /// ``PasskeyClient/init(breezApiKey:rpId:rpName:config:)`` and
    /// ``PasskeyClientBuilder`` when no `rpName` is supplied. Surfaces in
    /// some credential-manager UIs (iCloud Keychain, Google Password
    /// Manager).
    public static let defaultRpName: String = "Breez"

    private let rpId: String
    private let core: PasskeyAssertionCore

    /// Protocol for providing a presentation anchor for the authorization controller.
    public typealias PresentationAnchorProvider = PasskeyPresentationAnchorProvider

    /// Create a new platform passkey PRF provider.
    ///
    /// - Parameters:
    ///   - options: Relying Party and user identity (`rpId`, `rpName`,
    ///     `userName`, `userDisplayName`). Unset `rpId` / `rpName` default to
    ///     `PasskeyProvider.BREEZ_RP_ID` / `"Breez"`; `userName` defaults to
    ///     `rpName` and `userDisplayName` to `userName`. The same
    ///     `PasskeyProviderOptions` is settable on `PasskeyConfig` for the
    ///     zero-config client.
    ///   - iosOptions: iOS / macOS-specific knobs (team ID, URLSession,
    ///     presentation anchor). Defaults work for any signed build.
    public init(
        options: PasskeyProviderOptions = PasskeyProviderOptions(),
        iosOptions: IOSOptions? = nil
    ) {
        let rpId = options.rpId ?? PasskeyProvider.BREEZ_RP_ID
        let rpName = options.rpName ?? PasskeyProvider.defaultRpName
        let userName = options.userName ?? rpName
        self.rpId = rpId
        self.core = PasskeyAssertionCore(
            rpId: rpId,
            rpName: rpName,
            userName: userName,
            userDisplayName: options.userDisplayName ?? userName,
            explicitTeamId: iosOptions?.teamId,
            urlSession: iosOptions?.urlSession ?? .shared,
            anchorProvider: iosOptions?.anchorProvider
        )
    }

    /// Derive multiple PRF outputs in as few authenticator ceremonies as
    /// possible, using the iOS 18+ dual-salt path (2 derivations per prompt).
    ///
    /// Salt count semantics:
    /// - 0 salts: returns empty without prompting.
    /// - 1 salt: equivalent to `deriveSeed`.
    /// - 2 salts: one ceremony.
    /// - 3+ salts: pairs are batched into (N+1)/2 ceremonies.
    ///
    /// - Parameter salts: Salt strings in order.
    /// - Returns: One 32-byte output per salt (in input order) plus the
    ///   credential ID observed in the same assertion (nil when none).
    /// - Throws: `PrfProviderError` if any ceremony fails; the first
    ///   failure aborts the rest.
    ///
    /// Never auto-creates a credential during derivation: a missing
    /// credential surfaces as `.credentialNotFound`, not a new passkey.
    public func deriveSeeds(request: DeriveSeedsRequest) async throws -> DeriveSeedsOutput {
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
            let derivation = try await core.deriveSeeds(
                salts: saltDatas,
                autoRegister: false,
                allowCredentials: request.allowCredentials.map { Data($0) },
                preferImmediatelyAvailableCredentials:
                    request.preferImmediatelyAvailableCredentials ?? true
            )
            // The core observes the asserted credential ID inline and
            // returns it alongside the seeds.
            return DeriveSeedsOutput(seeds: derivation.seeds, credentialId: derivation.credentialId)
        } catch let err as PasskeyAssertionError {
            throw Self.toPrfProviderError(err)
        }
    }

    /// Create a new PRF-capable passkey (single platform prompt). Use it to
    /// separate registration from derivation in multi-step onboarding.
    ///
    /// `user.id` is never host-supplied: the core mints a fresh random
    /// 16-byte handle and returns it as `PasskeyCredential.userId`. Pass
    /// already-registered IDs in `excludeCredentials` so the platform
    /// refuses a duplicate even after reinstall.
    @discardableResult
    public func createPasskey(excludeCredentials: [Data]) async throws -> PasskeyCredential {
        do {
            let credential = try await core.register(excludeCredentials: excludeCredentials)
            return PasskeyCredential(
                credentialId: credential.credentialId,
                userId: credential.userId,
                aaguid: credential.aaguid,
                backupEligible: credential.backupEligible
            )
        } catch let err as PasskeyAssertionError {
            throw Self.toPrfProviderError(err)
        }
    }

    /// Whether this OS version exposes the passkey PRF API (iOS 18+ /
    /// macOS 15+). An availability check, not a readiness check: it does not
    /// verify biometric enrollment or a configured credential provider,
    /// which the OS handles at ceremony time. For a stronger "ready to
    /// derive" signal, attempt a real `deriveSeed` and handle the error.
    /// Always `true` here, since the class is `@available`-gated.
    public func isSupported() async throws -> Bool {
        return true
    }

    /// Verify the app's bundle ID is listed in the `webcredentials` section
    /// of the apple-app-site-association file for `rpId`, via Apple's CDN.
    ///
    /// Associated Domains is normally verified by iOS at install time and
    /// cached. If the AASA doesn't list the app, WebAuthn ceremonies fail
    /// with errors indistinguishable from "no credential" or "user
    /// cancelled". Calling this first lets the host show a dedicated
    /// misconfiguration error instead of a generic "passkey failed".
    ///
    /// The team ID for the `<TEAM_ID>.<BUNDLE_ID>` match comes from the
    /// `teamId` option, else the app's `application-identifier` entitlement.
    ///
    /// - Returns: A [`DomainAssociation`]. Never throws: returns `.skipped`
    ///   when the check can't complete (CDN unreachable, unsigned build) so
    ///   the caller can proceed with the ceremony normally.
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

