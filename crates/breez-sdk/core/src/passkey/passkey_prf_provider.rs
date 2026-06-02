use super::error::PrfProviderError;
use super::models::{DeriveSeedsOutput, PasskeyCredential};

/// Per-call inputs for [`PrfProvider::derive_seeds`]. Hosts that
/// don't need per-ceremony overrides fall back to [`Default`]
/// (`salts` only, all overrides empty / `None`).
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeriveSeedsRequest {
    /// Salt strings in caller order. One 32-byte PRF output is
    /// returned per salt, in the same order.
    pub salts: Vec<String>,

    /// Credential IDs the assertion is restricted to. The main use is
    /// reauthenticating a known user: if a listed credential is on the
    /// device the OS unlocks straight away (no account picker); otherwise
    /// it asks for another device (paired phone, security key) holding one.
    /// Empty falls through to the provider's configured default.
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub allow_credentials: Vec<Vec<u8>>,

    /// Restrict the assertion to credentials already present on this
    /// device. When `true`, the OS skips the cross-device picker (iOS
    /// QR, Android hybrid, web `mediation: undefined`) and surfaces a
    /// missing local credential as `CredentialNotFound` immediately.
    /// When `false`, the OS picker is shown as usual. Unset uses the
    /// provider's default (`true` for built-in providers).
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

/// Trait for PRF (Pseudo-Random Function) operations backing a
/// passkey-derived wallet seed.
///
/// Each platform's built-in `PasskeyProvider` implements this by
/// authenticating with a platform passkey and evaluating the `WebAuthn`
/// PRF extension. Custom providers (CLI tools backed by `YubiKey`, FIDO2
/// hmac-secret, on-disk key material, HSMs) implement the same contract:
/// anything that deterministically derives 32 bytes from a salt qualifies.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait PrfProvider: Send + Sync {
    /// Derive 32-byte PRF outputs for `request.salts` in as few
    /// authenticator ceremonies as the platform supports, preserving input
    /// order. Empty `salts` returns an empty vec without prompting. Built-in
    /// providers pair salts via `WebAuthn`'s `prf.eval.first` + `.second`
    /// (halving prompts); custom providers without bulk support loop.
    ///
    /// `request.allow_credentials` and
    /// `request.prefer_immediately_available_credentials` shape this single
    /// ceremony; providers that don't model them (file-backed, `YubiKey`)
    /// ignore them. Returns the seeds plus the credential ID observed in the
    /// same assertion, absent when the provider does not surface it.
    async fn derive_seeds(
        &self,
        request: DeriveSeedsRequest,
    ) -> Result<DeriveSeedsOutput, PrfProviderError>;

    /// Whether this provider can produce PRF outputs on the current
    /// device. Hosts gate UX on the result.
    async fn is_supported(&self) -> Result<bool, PrfProviderError>;

    /// Explicit registration. Platform passkey providers override this to
    /// drive the OS create ceremony and surface the credential metadata
    /// hosts need for `exclude_credentials` bookkeeping. CLI / hardware
    /// providers register lazily in [`Self::derive_seeds`] and inherit the
    /// default `PrfNotSupported`.
    ///
    /// `exclude_credentials` lists already-registered IDs and surfaces
    /// duplicates as `CredentialAlreadyExists`. The `user.id` is always
    /// provider-minted and returned on `PasskeyCredential.user_id`.
    async fn create_passkey(
        &self,
        exclude_credentials: Vec<Vec<u8>>,
    ) -> Result<PasskeyCredential, PrfProviderError> {
        let _ = exclude_credentials;
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
}
