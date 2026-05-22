use super::error::PrfProviderError;
use super::models::RegisteredCredential;

/// Per-call inputs for [`PrfProvider::derive_seeds`]. Hosts that
/// don't need per-ceremony overrides fall back to [`Default`]
/// (`salts` only, all overrides empty / `None`).
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeriveSeedsRequest {
    /// Salt strings in caller order. One 32-byte PRF output is
    /// returned per salt, in the same order.
    pub salts: Vec<String>,

    /// A list of credential IDs the assertion is restricted to. The
    /// primary use case is reauthentication when the user is already
    /// known: if any of the listed credentials is available locally,
    /// the OS prompts for device unlock straight away (no account
    /// picker); otherwise the user is asked to present another
    /// device (paired phone or security key) that holds a valid
    /// credential. Empty falls through to the provider's configured
    /// default.
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
    /// `request.allow_credentials` and
    /// `request.prefer_immediately_available_credentials` shape the
    /// platform ceremony for this single call. Custom providers that
    /// don't model those concepts (file-backed, `YubiKey` HMAC, etc.)
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
    /// hosts need for `exclude_credentials` bookkeeping. CLI /
    /// hardware providers register lazily inside [`Self::derive_seeds`]
    /// and inherit the default `PrfNotSupported`.
    ///
    /// `exclude_credentials` is a list of already-registered
    /// credential IDs: it prevents registering the same device twice
    /// by surfacing duplicates as `CredentialAlreadyExists`.
    /// Branding fields (`user_name`, `user_display_name`) live on
    /// the platform `PasskeyProvider` constructor. The `user.id` is
    /// always provider-minted and surfaced on
    /// `RegisteredCredential.user_id`.
    async fn create_passkey(
        &self,
        exclude_credentials: Vec<Vec<u8>>,
    ) -> Result<RegisteredCredential, PrfProviderError> {
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

    /// Take ownership of the credential ID observed during the most
    /// recent assertion ceremony, clearing the slot. Returns `None` if
    /// no assertion has completed since the last call OR if the
    /// provider does not surface this signal (the trait default).
    ///
    /// Built-in platform passkey providers (iOS, Android, Web JS)
    /// override this so [`PasskeyClient::sign_in`] can populate
    /// [`SignInResponse::credential_id`]. CLI / hardware providers
    /// (file-backed, FIDO2, `YubiKey`) inherit the `None` default.
    async fn take_last_observed_credential_id(&self) -> Option<Vec<u8>> {
        None
    }

    /// List credential IDs the provider has persisted for the current
    /// RP. Backs `PasskeyClient::credentials().get()`. Platform passkey
    /// providers delegate to their `CredentialRegistry` / native
    /// `KnownCredentialsStore`; file / `YubiKey` / FIDO2 providers
    /// inherit the empty-list default.
    async fn get_known_credential_ids(&self) -> Result<Vec<Vec<u8>>, PrfProviderError> {
        Ok(vec![])
    }

    /// Drop a single credential ID from the provider's persisted set
    /// for the current RP. Backs
    /// `PasskeyClient::credentials().remove(id)`. Default no-op for
    /// providers without a persistent registry.
    async fn remove_known_credential_id(&self, id: Vec<u8>) -> Result<(), PrfProviderError> {
        let _ = id;
        Ok(())
    }

    /// Clear the provider's persisted credential-ID set for the
    /// current RP. Backs `PasskeyClient::credentials().clear()`.
    /// Default no-op for providers without a persistent registry.
    async fn clear_known_credential_ids(&self) -> Result<(), PrfProviderError> {
        Ok(())
    }
}
