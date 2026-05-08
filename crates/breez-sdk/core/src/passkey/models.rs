use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Seed;

const DEFAULT_TIMEOUT_SECS: u32 = 30;

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
}

/// Response from [`crate::passkey::Passkey::setup_wallet`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WalletSetup {
    pub wallet: Wallet,
    /// 32 bytes per [`NamedSalt`] in the request, keyed by name.
    pub extra_seeds: HashMap<String, Vec<u8>>,
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
    /// Connection timeout in seconds. Defaults to 30 when `None`.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub timeout_secs: Option<u32>,
}

impl NostrRelayConfig {
    pub(crate) fn timeout_secs(&self) -> u32 {
        self.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS)
    }
}
