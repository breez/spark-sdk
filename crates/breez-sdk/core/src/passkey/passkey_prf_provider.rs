use super::error::PasskeyPrfError;

/// Result of a domain-association verification check against the platform's
/// well-known configuration source.
///
/// Passkey operations on iOS and Android both depend on out-of-band
/// verification files (`apple-app-site-association` / `assetlinks.json`) that
/// the platform caches independently of the app. When the verification is
/// missing or stale, the OS-level `WebAuthn` APIs fail with opaque errors
/// (`ASAuthorizationError.notHandled` / `.failed` on iOS; assorted
/// `GetCredentialException` variants on Android) that callers cannot reliably
/// distinguish from "no credential found" or "user cancelled".
///
/// [`PrfProvider::check_domain_association`] runs an active check
/// against the platform's own verification source (Apple's AASA CDN or
/// Google's Digital Asset Links API) so callers have a definitive signal
/// they can gate UX on — without heuristics over overloaded error codes.
///
/// # Caller semantics
///
/// - `Associated`: safe to proceed with `WebAuthn` calls.
/// - `NotAssociated`: subsequent `WebAuthn` calls will fail for
///   configuration reasons. Callers should surface a dedicated error state
///   rather than attempting the ceremony (which would produce an opaque
///   error that looks identical to "no credential").
/// - `Skipped`: the provider does not verify domain association, or the
///   check could not be performed (offline, endpoint timeout). Callers
///   should proceed with `WebAuthn` normally — `Skipped` is **not** a
///   negative signal.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DomainAssociation {
    /// The app's identity (bundle ID / package name + signing cert) is
    /// confirmed present in the platform's verification source for the
    /// configured `rpId`.
    Associated,
    /// The app's identity is confirmed **missing** from the platform's
    /// verification source. Subsequent `WebAuthn` calls will fail.
    NotAssociated {
        /// Origin of the verification check (e.g. `"Apple AASA CDN"`,
        /// `"Google Digital Asset Links API"`). Surfaced in diagnostic UIs
        /// and logs so maintainers can tell which side to fix.
        source: String,
        /// Human-readable explanation of what was missing (e.g.
        /// `"Bundle ID F7R2LZH3W5.technology.breez.glow not in
        /// webcredentials.apps for keys.breez.technology"`).
        reason: String,
    },
    /// Verification was not performed. The provider either does not have a
    /// verification source to check (custom / CLI providers, browser-side
    /// `WebAuthn`), or the check itself could not complete (network offline,
    /// CDN timeout). Callers proceed with `WebAuthn` as normal.
    Skipped {
        /// Human-readable reason for skipping (e.g. `"Provider does not
        /// verify domain association"`, `"Apple CDN request timed out
        /// after 3s"`).
        reason: String,
    },
}

/// Trait for PRF (Pseudo-Random Function) operations backing a passkey-derived
/// wallet seed.
///
/// The built-in passkey provider on each platform (`PasskeyProvider`)
/// implements this trait by authenticating with a platform passkey and
/// evaluating the `WebAuthn` PRF extension. Custom providers (CLI tools
/// backed by `YubiKey`, FIDO2 hmac-secret, on-disk key material, hardware
/// HSMs) also implement this trait, anything that can deterministically
/// derive 32 bytes from a salt is a valid `PrfProvider`.
///
/// The implementation is responsible for:
/// - Authenticating the user via platform-specific passkey APIs (`WebAuthn`, native passkey managers) or another deterministic source
/// - Evaluating the PRF extension (or equivalent) with the provided salt
/// - Returning the 32-byte PRF output
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait PrfProvider: Send + Sync {
    /// Derive a 32-byte seed from passkey PRF with the given salt.
    ///
    /// The platform authenticates the user via passkey and evaluates the PRF extension.
    /// The salt is used as input to the PRF to derive a deterministic output.
    ///
    /// # Arguments
    /// * `salt` - The salt string to use for PRF evaluation
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` - The 32-byte PRF output
    /// * `Err(PasskeyPrfError)` - If authentication fails or PRF is not supported
    async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError>;

    /// Check if a PRF-capable passkey is available on this device.
    ///
    /// This allows applications to gracefully degrade if passkey PRF is not supported.
    ///
    /// # Returns
    /// * `Ok(true)` - PRF-capable passkey is available
    /// * `Ok(false)` - No PRF-capable passkey available
    /// * `Err(PasskeyPrfError)` - If the check fails
    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError>;

    /// Diagnostic check: verify the app's identity against the platform's
    /// domain verification source (iOS AASA / Android assetlinks / browser
    /// rpId scope).
    ///
    /// This method is **advisory**. The SDK never blocks internally on its
    /// result. Applications choose their own policy for how to use it.
    ///
    /// # When to call
    ///
    /// - **First launch / onboarding**: call once to catch misconfiguration
    ///   early, before the first WebAuthn ceremony.
    /// - **Error recovery**: if `derivePrfSeed` returns `CredentialNotFound`
    ///   but you expect a credential to exist, call this to distinguish
    ///   "genuinely no credential" from "platform configuration is broken."
    ///   On iOS, `ASAuthorizationError.notHandled` collapses both cases into
    ///   the same error code, making this check the only reliable diagnostic.
    /// - **Not needed per-session**: once association is confirmed, the
    ///   configuration rarely changes. Re-check only after errors.
    ///
    /// # Platform behavior
    ///
    /// The built-in `PasskeyProvider` on each platform overrides this with
    /// an active check:
    /// - **iOS/macOS**: verifies the app's bundle ID + team ID appear in the
    ///   AASA `webcredentials.apps` array for the RP domain. Results may be
    ///   stale if the AASA file was recently updated.
    /// - **Android**: queries the Digital Asset Links API for a matching
    ///   `get_login_creds` statement. Returns `Skipped` (not `NotAssociated`)
    ///   on mismatch, because Android's CredentialManager performs its own
    ///   internal verification that may be more up-to-date than the public API.
    /// - **Browser**: checks that `rpId` is a registrable suffix of
    ///   `window.location.hostname` (the same rule the browser enforces
    ///   at WebAuthn call time). No network request needed.
    ///
    /// Custom providers (YubiKey, FIDO2, file-backed) that have no platform
    /// verification source should inherit the default, which returns
    /// `Skipped`.
    ///
    /// # Returns
    /// * `Ok(DomainAssociation::Associated)`: configuration looks correct.
    /// * `Ok(DomainAssociation::NotAssociated { .. })`: configuration problem
    ///   detected. On iOS this is a hard signal (AASA mismatch). On Android
    ///   the provider degrades this to `Skipped` (see above).
    /// * `Ok(DomainAssociation::Skipped { .. })`: check was not performed or
    ///   could not complete. Proceed normally.
    /// * `Err(PasskeyPrfError)`: the check mechanism itself failed.
    async fn check_domain_association(&self) -> Result<DomainAssociation, PasskeyPrfError> {
        Ok(DomainAssociation::Skipped {
            reason: "Provider does not verify domain association".to_string(),
        })
    }
}
