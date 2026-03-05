use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use nostr::nips::nip65;
use nostr::{Event, Filter, Kind, PublicKey, RelayUrl};
use nostr_sdk::Client;
use tokio_with_wasm::alias as tokio;
use tracing::{info, warn};

use super::derivation::derive_nip42_keypair;
use super::error::PasskeyError;
use super::models::NostrRelayConfig;

/// Public relays used as fallback when NIP-65 lists cannot be fetched.
/// The first entry doubles as the preferred read relay for non-API-key users.
const STATIC_RELAYS: &[&str] = &[
    "wss://relay.primal.net",
    "wss://relay.damus.io",
    "wss://relay.nostr.watch",
    "wss://relaypag.es",
    "wss://monitorlizard.nostr1.com",
];

/// Breez-operated relay URL (requires NIP-42 authentication).
const BREEZ_RELAY: &str = "wss://nr1.breez.technology";

/// Hex-encoded public key of the well-known Breez identity that publishes
/// the authoritative NIP-65 relay list.
const BREEZ_NIP65_PUBKEY: &str = "0478caf9d25260b7603154c4227d4af5c2e4937092fbdbc9958aef9ea8856e23";

/// Client for publishing and discovering wallet names on Nostr relays.
///
/// Wallet names are stored as kind-1 (text note) events with plain text content.
/// The Nostr identity is derived from the passkey PRF at account 55.
///
/// Relay URLs are managed internally:
/// - Public relays are always included for redundancy
/// - Breez relay is added when an API key is configured (enables NIP-42 auth)
#[derive(Clone)]
pub struct NostrSaltClient {
    config: NostrRelayConfig,
    /// Flag ensuring the NIP-65 relay sync is only spawned once per client lifetime.
    relay_sync_triggered: Arc<AtomicBool>,
    /// Server-provided relay list, set once by the relay sync task.
    server_relays: Arc<OnceLock<Vec<String>>>,
}

impl NostrSaltClient {
    /// Create a new Nostr salt client with the given relay configuration.
    pub fn new(config: NostrRelayConfig) -> Self {
        Self {
            config,
            relay_sync_triggered: Arc::new(AtomicBool::new(false)),
            server_relays: Arc::new(OnceLock::new()),
        }
    }

    /// Publish a wallet name to Nostr relays.
    ///
    /// The wallet name is published as a kind-1 text note event, signed by the provided keys.
    /// Per the seedless-restore spec, the content is plain text (the wallet name itself).
    ///
    /// # Arguments
    /// * `keys` - The Nostr keypair derived from the account master
    /// * `wallet_name` - The wallet name string to publish
    ///
    /// # Returns
    /// * `Ok(())` - Wallet name was published successfully
    /// * `Err(PasskeyError)` - Publication failed
    pub async fn publish_wallet_name(
        &self,
        keys: &nostr::Keys,
        wallet_name: &str,
    ) -> Result<(), PasskeyError> {
        let builder = nostr::EventBuilder::text_note(wallet_name);
        self.write_event(keys, builder).await
    }

    /// Query all wallet names published by the given Nostr identity.
    ///
    /// Returns all kind-1 text note events authored by the pubkey.
    /// The wallet name values are extracted from the event content.
    ///
    /// On the first call, spawns a background task to sync the NIP-65 relay list
    /// with the breez server's authoritative list. This does not block the response.
    ///
    /// # Arguments
    /// * `keys` - The Nostr keypair (used to identify the pubkey to query)
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of wallet names found
    /// * `Err(PasskeyError)` - Query failed
    pub async fn query_wallet_names(
        &self,
        keys: &nostr::Keys,
    ) -> Result<Vec<String>, PasskeyError> {
        let filter = Filter::new()
            .author(keys.public_key())
            .kind(nostr::Kind::TextNote);

        let events_vec = self.read_events(keys, filter).await?;

        // Extract wallet name content from events
        let wallet_names: Vec<String> = events_vec
            .iter()
            .map(|event| event.content.clone())
            .collect();

        // Trigger one-time NIP-65 relay sync in the background
        self.spawn_relay_sync(keys, events_vec);

        Ok(wallet_names)
    }

    /// Check if a specific wallet name has already been published.
    ///
    /// # Arguments
    /// * `keys` - The Nostr keypair
    /// * `wallet_name` - The wallet name to check for
    ///
    /// # Returns
    /// * `Ok(true)` - Wallet name already exists
    /// * `Ok(false)` - Wallet name not found
    /// * `Err(PasskeyError)` - Query failed
    pub async fn wallet_name_exists(
        &self,
        keys: &nostr::Keys,
        wallet_name: &str,
    ) -> Result<bool, PasskeyError> {
        let wallet_names = self.query_wallet_names(keys).await?;
        Ok(wallet_names.iter().any(|w| w == wallet_name))
    }

    /// Spawn the NIP-65 relay sync task if it hasn't been triggered yet.
    fn spawn_relay_sync(&self, keys: &nostr::Keys, events: Vec<Event>) {
        if self
            .relay_sync_triggered
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let client = self.clone();
        let keys = keys.clone();

        tokio::spawn(async move {
            if let Err(e) = client.sync_relay_list(&keys, events).await {
                warn!("NIP-65 relay sync failed: {e}");
                client.relay_sync_triggered.store(false, Ordering::Release);
            }
        });
    }

    /// Ensure the server relay list is populated, fetching it if necessary.
    ///
    /// Returns the cached list if already set (by a prior call or background sync).
    /// Otherwise fetches from the Breez NIP-65 event (falling back to static relays),
    /// caches the result, and returns it.
    async fn ensure_server_relays(&self, keys: &nostr::Keys) -> Vec<String> {
        if let Some(relays) = self.server_relays.get() {
            return relays.clone();
        }

        let relays = self.fetch_server_relay_list(keys).await;

        if let Err(existing) = self.server_relays.set(relays.clone()) {
            warn!("Server relay list already set: {existing:?}");
        }
        relays
    }

    /// Synchronize the NIP-65 relay list with the breez server's authoritative list.
    ///
    /// 1. Fetches the relay list (populating `server_relays` if not already set).
    /// 2. Queries the user's published NIP-65 event.
    /// 3. If they differ, re-publishes all wallet name events and updates the NIP-65 event.
    async fn sync_relay_list(
        &self,
        keys: &nostr::Keys,
        existing_events: Vec<Event>,
    ) -> Result<(), PasskeyError> {
        let server_relays = self.ensure_server_relays(keys).await;

        // Query the currently published NIP-65 relay list
        let published_relays = self.query_nip65_relay_list(keys).await?;

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
                self.republish_events_to_relays(keys, &existing_events)
                    .await?;
            }

            // Publish updated NIP-65 event
            self.publish_nip65_relay_list(keys, &server_relays).await?;

            info!("NIP-65 relay list sync completed successfully.");
        }

        Ok(())
    }

    /// Fetch the recommended relay list from the Breez NIP-65 event.
    ///
    /// Queries relays (Breez first if API key set, then static) for the well-known
    /// Breez pubkey's kind-10002 event and extracts the relay URLs.
    ///
    /// Returns `None` on any failure (logged as warnings); the caller falls back
    /// to the static relay list.
    async fn fetch_breez_nip65(&self, keys: &nostr::Keys) -> Option<Vec<String>> {
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

        let events = match self.fetch_events_with_fallback(keys, &relays, filter).await {
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
    ///
    /// Fallback chain: Breez NIP-65 → user's own published NIP-65 → static list.
    /// Using the user's NIP-65 as a middle fallback avoids overwriting a previously
    /// synced relay list with the static list when the Breez NIP-65 is temporarily
    /// unavailable.
    async fn fetch_server_relay_list(&self, keys: &nostr::Keys) -> Vec<String> {
        if let Some(relays) = self.fetch_breez_nip65(keys).await {
            info!("Fetched {} relays from Breez NIP-65 event", relays.len());
            return relays;
        }

        // Try the user's own published NIP-65 list before falling back to static
        match self.query_nip65_relay_list(keys).await {
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
        if self.config.breez_api_key.is_some() {
            relays.push(BREEZ_RELAY.to_string());
        }
        relays.extend(STATIC_RELAYS.iter().map(|s| (*s).to_string()));
        relays
    }

    /// Query the user's published NIP-65 relay list from read relays.
    ///
    /// Returns the set of relay URLs from the most recent event, or `None` if
    /// no NIP-65 event has been published.
    async fn query_nip65_relay_list(
        &self,
        keys: &nostr::Keys,
    ) -> Result<Option<Vec<String>>, PasskeyError> {
        let filter = Filter::new()
            .author(keys.public_key())
            .kind(Kind::RelayList)
            .limit(1);

        let events = self.read_events(keys, filter).await?;

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
    async fn publish_nip65_relay_list(
        &self,
        keys: &nostr::Keys,
        relay_urls: &[String],
    ) -> Result<(), PasskeyError> {
        let relay_entries: Vec<(RelayUrl, Option<nip65::RelayMetadata>)> = relay_urls
            .iter()
            .filter_map(|url| RelayUrl::parse(url).ok().map(|r| (r, None)))
            .collect();

        let builder = nostr::EventBuilder::relay_list(relay_entries);
        self.write_event(keys, builder).await
    }

    /// Re-publish existing wallet name events to the current write relay set.
    async fn republish_events_to_relays(
        &self,
        keys: &nostr::Keys,
        events: &[Event],
    ) -> Result<(), PasskeyError> {
        let client = self.create_write_client(keys).await?;

        for event in events {
            if let Err(e) = client.send_event(event).await {
                warn!("Failed to republish event {}: {e}", event.id);
            }
        }

        client.disconnect().await;
        Ok(())
    }

    /// Sign and publish an event to write relays.
    async fn write_event(
        &self,
        keys: &nostr::Keys,
        builder: nostr::EventBuilder,
    ) -> Result<(), PasskeyError> {
        let client = self.create_write_client(keys).await?;

        let event = builder
            .sign_with_keys(keys)
            .map_err(|e| PasskeyError::NostrWriteFailed(format!("Failed to sign event: {e}")))?;

        client
            .send_event(&event)
            .await
            .map_err(|e| PasskeyError::NostrWriteFailed(e.to_string()))?;

        client.disconnect().await;
        Ok(())
    }

    /// Build the ordered list of relay candidates for read operations.
    ///
    /// Priority: Breez relay (if API key set), then static relays.
    fn read_relay_candidates(&self) -> Vec<String> {
        let mut candidates: Vec<String> = Vec::new();
        if self.config.breez_api_key.is_some() {
            candidates.push(BREEZ_RELAY.to_string());
        }
        candidates.extend(STATIC_RELAYS.iter().map(|s| (*s).to_string()));
        candidates
    }

    /// Fetch events matching a filter, trying relays in batches of 2 with cascading fallback.
    ///
    /// Queries relays in pairs for redundancy: if one relay is missing events the other
    /// may have them. The nostr-sdk `Client` fans out queries to all connected relays
    /// and merges results internally.
    ///
    /// On batch failure (all relays in the batch error), tries the next batch.
    /// On success (even empty results), returns immediately.
    /// Errors only if all batches fail.
    async fn fetch_events_with_fallback(
        &self,
        keys: &nostr::Keys,
        relays: &[String],
        filter: Filter,
    ) -> Result<Vec<Event>, PasskeyError> {
        let timeout = Duration::from_secs(u64::from(self.config.timeout_secs()));
        let mut last_err = None;

        for chunk in relays.chunks(2) {
            let client = self.new_client(keys)?;
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
    ///
    /// Tries Breez relay first (if API key set), then static relays one at a time.
    /// Falls through to the next relay only on connection or fetch errors.
    async fn read_events(
        &self,
        keys: &nostr::Keys,
        filter: Filter,
    ) -> Result<Vec<Event>, PasskeyError> {
        let relays = self.read_relay_candidates();
        self.fetch_events_with_fallback(keys, &relays, filter).await
    }

    /// Create a Nostr client connected to all relays for write operations.
    ///
    /// Ensures `server_relays` is populated before writing (fetches if needed).
    /// Tolerates individual relay failures — only errors if no relays could be added.
    async fn create_write_client(&self, keys: &nostr::Keys) -> Result<Client, PasskeyError> {
        let client = self.new_client(keys)?;
        let write_relays = self.ensure_server_relays(keys).await;

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
    /// authentication. Content events are signed manually with passkey-derived
    /// keys via `sign_with_keys()`.
    fn new_client(&self, keys: &nostr::Keys) -> Result<Client, PasskeyError> {
        Ok(if let Some(ref api_key) = self.config.breez_api_key {
            let auth_keys = derive_nip42_keypair(api_key)?;
            Client::new(auth_keys)
        } else {
            Client::new(keys.clone())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[macros::test_all]
    fn test_nostr_salt_client_new_default() {
        let config = NostrRelayConfig::default();
        let client = NostrSaltClient::new(config);

        assert!(client.config.breez_api_key.is_none());
        assert_eq!(client.config.timeout_secs(), 30);
    }

    #[macros::test_all]
    fn test_nostr_salt_client_with_api_key() {
        let config = NostrRelayConfig {
            breez_api_key: Some("dGVzdC1hcGkta2V5".to_string()),
            ..Default::default()
        };
        let client = NostrSaltClient::new(config);

        assert!(client.config.breez_api_key.is_some());
        assert_eq!(client.config.timeout_secs(), 30);
    }

    #[macros::test_all]
    fn test_relay_sync_state_shared_across_clones() {
        let config = NostrRelayConfig::default();
        let client1 = NostrSaltClient::new(config);
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
        let config = NostrRelayConfig::default();
        let client = NostrSaltClient::new(config);
        let candidates = client.read_relay_candidates();

        assert_eq!(candidates.len(), STATIC_RELAYS.len());
        assert_eq!(candidates[0], STATIC_RELAYS[0]);
        assert!(!candidates.contains(&BREEZ_RELAY.to_string()));
    }

    #[macros::test_all]
    fn test_read_relay_candidates_with_api_key() {
        let config = NostrRelayConfig {
            breez_api_key: Some("dGVzdC1hcGkta2V5".to_string()),
            ..Default::default()
        };
        let client = NostrSaltClient::new(config);
        let candidates = client.read_relay_candidates();

        // Breez relay should be first
        assert_eq!(candidates[0], BREEZ_RELAY);
        assert_eq!(candidates.len(), STATIC_RELAYS.len() + 1);
    }
}
