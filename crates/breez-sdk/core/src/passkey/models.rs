use serde::{Deserialize, Serialize};

use crate::Seed;

const DEFAULT_TIMEOUT_SECS: u32 = 30;

/// A wallet derived from a passkey.
///
/// Contains the derived seed and the wallet name used during derivation.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Wallet {
    /// The derived seed.
    pub seed: Seed,
    /// The wallet name used for derivation (either user-provided or the default).
    pub name: String,
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
