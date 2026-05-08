use super::error::PrfProviderError;
use super::models::{CreatePasskeyRequest, RegisteredCredential};

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
    /// Derive 32-byte PRF outputs for `salts` in as few authenticator
    /// ceremonies as the platform supports. Output ordering matches
    /// input ordering. Empty `salts` returns an empty vec without
    /// prompting. Built-in providers chunk pairs via `WebAuthn`'s
    /// `prf.eval.first` + `.second` (halving prompt count); custom
    /// providers without bulk capability should loop internally.
    async fn derive_seeds(&self, salts: Vec<String>) -> Result<Vec<Vec<u8>>, PrfProviderError>;

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
}
