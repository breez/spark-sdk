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
    private let anchor: PresentationAnchorProvider
    private let explicitTeamId: String?
    private let urlSession: URLSession

    /// Protocol for providing a presentation anchor for the authorization controller.
    public protocol PresentationAnchorProvider {
        func presentationAnchor() -> ASPresentationAnchor
    }

    /// Default anchor provider that uses the key window from connected scenes.
    private class DefaultAnchorProvider: PresentationAnchorProvider {
        func presentationAnchor() -> ASPresentationAnchor {
            #if os(iOS)
            if let scene = UIApplication.shared.connectedScenes
                .compactMap({ $0 as? UIWindowScene })
                .first(where: { $0.activationState == .foregroundActive }),
                let window = scene.windows.first(where: { $0.isKeyWindow }) {
                return window
            }
            // Fallback: first available window
            if let window = UIApplication.shared.connectedScenes
                .compactMap({ $0 as? UIWindowScene })
                .flatMap({ $0.windows })
                .first {
                return window
            }
            return ASPresentationAnchor()
            #elseif os(macOS)
            return NSApplication.shared.keyWindow ?? ASPresentationAnchor()
            #endif
        }
    }

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
    ///   - autoRegister: When `true` (default), `derivePrfSeed` automatically
    ///     creates a new passkey if none exists for this RP ID, then retries
    ///     the assertion. When `false`, `derivePrfSeed` throws
    ///     `PasskeyPrfError.CredentialNotFound` instead, letting the caller
    ///     control registration separately via `createPasskey()`.
    public init(
        rpId: String = "keys.breez.technology",
        rpName: String = "Breez SDK",
        userName: String? = nil,
        userDisplayName: String? = nil,
        anchorProvider: PresentationAnchorProvider? = nil,
        teamId: String? = nil,
        urlSession: URLSession = .shared,
        autoRegister: Bool = true
    ) {
        self.rpId = rpId
        self.rpName = rpName
        self.userName = userName ?? rpName
        self.userDisplayName = userDisplayName ?? (userName ?? rpName)
        self.autoRegister = autoRegister
        self.anchor = anchorProvider ?? DefaultAnchorProvider()
        self.explicitTeamId = teamId
        self.urlSession = urlSession
    }

    /// Derive a 32-byte seed from passkey PRF with the given salt.
    ///
    /// Authenticates the user via a platform passkey and evaluates the PRF extension.
    /// If `autoRegister` is `true` (the default) and no credential exists for this
    /// RP ID, a new passkey is created automatically before retrying. If `autoRegister`
    /// is `false`, throws `PasskeyPrfError.CredentialNotFound` instead.
    ///
    /// - Parameter salt: The salt string to use for PRF evaluation.
    /// - Returns: The 32-byte PRF output.
    /// - Throws: `PasskeyPrfError` if authentication fails, PRF is not supported, or the user cancels.
    public func derivePrfSeed(salt: String) async throws -> Data {
        guard let saltData = salt.data(using: .utf8) else {
            throw PasskeyPrfError.Generic("Failed to encode salt as UTF-8")
        }

        // Try assertion first (existing credential)
        do {
            return try await performAssertionWithPrf(saltData: saltData)
        } catch let error as PasskeyPrfError where error.isCredentialNotFound {
            guard autoRegister else { throw error }

            // No credential found, auto-register a new one and retry
            do {
                _ = try await registerCredential()
            } catch let regError as PasskeyPrfError where regError.isCredentialNotFound {
                // Registration also got notHandled: the entitlement or
                // domain association is misconfigured, not a missing credential.
                throw PasskeyPrfError.Configuration(
                    "Associated Domains entitlement not configured. "
                    + "Add 'webcredentials:\(rpId)' to your app's entitlements "
                    + "and ensure a valid provisioning profile."
                )
            }
            return try await performAssertionWithPrf(saltData: saltData)
        }
    }

    /// Create a new passkey with PRF support.
    ///
    /// Only registers the credential — no seed derivation. Triggers exactly
    /// 1 platform prompt. Use this to separate credential creation from
    /// derivation in multi-step onboarding flows.
    ///
    /// - Parameters:
    ///   - excludeCredentialIds: Optional list of credential IDs to exclude.
    ///     Pass previously created credential IDs to prevent the authenticator
    ///     from creating a duplicate on the same device.
    /// - Returns: The credential ID of the newly created passkey.
    /// - Throws: `PasskeyPrfError` if the user cancels or PRF is not supported by the authenticator.
    @discardableResult
    public func createPasskey(excludeCredentialIds: [Data] = []) async throws -> Data {
        return try await registerCredential(excludeCredentialIds: excludeCredentialIds)
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
    ///   system at call time: when `derivePrfSeed` runs, the OS surfaces
    ///   its own "set up biometrics / pick a credential provider" prompts
    ///   and the call either succeeds or fails with a `PasskeyPrfError`
    ///   (e.g. `.userCancelled`, `.authenticationFailed`).
    ///
    /// Callers that need a stronger "ready to derive" signal should try a
    /// real `derivePrfSeed` and handle the error, rather than pre-checking.
    ///
    /// - Returns: `true` on supported OS versions.
    public func isPrfAvailable() async throws -> Bool {
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
    /// — errors that are **indistinguishable** from "no credential found"
    /// or "user cancelled" at the error-code layer.
    ///
    /// By proactively hitting the same CDN iOS consults
    /// (`app-site-association.cdn-apple.com/a/v1/<rpId>`), callers can
    /// detect this condition before the first WebAuthn ceremony and show
    /// a dedicated error state rather than falling through to the generic
    /// "passkey failed" handler.
    ///
    /// # Detection asymmetry
    ///
    /// - CDN lists this bundle → device will also list it (CDN is the
    ///   upstream; propagation is monotonic). Return `.associated`.
    /// - CDN does **not** list this bundle → device on this region almost
    ///   certainly does not either. Return `.notAssociated`.
    /// - CDN unreachable / timed out / returned invalid JSON → the check
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
    /// unavailable — e.g. unsigned test builds), returns `.skipped`.
    ///
    /// - Returns: A [`DomainAssociation`] describing the verification
    ///   outcome. Never throws; uses `.skipped` for verification-level
    ///   failures so callers have a single surface to handle.
    public func checkDomainAssociation() async throws -> DomainAssociation {
        let bundleId = Bundle.main.bundleIdentifier ?? ""
        guard !bundleId.isEmpty else {
            return .skipped(
                reason: "Bundle.main.bundleIdentifier is empty (unsigned / test context?)"
            )
        }

        guard let teamId = explicitTeamId ?? Self.detectTeamId() else {
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

    /// Auto-detect the Apple Developer Team ID from the running app's
    /// signing information.
    ///
    /// Two different mechanisms per platform, both yielding the same 10-char
    /// team ID, because the iOS public SDK doesn't expose the SecTask
    /// family:
    ///
    /// - **macOS**: read the `application-identifier` entitlement via
    ///   `SecTaskCopyValueForEntitlement`. Entitlement format is
    ///   `<TEAM_ID>.<BUNDLE_ID>`; split on the first dot.
    ///
    /// - **iOS**: parse `embedded.mobileprovision` from the app bundle.
    ///   Provisioning profiles are PKCS#7-wrapped plists — the plist bytes
    ///   are plain-text inside the CMS envelope, so we locate the
    ///   `<?xml>…</plist>` span and deserialize. The profile declares
    ///   `TeamIdentifier` (array of one entry, the team ID) and
    ///   `ApplicationIdentifierPrefix` (same value) — we use the former.
    ///
    ///   Simulator and unsigned builds don't ship a provisioning profile,
    ///   so this returns nil and `checkDomainAssociation` reports
    ///   `.skipped`.
    ///
    /// Returns nil when auto-detection fails; the caller then falls back
    /// to the `teamId` constructor argument, or reports `.skipped` if
    /// neither source yields an ID.
    private static func detectTeamId() -> String? {
        #if os(macOS)
        return detectTeamIdFromSecTask()
        #elseif os(iOS)
        return detectTeamIdFromProvisioningProfile()
        #else
        return nil
        #endif
    }

    #if os(macOS)
    private static func detectTeamIdFromSecTask() -> String? {
        guard let task = SecTaskCreateFromSelf(nil) else { return nil }
        let key = "application-identifier" as CFString
        var error: Unmanaged<CFError>?
        guard let value = SecTaskCopyValueForEntitlement(task, key, &error)
                as? String
        else { return nil }
        return parseTeamIdFromApplicationIdentifier(value)
    }
    #endif

    #if os(iOS)
    private static func detectTeamIdFromProvisioningProfile() -> String? {
        // Release / ad-hoc / TestFlight / Enterprise builds all embed
        // `embedded.mobileprovision` at the bundle root. App Store builds
        // also ship it. Simulator + unsigned builds do not.
        guard let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision"),
              let data = try? Data(contentsOf: url)
        else { return nil }
        // The PKCS#7 CMS envelope around the signed plist contains binary
        // DER bytes with values > 127. `.ascii` encoding rejects those and
        // returns nil for the whole string. `.isoLatin1` maps every byte
        // 0..255 to the corresponding Unicode code point 1:1 — it never
        // fails, and our ASCII sentinels (`<?xml`, `</plist>`) still match
        // identically because ASCII is a strict subset of Latin-1. We only
        // use the string to LOCATE the plist span; actual plist parsing
        // uses the raw Data slice (not the Latin-1 view) so non-ASCII
        // byte corruption in other regions doesn't matter.
        guard let raw = String(data: data, encoding: .isoLatin1),
              let startRange = raw.range(of: "<?xml"),
              let endRange = raw.range(of: "</plist>")
        else { return nil }
        // Convert character offsets back to byte offsets. Latin-1 is a 1:1
        // byte-to-codepoint map so the distance in characters equals the
        // distance in bytes, but Swift's String/Data bridging is safer
        // when we compute via utf16 offsets (Latin-1 codepoints are all
        // single-utf16-unit) and then read the corresponding Data slice.
        let startByteOffset = raw.utf16.distance(
            from: raw.utf16.startIndex,
            to: startRange.lowerBound.samePosition(in: raw.utf16) ?? raw.utf16.startIndex
        )
        let endByteOffset = raw.utf16.distance(
            from: raw.utf16.startIndex,
            to: endRange.upperBound.samePosition(in: raw.utf16) ?? raw.utf16.endIndex
        )
        guard startByteOffset < endByteOffset,
              endByteOffset <= data.count
        else { return nil }
        let plistData = data.subdata(in: startByteOffset..<endByteOffset)
        guard let plist = try? PropertyListSerialization.propertyList(
            from: plistData, format: nil
        ) as? [String: Any]
        else { return nil }
        // `TeamIdentifier` is an array of 10-char team IDs; first entry
        // is what we want. Defensive fallback to
        // `ApplicationIdentifierPrefix` which holds the same value.
        if let team = (plist["TeamIdentifier"] as? [String])?.first {
            return validateTeamId(team)
        }
        if let prefix = (plist["ApplicationIdentifierPrefix"] as? [String])?.first {
            return validateTeamId(prefix)
        }
        return nil
    }
    #endif

    private static func parseTeamIdFromApplicationIdentifier(_ value: String) -> String? {
        // `application-identifier` entitlement looks like
        // "F7R2LZH3W5.technology.breez.glow" — split on the first "."
        // to extract the team prefix.
        guard let firstDot = value.firstIndex(of: ".") else { return nil }
        return validateTeamId(String(value[..<firstDot]))
    }

    private static func validateTeamId(_ candidate: String) -> String? {
        // Apple Team IDs are 10-character alphanumeric identifiers.
        // Defensive guard against malformed entitlements / profiles.
        guard candidate.count == 10,
              candidate.allSatisfy({ $0.isLetter || $0.isNumber })
        else { return nil }
        return candidate
    }

    // MARK: - Private

    private func performAssertionWithPrf(saltData: Data) async throws -> Data {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = randomBytes(count: 32)
        let assertionRequest = provider.createCredentialAssertionRequest(challenge: challenge)

        // Configure PRF extension via ObjC helper
        // (PRF types are NS_REFINED_FOR_SWIFT with no accessible Swift initializers)
        PasskeyPRFHelper.setAssertionPRFOn(assertionRequest, withSalt: saltData)

        let delegate = AuthorizationDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [assertionRequest])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate
        delegate.anchor = anchor.presentationAnchor()

        return try await withCheckedThrowingContinuation { continuation in
            delegate.continuation = continuation
            delegate.extractPrf = true
            controller.performRequests()
        }
    }

    private func registerCredential(excludeCredentialIds: [Data] = []) async throws -> Data {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
        let challenge = randomBytes(count: 32)
        let userId = randomBytes(count: 16)
        let registrationRequest = provider.createCredentialRegistrationRequest(
            challenge: challenge,
            name: userName,
            userID: userId
        )

        if !excludeCredentialIds.isEmpty {
            registrationRequest.excludedCredentials = excludeCredentialIds.map {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
            }
        }

        // Request PRF support during registration via ObjC helper
        PasskeyPRFHelper.setRegistrationPRFOn(registrationRequest)

        let delegate = AuthorizationDelegate()
        let controller = ASAuthorizationController(authorizationRequests: [registrationRequest])
        controller.delegate = delegate
        controller.presentationContextProvider = delegate
        delegate.anchor = anchor.presentationAnchor()

        return try await withCheckedThrowingContinuation { continuation in
            delegate.continuation = continuation
            delegate.extractPrf = false
            controller.performRequests()
        }
    }

    private func randomBytes(count: Int) -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        _ = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        return Data(bytes)
    }
}

// MARK: - Authorization Delegate

@available(iOS 18.0, macOS 15.0, *)
private class AuthorizationDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    var continuation: CheckedContinuation<Data, Error>?
    var anchor: ASPresentationAnchor = ASPresentationAnchor()
    var extractPrf = true

    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        return anchor
    }

    func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        if extractPrf {
            // Assertion — extract PRF output
            guard let credential = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialAssertion
            else {
                continuation?.resume(
                    throwing: PasskeyPrfError.AuthenticationFailed(
                        "Unexpected credential type"))
                return
            }

            guard let prfData = PasskeyPRFHelper.extractPRFOutput(from: credential) else {
                continuation?.resume(throwing: PasskeyPrfError.PrfNotSupported)
                return
            }

            continuation?.resume(returning: prfData)
        } else {
            // Registration complete — extract and return the credential ID
            guard let credential = authorization.credential
                as? ASAuthorizationPlatformPublicKeyCredentialRegistration
            else {
                continuation?.resume(throwing: PasskeyPrfError.AuthenticationFailed("Unexpected credential type"))
                return
            }
            continuation?.resume(returning: credential.credentialID)
        }
    }

    func authorizationController(
        controller: ASAuthorizationController, didCompleteWithError error: Error
    ) {
        let mapped = mapAuthorizationError(error)
        continuation?.resume(throwing: mapped)
    }

    private func mapAuthorizationError(_ error: Error) -> PasskeyPrfError {
        let nsError = error as NSError

        if nsError.domain == ASAuthorizationError.errorDomain {
            switch ASAuthorizationError.Code(rawValue: nsError.code) {
            case .canceled:
                return .UserCancelled
            case .unknown:
                if nsError.localizedDescription.contains("no credential")
                    || nsError.localizedDescription.contains("No credentials")
                {
                    return .CredentialNotFound
                }
                return .AuthenticationFailed(nsError.localizedDescription)
            case .invalidResponse:
                return .PrfEvaluationFailed(nsError.localizedDescription)
            case .notHandled:
                return .CredentialNotFound
            case .failed:
                return .AuthenticationFailed(nsError.localizedDescription)
            case .notInteractive:
                return .AuthenticationFailed("User interaction required")
            case .matchedExcludedCredential:
                return .AuthenticationFailed("Credential already registered")
            default:
                return .Generic(nsError.localizedDescription)
            }
        }

        return .Generic(error.localizedDescription)
    }
}

// MARK: - Error Extension

@available(iOS 18.0, macOS 15.0, *)
extension PasskeyPrfError {
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
