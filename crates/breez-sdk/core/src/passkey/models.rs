use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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
    /// User-chosen label. Defaults to `"Default"` when `None`.
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

    /// Override for the `WebAuthn` `user.id` field. Must be 1-64 bytes
    /// per spec; rejected otherwise. Always randomize per call:
    /// reusing a `user_id` across creates on the same `rp_id` causes
    /// some authenticators (Apple Passwords) to silently destroy the
    /// existing credential. Default is a fresh 16 random bytes.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_id: Option<Vec<u8>>,

    /// Override for `user.name`. Defaults to the provider's `user_name`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_name: Option<String>,

    /// Override for `user.displayName`. Defaults to the provider's `user_display_name`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_display_name: Option<String>,
}

/// Result of a successful [`crate::passkey::PrfProvider::create_passkey`].
/// `aaguid` (16-byte authenticator identifier) and `backup_eligible`
/// (BE flag) are parsed from authenticator data when available; they
/// are `None` on platforms that don't expose attestation data (e.g.
/// Safari without `getAuthenticatorData()`). Use only as display
/// hints, never for trust decisions: AAGUID is unverified attestation.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisteredCredential {
    pub credential_id: Vec<u8>,
    pub aaguid: Option<Vec<u8>>,
    pub backup_eligible: Option<bool>,
}

/// Configuration for Nostr relay connections used in `Passkey`.
///
/// Relay URLs are managed internally by the client:
/// - Public relays are always included
/// - Breez relay is added when `breez_api_key` is provided (enables NIP-42 auth)
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NostrRelayConfig {
    /// Optional Breez API key for authenticated access to the Breez relay.
    /// When provided, the Breez relay is added and NIP-42 authentication is enabled.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub breez_api_key: Option<String>,
}
