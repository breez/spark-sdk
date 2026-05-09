use super::error::PrfProviderError;
use super::models::{CreatePasskeyRequest, RegisteredCredential};

/// Per-call inputs for [`PrfProvider::derive_seeds`]. Bundles the salt
/// list with optional ceremony-shaping fields so providers can apply
/// them per call without forcing every host to reconstruct the
/// provider for each ceremony. Hosts that don't care fall back to
/// [`Default`] (= `salts` only, all overrides empty / `None`).
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeriveSeedsRequest {
    /// Salt strings in caller order. One 32-byte PRF output is
    /// returned per salt, in the same order.
    pub salts: Vec<String>,

    /// Per-call assertion allow-list. When non-empty, the platform is
    /// asked to refuse any credential whose ID is not in this list.
    /// Server-driven authentication (`/passkey/options` returning the
    /// user's known credentials) is the canonical use case. Empty
    /// (default) lets the provider's configured default apply (built-in
    /// providers fall through to their per-instance `allow_credential_ids`
    /// or to "any matching credential" when that is also empty).
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub allow_credential_ids: Vec<Vec<u8>>,

    /// Per-call control over the platform's "fast-fail when no local
    /// credential is available" behavior. `Some(true)` (the historical
    /// default) suppresses the cross-device picker and lets a missing
    /// credential surface as `CredentialNotFound` immediately.
    /// `Some(false)` opts back into the OS picker (e.g. cross-device
    /// QR sign-in on iOS, hybrid transports on Android, browser
    /// `mediation: undefined` on web). `None` means "use the
    /// provider's default" (same as `Some(true)` for built-in
    /// providers).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub prefer_immediately_available_credentials: Option<bool>,
}

/// Result of [`PrfProvider::check_domain_association`]. The platform's
/// out-of-band verification (AASA / assetlinks) gates passkey
/// ceremonies but its failures collapse into opaque platform errors;
/// this gives callers a definitive signal they can gate UX on.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DomainAssociation {
    /// Configuration verified; safe to proceed.
    Associated,
    /// Configuration is broken; subsequent ceremonies will fail.
    /// `source` names the verification origin (e.g. `"Apple AASA CDN"`)
    /// for diagnostic UIs; `reason` explains what was missing.
    NotAssociated { source: String, reason: String },
    /// Check was not performed (provider has no verification source,
    /// or the check itself could not complete). Not a negative signal.
    Skipped { reason: String },
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
    /// Derive 32-byte PRF outputs for `request.salts` in as few
    /// authenticator ceremonies as the platform supports. Output
    /// ordering matches input ordering. Empty `salts` returns an
    /// empty vec without prompting. Built-in providers chunk pairs
    /// via `WebAuthn`'s `prf.eval.first` + `.second` (halving prompt
    /// count); custom providers without bulk capability should loop
    /// internally.
    ///
    /// `request.allow_credential_ids` and
    /// `request.prefer_immediately_available_credentials` shape the
    /// platform ceremony for this single call. Custom providers that
    /// don't model those concepts (file-backed, YubiKey HMAC, etc.)
    /// can ignore them.
    async fn derive_seeds(
        &self,
        request: DeriveSeedsRequest,
    ) -> Result<Vec<Vec<u8>>, PrfProviderError>;

    /// Whether this provider can produce PRF outputs on the current
    /// device. Hosts gate UX on the result.
    async fn is_supported(&self) -> Result<bool, PrfProviderError>;

    /// Explicit registration. Platform passkey providers override this
    /// to drive the OS create ceremony and surface credential metadata
    /// hosts need for `exclude_credential_ids` bookkeeping. CLI /
    /// hardware providers register lazily inside [`Self::derive_seeds`]
    /// and inherit the default `PrfNotSupported`.
    async fn create_passkey(
        &self,
        request: CreatePasskeyRequest,
    ) -> Result<RegisteredCredential, PrfProviderError> {
        let _ = request;
        Err(PrfProviderError::PrfNotSupported)
    }

    /// Advisory check against the platform's out-of-band verification
    /// source (iOS AASA / Android assetlinks / browser rpId scope).
    /// The SDK never gates internally; hosts pick their own policy.
    ///
    /// Built-in providers override:
    /// - **iOS/macOS**: AASA `webcredentials.apps` lookup. May be stale.
    /// - **Android**: Digital Asset Links query. Degrades `NotAssociated`
    ///   to `Skipped` because `CredentialManager` runs its own check.
    /// - **Browser**: `rpId` is a registrable suffix of `window.location.hostname`.
    ///
    /// Custom providers without a verification source inherit the
    /// `Skipped` default.
    async fn check_domain_association(&self) -> Result<DomainAssociation, PrfProviderError> {
        Ok(DomainAssociation::Skipped {
            reason: "Provider does not verify domain association".to_string(),
        })
    }

    /// Take ownership of the credential ID observed during the most
    /// recent assertion ceremony, clearing the slot. Returns `None` if
    /// no assertion has completed since the last call OR if the
    /// provider does not surface this signal (the trait default).
    ///
    /// Built-in platform passkey providers (iOS, Android, Web JS)
    /// override this so [`PasskeyClient::sign_in`] can populate
    /// [`SignInResponse::credential_id`]. CLI / hardware providers
    /// (file-backed, FIDO2, YubiKey) inherit the `None` default.
    async fn take_last_observed_credential_id(&self) -> Option<Vec<u8>> {
        None
    }
}
