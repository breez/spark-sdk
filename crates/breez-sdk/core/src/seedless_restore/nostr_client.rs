use std::time::Duration;

use nostr::Filter;
use nostr_sdk::Client;

use super::derivation::derive_nip42_keypair;
use super::error::SeedlessRestoreError;
use super::models::NostrRelayConfig;

/// Default public Nostr relays for salt storage.
const DEFAULT_PUBLIC_RELAYS: &[&str] = &[
    "wss://relay.nostr.watch",
    "wss://relaypag.es",
    "wss://monitorlizard.nostr1.com",
    "wss://relay.damus.io",
    "wss://relay.nostr.band",
    "wss://relay.primal.net",
];

/// Breez-operated relay URL (requires NIP-42 authentication).
const BREEZ_RELAY: &str = "wss://relay.breez.technology";

/// Client for publishing and discovering salts on Nostr relays.
///
/// Salts are stored as kind-1 (text note) events with plain text content.
/// The Nostr identity is derived from the passkey PRF at account 55.
///
/// Relay URLs are managed internally:
/// - Public relays are always included for redundancy
/// - Breez relay is added when an API key is configured (enables NIP-42 auth)
pub struct NostrSaltClient {
    config: NostrRelayConfig,
}

impl NostrSaltClient {
    /// Create a new Nostr salt client with the given relay configuration.
    pub fn new(config: NostrRelayConfig) -> Self {
        Self { config }
    }

    /// Publish a salt to Nostr relays.
    ///
    /// The salt is published as a kind-1 text note event, signed by the provided keys.
    /// Per the seedless-restore spec, the content is plain text (the salt itself).
    ///
    /// # Arguments
    /// * `keys` - The Nostr keypair derived from the account master
    /// * `salt` - The salt string to publish
    ///
    /// # Returns
    /// * `Ok(())` - Salt was published successfully
    /// * `Err(SeedlessRestoreError)` - Publication failed
    pub async fn publish_salt(
        &self,
        keys: &nostr::Keys,
        salt: &str,
    ) -> Result<(), SeedlessRestoreError> {
        let client = self.create_client(keys).await?;

        // Create a text note event with the salt as content
        let event_builder = nostr::EventBuilder::text_note(salt);

        // Build and sign the event
        let event = event_builder.sign_with_keys(keys).map_err(|e| {
            SeedlessRestoreError::SaltPublishFailed(format!("Failed to sign event: {e}"))
        })?;

        // Send the event to all connected relays
        client
            .send_event(&event)
            .await
            .map_err(|e| SeedlessRestoreError::SaltPublishFailed(e.to_string()))?;

        // Disconnect from relays
        client.disconnect().await;

        Ok(())
    }

    /// Query all salts published by the given Nostr identity.
    ///
    /// Returns all kind-1 text note events authored by the pubkey.
    /// The salt values are extracted from the event content.
    ///
    /// # Arguments
    /// * `keys` - The Nostr keypair (used to identify the pubkey to query)
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of salts found
    /// * `Err(SeedlessRestoreError)` - Query failed
    pub async fn query_salts(
        &self,
        keys: &nostr::Keys,
    ) -> Result<Vec<String>, SeedlessRestoreError> {
        let client = self.create_client(keys).await?;

        // Query for all text notes from this pubkey
        let filter = Filter::new()
            .author(keys.public_key())
            .kind(nostr::Kind::TextNote);

        let timeout = Duration::from_secs(u64::from(self.config.timeout_secs));

        let events = client
            .fetch_events(filter, timeout)
            .await
            .map_err(|e| SeedlessRestoreError::SaltQueryFailed(e.to_string()))?;

        // Disconnect from relays
        client.disconnect().await;

        // Extract salt content from events
        let salts: Vec<String> = events
            .into_iter()
            .map(|event| event.content.clone())
            .collect();

        Ok(salts)
    }

    /// Check if a specific salt has already been published.
    ///
    /// # Arguments
    /// * `keys` - The Nostr keypair
    /// * `salt` - The salt to check for
    ///
    /// # Returns
    /// * `Ok(true)` - Salt already exists
    /// * `Ok(false)` - Salt not found
    /// * `Err(SeedlessRestoreError)` - Query failed
    pub async fn salt_exists(
        &self,
        keys: &nostr::Keys,
        salt: &str,
    ) -> Result<bool, SeedlessRestoreError> {
        let salts = self.query_salts(keys).await?;
        Ok(salts.iter().any(|s| s == salt))
    }

    /// Create and connect a Nostr client to the configured relays.
    ///
    /// When an API key is configured, the client uses API key-derived keys for NIP-42
    /// authentication with the Breez relay. Content events (salts) are signed manually
    /// with passkey-derived keys via `sign_with_keys()`, so they use the correct identity.
    ///
    /// Tolerates individual relay failures — only errors if no relays could be added.
    async fn create_client(&self, keys: &nostr::Keys) -> Result<Client, SeedlessRestoreError> {
        let client = if let Some(ref api_key) = self.config.breez_api_key {
            // Derive auth keys from API key for NIP-42 authentication
            // Content events are signed manually with passkey keys via sign_with_keys()
            let auth_keys = derive_nip42_keypair(api_key)?;
            Client::new(auth_keys)
        } else {
            // No API key - use passkey keys directly
            Client::new(keys.clone())
        };

        // Add default public relays
        let mut added = 0usize;
        for relay_url in DEFAULT_PUBLIC_RELAYS {
            match client.add_relay(*relay_url).await {
                #[allow(clippy::arithmetic_side_effects)]
                Ok(_) => added += 1,
                Err(e) => {
                    tracing::warn!("Failed to add relay {relay_url}: {e}");
                }
            }
        }

        // Add Breez relay if API key is configured
        if self.config.breez_api_key.is_some() {
            match client.add_relay(BREEZ_RELAY).await {
                #[allow(clippy::arithmetic_side_effects)]
                Ok(_) => {
                    added += 1;
                    tracing::info!("Added Breez relay with NIP-42 authentication");
                }
                Err(e) => {
                    // Log warning but continue - fall back to public relays
                    tracing::warn!(
                        "Failed to add Breez relay (continuing with public relays): {e}"
                    );
                }
            }
        }

        if added == 0 {
            return Err(SeedlessRestoreError::RelayConnectionFailed(
                "failed to add any relay".to_string(),
            ));
        }

        // Connect to relays
        client.connect().await;

        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nostr_salt_client_new_default() {
        let config = NostrRelayConfig::default();
        let client = NostrSaltClient::new(config);

        assert!(client.config.breez_api_key.is_none());
        assert_eq!(client.config.timeout_secs, 30);
    }

    #[test]
    fn test_nostr_salt_client_with_api_key() {
        let config = NostrRelayConfig {
            breez_api_key: Some("dGVzdC1hcGkta2V5".to_string()),
            ..Default::default()
        };
        let client = NostrSaltClient::new(config);

        assert!(client.config.breez_api_key.is_some());
        assert_eq!(client.config.timeout_secs, 30);
    }
}
