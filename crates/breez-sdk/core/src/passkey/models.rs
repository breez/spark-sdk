use std::collections::HashMap;

use crate::Seed;

/// A wallet derived from a passkey.
///
/// Contains the derived seed and the label used during derivation.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Wallet {
    /// The derived seed.
    pub seed: Seed,
    /// The label used for derivation (either user-provided or the default).
    pub label: String,
}

/// A caller-supplied salt to derive an extra 32-byte seed in the same
/// PRF ceremony as the wallet seed. Output keyed by `name` in
/// [`WalletSetup::extra_seeds`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NamedSalt {
    /// Caller-chosen name; appears as the lookup key on the result.
    pub name: String,
}

/// Request for [`crate::passkey::Passkey::setup_wallet`].
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SetupWalletRequest {
    /// User-chosen label. Defaults to the configured default label
    /// (`PasskeyConfig::default_label`, or `"Default"` when unset) when
    /// `None`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub label: Option<String>,
    /// Whether to publish `label` to Nostr after deriving. Pass `false`
    /// for speculative derivations (cold restore).
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub publish_label: bool,
    /// Extra salts to derive in the same ceremony. Each yields a
    /// 32-byte output keyed by name in [`WalletSetup::extra_seeds`].
    /// On platforms that pair `prf.eval.first` + `prf.eval.second` per
    /// ceremony, N extra salts cost ⌈(2 + N) / 2⌉ user prompts.
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub extra_salts: Vec<NamedSalt>,

    /// Forwarded to
    /// [`crate::passkey::DeriveSeedsRequest::allow_credential_ids`].
    /// Lets server-driven flows pass a discovery-provided allow-list
    /// per ceremony instead of pinning it on the provider instance.
    /// Empty (default) preserves the historical behavior.
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub allow_credential_ids: Vec<Vec<u8>>,

    /// Forwarded to
    /// [`crate::passkey::DeriveSeedsRequest::prefer_immediately_available_credentials`].
    /// `None` (default) keeps the provider's existing behavior.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub prefer_immediately_available_credentials: Option<bool>,
}

/// Response from [`crate::passkey::Passkey::setup_wallet`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WalletSetup {
    pub wallet: Wallet,
    /// 32 bytes per [`NamedSalt`] in the request, keyed by name.
    pub extra_seeds: HashMap<String, Vec<u8>>,
}

/// Per-call overrides for [`crate::passkey::PrfProvider::create_passkey`].
/// All fields optional; missing values fall back to the provider's
/// configured defaults.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CreatePasskeyRequest {
    /// Credential IDs the authenticator must refuse to duplicate.
    /// Surfaces as `PrfProviderError::CredentialAlreadyExists` when
    /// any entry matches a credential already on the device.
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub exclude_credential_ids: Vec<Vec<u8>>,

    /// Override for `user.name`. Defaults to the provider's `user_name`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_name: Option<String>,

    /// Override for `user.displayName`. Defaults to the provider's `user_display_name`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_display_name: Option<String>,
}

/// Result of a successful [`crate::passkey::PrfProvider::create_passkey`].
///
/// `user_id` is the `WebAuthn` `user.id` (user handle) the provider
/// generated for this credential. Always returned so hosts can
/// correlate server-side; never host-supplied (see the SDK's
/// passkey guide for the rationale).
///
/// `aaguid` (16-byte authenticator identifier) and `backup_eligible`
/// (BE flag) are parsed from authenticator data when available; they
/// are `None` on platforms that don't expose attestation data (e.g.
/// Safari without `getAuthenticatorData()`). Use only as display
/// hints, never for trust decisions: AAGUID is unverified attestation.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisteredCredential {
    pub credential_id: Vec<u8>,
    /// The `WebAuthn` `user.id` (user handle) the provider generated
    /// for this credential. 1-64 bytes per spec.
    pub user_id: Vec<u8>,
    pub aaguid: Option<Vec<u8>>,
    pub backup_eligible: Option<bool>,
}

/// Optional configuration for [`crate::passkey::PasskeyClient::new`].
///
/// Replaces the legacy bare `NostrRelayConfig` constructor parameter.
/// Only fields that are not provider-scoped belong here; all other
/// knobs (`rp_id`, `auto_register`, `credential_registry`, etc.) live
/// on the platform `PasskeyProvider` constructor.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PasskeyConfig {
    /// Breez API key for the authenticated Breez Nostr relay (NIP-42).
    /// `None` ⇒ public relays only (label sync still works, less robust).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub breez_api_key: Option<String>,
    /// Wallet label used when `register` / `sign_in` receive
    /// `label = None`. `None` ⇒ internal `DEFAULT_LABEL` (`"Default"`).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub default_label: Option<String>,
}
