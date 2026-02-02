use std::time::Duration;

use nostr::Filter;
use nostr_sdk::Client;

use super::error::SeedlessRestoreError;
use super::models::NostrRelayConfig;

/// Client for publishing and discovering salts on Nostr relays.
///
/// Salts are stored as kind-1 (text note) events with plain text content.
/// The Nostr identity is derived from the passkey PRF at account 55.
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
    /// Tolerates individual relay failures — only errors if no relays could be added.
    async fn create_client(&self, keys: &nostr::Keys) -> Result<Client, SeedlessRestoreError> {
        let client = Client::new(keys.clone());

        // Add all configured relays, tolerating individual failures
        let mut added = 0usize;
        for relay_url in &self.config.relay_urls {
            match client.add_relay(relay_url).await {
                #[allow(clippy::arithmetic_side_effects)]
                Ok(_) => added += 1,
                Err(e) => {
                    tracing::warn!("Failed to add relay {relay_url}: {e}");
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
    fn test_nostr_salt_client_new() {
        let config = NostrRelayConfig::default();
        let client = NostrSaltClient::new(config.clone());

        assert_eq!(client.config.relay_urls, config.relay_urls);
        assert_eq!(client.config.timeout_secs, config.timeout_secs);
    }

    #[test]
    fn test_nostr_salt_client_breez_relays() {
        let config = NostrRelayConfig::breez_relays();
        let client = NostrSaltClient::new(config);

        assert_eq!(client.config.relay_urls.len(), 1);
        assert!(client.config.relay_urls[0].contains("breez"));
    }
}
