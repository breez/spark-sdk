use serde::{Deserialize, Serialize};

/// Configuration for Nostr relay connections used in seedless restore.
///
/// Relay URLs are managed internally by the client:
/// - Public relays are always included
/// - Breez relay is added when `breez_api_key` is provided (enables NIP-42 auth)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NostrRelayConfig {
    /// Optional Breez API key for authenticated access to the Breez relay.
    /// When provided, the Breez relay is added and NIP-42 authentication is enabled.
    pub breez_api_key: Option<String>,
    /// Connection timeout in seconds (default: 30)
    pub timeout_secs: u32,
}

impl Default for NostrRelayConfig {
    fn default() -> Self {
        Self {
            breez_api_key: None,
            timeout_secs: 30,
        }
    }
}
