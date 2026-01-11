use serde::{Deserialize, Serialize};

/// Configuration for Nostr relay connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NostrRelayConfig {
    /// List of relay URLs to connect to
    pub relay_urls: Vec<String>,
    /// Connection timeout in seconds
    pub timeout_secs: u32,
}

impl Default for NostrRelayConfig {
    fn default() -> Self {
        Self {
            relay_urls: vec![
                "wss://relay.nostr.watch".to_string(),
                "wss://relaypag.es".to_string(),
                "wss://monitorlizard.nostr1.com".to_string(),
                "wss://relay.damus.io".to_string(),
                "wss://relay.nostr.band".to_string(),
                "wss://relay.primal.net".to_string(),
            ],
            timeout_secs: 30,
        }
    }
}

impl NostrRelayConfig {
    /// Create a configuration with Breez-operated relays
    pub fn breez_relays() -> Self {
        Self {
            relay_urls: vec!["wss://relay.breez.technology".to_string()],
            timeout_secs: 30,
        }
    }

    /// Create a custom configuration
    pub fn custom(relay_urls: Vec<String>, timeout_secs: u32) -> Self {
        Self {
            relay_urls,
            timeout_secs,
        }
    }
}
