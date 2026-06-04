use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use nostr::nips::nip65;
use nostr::{Event, Filter, Kind, PublicKey, RelayUrl};
use nostr_sdk::Client;
use platform_utils::tokio;
use tracing::{info, warn};

use super::derivation::derive_nip42_keypair;
use super::error::PasskeyError;

/// Public relays used as fallback when NIP-65 lists cannot be fetched.
/// The first entry doubles as the preferred read relay for non-API-key users.
const STATIC_RELAYS: &[&str] = &[
    "wss://relay.primal.net",
    "wss://relay.damus.io",
    "wss://relay.nostr.watch",
    "wss://relaypag.es",
    "wss://monitorlizard.nostr1.com",
];

/// Per-relay-batch read/write timeout. Picked to outlast a slow public
/// relay's TLS handshake without keeping a stalled batch alive long
/// enough to block the surrounding flow.
const RELAY_TIMEOUT_SECS: u64 = 30;

/// Breez-operated relay URL (requires NIP-42 authentication).
const BREEZ_RELAY: &str = "wss://nr1.breez.technology";

/// Hex-encoded public key of the well-known Breez identity that publishes
/// the authoritative NIP-65 relay list.
const BREEZ_NIP65_PUBKEY: &str = "0478caf9d25260b7603154c4227d4af5c2e4937092fbdbc9958aef9ea8856e23";

/// Sole concrete, internal label store for the passkey orchestrator.
/// Owns the full `nostr::Keys` derived from the passkey's account-master
/// PRF output plus the optional Breez API key used to authenticate with
/// the Breez relay (NIP-42).
///
/// Labels are stored as kind-1 (text note) events with plain text content.
///
/// Relay URLs are managed internally:
/// - Public relays are always included for redundancy
/// - Breez relay is added when an API key is configured (enables NIP-42 auth)
#[derive(Clone)]
pub struct NostrSaltClient {
    keys: nostr::Keys,
    breez_api_key: Option<String>,
    /// Flag ensuring the NIP-65 relay sync is only spawned once per client lifetime.
    relay_sync_triggered: Arc<AtomicBool>,
    /// Server-provided relay list, set once by the relay sync task.
    server_relays: Arc<OnceLock<Vec<String>>>,
}

impl NostrSaltClient {
    /// Create a new Nostr salt client owning the passkey-derived
    /// signing keys and an optional Breez API key.
    pub fn new(keys: nostr::Keys, breez_api_key: Option<String>) -> Self {
        Self {
            keys,
            breez_api_key,
            relay_sync_triggered: Arc::new(AtomicBool::new(false)),
            server_relays: Arc::new(OnceLock::new()),
        }
    }

    /// Query all labels published by the owned identity.
    ///
    /// Returns all kind-1 text note events authored by the pubkey.
    /// The label values are extracted from the event content.
    ///
    /// On the first call, spawns a background task to sync the NIP-65 relay list
    /// with the breez server's authoritative list. This does not block the response.
    pub async fn list_labels(&self) -> Result<Vec<String>, PasskeyError> {
        let filter = Filter::new()
            .author(self.keys.public_key())
            .kind(nostr::Kind::TextNote);

        let events_vec = self.read_events(filter).await?;

        // Extract label content from events
        let labels: Vec<String> = events_vec
            .iter()
            .map(|event| event.content.clone())
            .collect();

        // Trigger one-time NIP-65 relay sync in the background
        self.spawn_relay_sync(events_vec);

        Ok(labels)
    }

    /// Idempotently ensure `label` is published for the owned identity.
    /// Writes the event to every reachable relay batch, not just the
    /// first: the read path early-exits on the first batch that responds
    /// (even empty), so a single-batch write could be missed after a
    /// relay flap. Skips batches that already carry the label and avoids
    /// the cold NIP-65 fetch `create_write_client` would otherwise trigger.
    ///
    /// Triggers the one-time background NIP-65 relay sync (same as
    /// `list_labels`), keyed off the first batch's events.
    pub async fn store_label(&self, label: &str) -> Result<(), PasskeyError> {
        let relays = self.read_relay_candidates();
        let timeout = Duration::from_secs(RELAY_TIMEOUT_SECS);
        let filter = Filter::new()
            .author(self.keys.public_key())
            .kind(nostr::Kind::TextNote);

        // Sign once and broadcast the same event to every batch missing the
        // label, so all relays converge on a single event id.
        let event = nostr::EventBuilder::text_note(label)
            .sign_with_keys(&self.keys)
            .map_err(|e| PasskeyError::NostrWriteFailed(format!("Failed to sign event: {e}")))?;

        let mut last_err: Option<String> = None;
        let mut any_stored = false;
        let mut sync_events: Option<Vec<Event>> = None;

        for chunk in relays.chunks(2) {
            let client = self.new_client()?;
            let mut added = 0usize;
            for relay_url in chunk {
                match client.add_relay(relay_url.as_str()).await {
                    #[allow(clippy::arithmetic_side_effects)]
                    Ok(_) => added += 1,
                    Err(e) => {
                        warn!("Failed to add relay {relay_url}: {e}");
                        last_err = Some(e.to_string());
                    }
                }
            }
            if added == 0 {
                continue;
            }
            client.connect().await;

            let events = match client.fetch_events(filter.clone(), timeout).await {
                Ok(events) => events,
                Err(e) => {
                    client.disconnect().await;
                    warn!("Failed to fetch events from relay batch: {e}");
                    last_err = Some(e.to_string());
                    continue;
                }
            };
            let events_vec: Vec<Event> = events.into_iter().collect();

            if events_vec.iter().any(|e| e.content == label) {
                any_stored = true;
            } else if let Err(e) = client.send_event(&event).await {
                warn!("Failed to write label to relay batch: {e}");
                last_err = Some(e.to_string());
            } else {
                any_stored = true;
            }

            // Seed the one-time NIP-65 sync from the first batch we read.
            if sync_events.is_none() {
                sync_events = Some(events_vec);
            }
            client.disconnect().await;
        }

        if let Some(events) = sync_events {
            self.spawn_relay_sync(events);
        }

        if any_stored {
            Ok(())
        } else {
            Err(PasskeyError::NostrReadFailed(
                last_err.unwrap_or_else(|| "no relays available".to_string()),
            ))
        }
    }

    /// Spawn the NIP-65 relay sync task if it hasn't been triggered yet.
    fn spawn_relay_sync(&self, events: Vec<Event>) {
        if self
            .relay_sync_triggered
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let client = self.clone();

        tokio::spawn(async move {
            if let Err(e) = client.sync_relay_list(events).await {
                warn!("NIP-65 relay sync failed: {e}");
                client.relay_sync_triggered.store(false, Ordering::Release);
            }
        });
    }

    /// Ensure the server relay list is populated, fetching it if necessary.
    async fn ensure_server_relays(&self) -> Vec<String> {
        if let Some(relays) = self.server_relays.get() {
            return relays.clone();
        }

        let relays = self.fetch_server_relay_list().await;

        if let Err(existing) = self.server_relays.set(relays.clone()) {
            warn!("Server relay list already set: {existing:?}");
        }
        relays
    }

    /// Synchronize the NIP-65 relay list with the breez server's authoritative list.
    async fn sync_relay_list(&self, existing_events: Vec<Event>) -> Result<(), PasskeyError> {
        let server_relays = self.ensure_server_relays().await;

        // Query the currently published NIP-65 relay list
        let published_relays = self.query_nip65_relay_list().await?;

        let needs_update = match &published_relays {
            None => true,
            Some(published) => {
                let server_set: HashSet<&str> = server_relays.iter().map(String::as_str).collect();
                let published_set: HashSet<&str> = published.iter().map(String::as_str).collect();
                server_set != published_set
            }
        };

        if needs_update {
            info!(
                "NIP-65 relay list mismatch detected. Re-publishing {} events to {} relays.",
                existing_events.len(),
                server_relays.len()
            );

            // Re-publish all existing events to the new relay set
            if !existing_events.is_empty() {
                self.republish_events_to_relays(&existing_events).await?;
            }

            // Publish updated NIP-65 event
            self.publish_nip65_relay_list(&server_relays).await?;

            info!("NIP-65 relay list sync completed successfully.");
        }

        Ok(())
    }

    /// Fetch the recommended relay list from the Breez NIP-65 event.
    async fn fetch_breez_nip65(&self) -> Option<Vec<String>> {
        let breez_pubkey = match PublicKey::from_hex(BREEZ_NIP65_PUBKEY) {
            Ok(pk) => pk,
            Err(e) => {
                warn!("Invalid Breez NIP-65 pubkey constant: {e}");
                return None;
            }
        };

        let filter = Filter::new()
            .author(breez_pubkey)
            .kind(Kind::RelayList)
            .limit(1);

        let relays = self.read_relay_candidates();

        let events = match self.fetch_events_with_fallback(&relays, filter).await {
            Ok(events) => events,
            Err(e) => {
                warn!("Failed to fetch Breez NIP-65 event: {e}");
                return None;
            }
        };

        let event = events.into_iter().next()?;

        let relay_urls: Vec<String> = nip65::extract_relay_list(&event)
            .map(|(url, _metadata)| url.to_string())
            .collect();

        if relay_urls.is_empty() {
            None
        } else {
            Some(relay_urls)
        }
    }

    /// Fetch the authoritative relay list.
    async fn fetch_server_relay_list(&self) -> Vec<String> {
        if let Some(relays) = self.fetch_breez_nip65().await {
            info!("Fetched {} relays from Breez NIP-65 event", relays.len());
            return relays;
        }

        // Try the user's own published NIP-65 list before falling back to static
        match self.query_nip65_relay_list().await {
            Ok(Some(relays)) => {
                info!(
                    "Using user's published NIP-65 relay list ({} relays)",
                    relays.len()
                );
                return relays;
            }
            Ok(None) => {}
            Err(e) => warn!("Failed to query user NIP-65 relay list: {e}"),
        }

        info!("Falling back to static relay list");
        let mut relays: Vec<String> = Vec::new();
        if self.breez_api_key.is_some() {
            relays.push(BREEZ_RELAY.to_string());
        }
        relays.extend(STATIC_RELAYS.iter().map(|s| (*s).to_string()));
        relays
    }

    /// Query the user's published NIP-65 relay list from read relays.
    async fn query_nip65_relay_list(&self) -> Result<Option<Vec<String>>, PasskeyError> {
        let filter = Filter::new()
            .author(self.keys.public_key())
            .kind(Kind::RelayList)
            .limit(1);

        let events = self.read_events(filter).await?;

        // NIP-65 is a replaceable event, so there should be at most one
        let Some(event) = events.into_iter().next() else {
            return Ok(None);
        };

        let relay_urls: Vec<String> = nip65::extract_relay_list(&event)
            .map(|(url, _metadata)| url.to_string())
            .collect();

        if relay_urls.is_empty() {
            Ok(None)
        } else {
            Ok(Some(relay_urls))
        }
    }

    /// Publish a NIP-65 relay list metadata event.
    async fn publish_nip65_relay_list(&self, relay_urls: &[String]) -> Result<(), PasskeyError> {
        let relay_entries: Vec<(RelayUrl, Option<nip65::RelayMetadata>)> = relay_urls
            .iter()
            .filter_map(|url| RelayUrl::parse(url).ok().map(|r| (r, None)))
            .collect();

        let builder = nostr::EventBuilder::relay_list(relay_entries);
        self.write_event(builder).await
    }

    /// Re-publish existing label events to the current write relay set.
    async fn republish_events_to_relays(&self, events: &[Event]) -> Result<(), PasskeyError> {
        let client = self.create_write_client().await?;

        for event in events {
            if let Err(e) = client.send_event(event).await {
                warn!("Failed to republish event {}: {e}", event.id);
            }
        }

        client.disconnect().await;
        Ok(())
    }

    /// Sign and publish an event to write relays.
    async fn write_event(&self, builder: nostr::EventBuilder) -> Result<(), PasskeyError> {
        let client = self.create_write_client().await?;

        let event = builder
            .sign_with_keys(&self.keys)
            .map_err(|e| PasskeyError::NostrWriteFailed(format!("Failed to sign event: {e}")))?;

        client
            .send_event(&event)
            .await
            .map_err(|e| PasskeyError::NostrWriteFailed(e.to_string()))?;

        client.disconnect().await;
        Ok(())
    }

    /// Build the ordered list of relay candidates for read operations.
    fn read_relay_candidates(&self) -> Vec<String> {
        let mut candidates: Vec<String> = Vec::new();
        if self.breez_api_key.is_some() {
            candidates.push(BREEZ_RELAY.to_string());
        }
        candidates.extend(STATIC_RELAYS.iter().map(|s| (*s).to_string()));
        candidates
    }

    /// Fetch events matching a filter, trying relays in batches of 2 with cascading fallback.
    async fn fetch_events_with_fallback(
        &self,
        relays: &[String],
        filter: Filter,
    ) -> Result<Vec<Event>, PasskeyError> {
        let timeout = Duration::from_secs(RELAY_TIMEOUT_SECS);
        let mut last_err = None;

        for chunk in relays.chunks(2) {
            let client = self.new_client()?;
            let mut added = 0usize;

            for relay_url in chunk {
                match client.add_relay(relay_url.as_str()).await {
                    #[allow(clippy::arithmetic_side_effects)]
                    Ok(_) => added += 1,
                    Err(e) => {
                        warn!("Failed to add relay {relay_url}: {e}");
                        last_err = Some(e.to_string());
                    }
                }
            }

            if added == 0 {
                continue;
            }

            client.connect().await;

            match client.fetch_events(filter.clone(), timeout).await {
                Ok(events) => {
                    client.disconnect().await;
                    return Ok(events.into_iter().collect());
                }
                Err(e) => {
                    client.disconnect().await;
                    warn!("Failed to fetch events from relay batch: {e}");
                    last_err = Some(e.to_string());
                }
            }
        }

        Err(PasskeyError::NostrReadFailed(
            last_err.unwrap_or_else(|| "no relays available".to_string()),
        ))
    }

    /// Fetch events matching a filter from read relays, with cascading fallback.
    async fn read_events(&self, filter: Filter) -> Result<Vec<Event>, PasskeyError> {
        let relays = self.read_relay_candidates();
        self.fetch_events_with_fallback(&relays, filter).await
    }

    /// Create a Nostr client connected to all relays for write operations.
    async fn create_write_client(&self) -> Result<Client, PasskeyError> {
        let client = self.new_client()?;
        let write_relays = self.ensure_server_relays().await;

        let mut added = 0usize;
        for relay_url in &write_relays {
            match client.add_relay(relay_url.as_str()).await {
                #[allow(clippy::arithmetic_side_effects)]
                Ok(_) => added += 1,
                Err(e) => {
                    warn!("Failed to add relay {relay_url}: {e}");
                }
            }
        }

        if added == 0 {
            return Err(PasskeyError::RelayConnectionFailed(
                "failed to add any relay".to_string(),
            ));
        }

        client.connect().await;
        Ok(client)
    }

    /// Create a new Nostr client with the appropriate signing keys.
    ///
    /// When an API key is configured, uses API key-derived keys for NIP-42
    /// authentication. Content events are signed manually with the owned
    /// passkey-derived keys via `sign_with_keys()`.
    fn new_client(&self) -> Result<Client, PasskeyError> {
        Ok(if let Some(ref api_key) = self.breez_api_key {
            let auth_keys = derive_nip42_keypair(api_key)?;
            Client::new(auth_keys)
        } else {
            Client::new(self.keys.clone())
        })
    }
}

/// Internal label store the passkey orchestrator persists wallet labels
/// through. [`NostrSaltClient`] is the production implementation (Nostr
/// relays); tests inject an in-memory double so unit tests never reach
/// the network. Built per-identity from the keys derived in a PRF
/// ceremony (see [`super::Passkey`]'s store builder).
#[macros::async_trait]
pub(crate) trait LabelStore: Send + Sync {
    /// Idempotently publish `label` for the owned identity.
    async fn store_label(&self, label: &str) -> Result<(), PasskeyError>;

    /// List labels published by the owned identity.
    async fn list_labels(&self) -> Result<Vec<String>, PasskeyError>;

    /// The signing identity backing this store. Lets the orchestrator
    /// verify deterministic key derivation across the lazy-init boundary.
    #[cfg(test)]
    fn signing_keys(&self) -> nostr::Keys;
}

#[macros::async_trait]
impl LabelStore for NostrSaltClient {
    async fn store_label(&self, label: &str) -> Result<(), PasskeyError> {
        NostrSaltClient::store_label(self, label).await
    }

    async fn list_labels(&self) -> Result<Vec<String>, PasskeyError> {
        NostrSaltClient::list_labels(self).await
    }

    #[cfg(test)]
    fn signing_keys(&self) -> nostr::Keys {
        self.keys.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn test_keys() -> nostr::Keys {
        nostr::Keys::generate()
    }

    #[macros::test_all]
    fn test_nostr_salt_client_new_default() {
        let client = NostrSaltClient::new(test_keys(), None);

        assert!(client.breez_api_key.is_none());
    }

    #[macros::test_all]
    fn test_nostr_salt_client_with_api_key() {
        let client = NostrSaltClient::new(test_keys(), Some("dGVzdC1hcGkta2V5".to_string()));

        assert!(client.breez_api_key.is_some());
    }

    #[macros::test_all]
    fn test_relay_sync_state_shared_across_clones() {
        let client1 = NostrSaltClient::new(test_keys(), None);
        let client2 = client1.clone();

        assert!(Arc::ptr_eq(
            &client1.relay_sync_triggered,
            &client2.relay_sync_triggered
        ));
        assert!(Arc::ptr_eq(&client1.server_relays, &client2.server_relays));
    }

    #[macros::test_all]
    fn test_breez_nip65_pubkey_is_valid_hex() {
        let result = PublicKey::from_hex(BREEZ_NIP65_PUBKEY);
        assert!(
            result.is_ok(),
            "BREEZ_NIP65_PUBKEY must be a valid hex pubkey"
        );
    }

    #[macros::test_all]
    fn test_static_relays_first_is_preferred_read() {
        assert_eq!(STATIC_RELAYS[0], "wss://relay.primal.net");
    }

    #[macros::test_all]
    fn test_read_relay_candidates_without_api_key() {
        let client = NostrSaltClient::new(test_keys(), None);
        let candidates = client.read_relay_candidates();

        assert_eq!(candidates.len(), STATIC_RELAYS.len());
        assert_eq!(candidates[0], STATIC_RELAYS[0]);
        assert!(!candidates.contains(&BREEZ_RELAY.to_string()));
    }

    #[macros::test_all]
    fn test_read_relay_candidates_with_api_key() {
        let client = NostrSaltClient::new(test_keys(), Some("dGVzdC1hcGkta2V5".to_string()));
        let candidates = client.read_relay_candidates();

        // Breez relay should be first
        assert_eq!(candidates[0], BREEZ_RELAY);
        assert_eq!(candidates.len(), STATIC_RELAYS.len() + 1);
    }
}
