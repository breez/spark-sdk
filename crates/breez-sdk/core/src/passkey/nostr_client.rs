use std::collections::HashSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use bitcoin::secp256k1::{Keypair, XOnlyPublicKey};
use platform_utils::time::Duration;
use platform_utils::tokio;
use tracing::{info, warn};

use super::derivation::derive_nip42_keypair;
use super::error::PasskeyError;
use super::nostr::{
    self, Event, Filter, KIND_RELAY_LIST, KIND_TEXT_NOTE, Relay, RelayOptions, normalize_relay_url,
    relay_list_tags, relay_list_urls,
};

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
/// Owns the full Nostr keypair derived from the passkey's account-master
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
    keys: Keypair,
    breez_api_key: Option<String>,
    /// SOCKS5 proxy carrying every relay connection, when configured.
    proxy: Option<crate::ProxyConfig>,
    /// Flag ensuring the NIP-65 relay sync is only spawned once per client lifetime.
    relay_sync_triggered: Arc<AtomicBool>,
    /// Server-provided relay list, set once by the relay sync task.
    server_relays: Arc<OnceLock<Vec<String>>>,
}

impl NostrSaltClient {
    /// Create a new Nostr salt client owning the passkey-derived
    /// signing keys and an optional Breez API key.
    ///
    /// `proxy` routes every relay connection through a SOCKS5 proxy.
    pub fn new(
        keys: Keypair,
        breez_api_key: Option<String>,
        proxy: Option<crate::ProxyConfig>,
    ) -> Self {
        Self {
            keys,
            breez_api_key,
            proxy,
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
        let filter = Filter::new(self.public_key(), KIND_TEXT_NOTE, None);

        let events_vec = self.read_events(&filter).await?;

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
    /// the cold NIP-65 fetch `connect_write_relays` would otherwise trigger.
    ///
    /// Triggers the one-time background NIP-65 relay sync (same as
    /// `list_labels`), keyed off the first batch's events.
    pub async fn store_label(&self, label: &str) -> Result<(), PasskeyError> {
        let relays = self.read_relay_candidates();
        let timeout = Duration::from_secs(RELAY_TIMEOUT_SECS);
        let filter = Filter::new(self.public_key(), KIND_TEXT_NOTE, None);
        let options = self.relay_options()?;

        // Sign once and broadcast the same event to every batch missing the
        // label, so all relays converge on a single event id.
        let event = Event::sign(&self.keys, KIND_TEXT_NOTE, vec![], label);

        let mut last_err: Option<String> = None;
        let mut any_stored = false;
        let mut sync_events: Option<Vec<Event>> = None;

        for chunk in relays.chunks(2) {
            let (mut answered, events_vec) =
                match nostr::fetch(chunk, &options, &filter, timeout).await {
                    Ok(fetched) => fetched,
                    Err(e) => {
                        warn!("Failed to fetch events from relay batch: {e}");
                        last_err = Some(e);
                        continue;
                    }
                };

            if events_vec.iter().any(|e| e.content == label) {
                any_stored = true;
            } else if let Err(e) = nostr::publish(&mut answered, &event).await {
                warn!("Failed to write label to relay batch: {e}");
                last_err = Some(e);
            } else {
                any_stored = true;
            }

            // Seed the one-time NIP-65 sync from the first batch we read.
            if sync_events.is_none() {
                sync_events = Some(events_vec);
            }
            nostr::close(answered).await;
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
        let breez_pubkey = match XOnlyPublicKey::from_str(BREEZ_NIP65_PUBKEY) {
            Ok(pk) => pk,
            Err(e) => {
                warn!("Invalid Breez NIP-65 pubkey constant: {e}");
                return None;
            }
        };

        let filter = Filter::new(breez_pubkey, KIND_RELAY_LIST, Some(1));

        let relays = self.read_relay_candidates();

        let events = match self.fetch_events_with_fallback(&relays, &filter).await {
            Ok(events) => events,
            Err(e) => {
                warn!("Failed to fetch Breez NIP-65 event: {e}");
                return None;
            }
        };

        let event = events.into_iter().next()?;

        let relay_urls = relay_list_urls(&event);

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
        let filter = Filter::new(self.public_key(), KIND_RELAY_LIST, Some(1));

        let events = self.read_events(&filter).await?;

        // NIP-65 is a replaceable event, so there should be at most one
        let Some(event) = events.into_iter().next() else {
            return Ok(None);
        };

        let relay_urls = relay_list_urls(&event);

        if relay_urls.is_empty() {
            Ok(None)
        } else {
            Ok(Some(relay_urls))
        }
    }

    /// Publish a NIP-65 relay list metadata event.
    async fn publish_nip65_relay_list(&self, relay_urls: &[String]) -> Result<(), PasskeyError> {
        let relays: Vec<String> = relay_urls
            .iter()
            .filter_map(|url| normalize_relay_url(url))
            .collect();

        let event = Event::sign(&self.keys, KIND_RELAY_LIST, relay_list_tags(&relays), "");
        let mut relays = self.connect_write_relays().await?;
        let result = nostr::publish(&mut relays, &event)
            .await
            .map_err(PasskeyError::NostrWriteFailed);
        nostr::close(relays).await;
        result
    }

    /// Re-publish existing label events to the current write relay set.
    async fn republish_events_to_relays(&self, events: &[Event]) -> Result<(), PasskeyError> {
        let mut relays = self.connect_write_relays().await?;

        for event in events {
            if let Err(e) = nostr::publish(&mut relays, event).await {
                warn!("Failed to republish event {}: {e}", event.id);
            }
        }

        nostr::close(relays).await;
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
        filter: &Filter,
    ) -> Result<Vec<Event>, PasskeyError> {
        let timeout = Duration::from_secs(RELAY_TIMEOUT_SECS);
        let options = self.relay_options()?;
        let mut last_err = None;

        for chunk in relays.chunks(2) {
            match nostr::fetch(chunk, &options, filter, timeout).await {
                Ok((answered, events)) => {
                    nostr::close(answered).await;
                    return Ok(events);
                }
                Err(e) => {
                    warn!("Failed to fetch events from relay batch: {e}");
                    last_err = Some(e);
                }
            }
        }

        Err(PasskeyError::NostrReadFailed(
            last_err.unwrap_or_else(|| "no relays available".to_string()),
        ))
    }

    /// Fetch events matching a filter from read relays, with cascading fallback.
    async fn read_events(&self, filter: &Filter) -> Result<Vec<Event>, PasskeyError> {
        let relays = self.read_relay_candidates();
        self.fetch_events_with_fallback(&relays, filter).await
    }

    /// Connect to the write relays, keeping those that could be reached.
    async fn connect_write_relays(&self) -> Result<Vec<Relay>, PasskeyError> {
        let options = self.relay_options()?;
        let write_relays = self.ensure_server_relays().await;
        let timeout = Duration::from_secs(RELAY_TIMEOUT_SECS);
        nostr::connect(&write_relays, &options, timeout)
            .await
            .map_err(PasskeyError::RelayConnectionFailed)
    }

    /// How relays are reached: the NIP-42 keys and the proxy.
    ///
    /// When an API key is configured, uses API key-derived keys for NIP-42
    /// authentication. Content events are signed separately with the owned
    /// passkey-derived keys.
    fn relay_options(&self) -> Result<RelayOptions, PasskeyError> {
        let auth_keys = match &self.breez_api_key {
            Some(api_key) => derive_nip42_keypair(api_key)?,
            None => self.keys,
        };
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        let proxy = self.proxy.as_ref().map(platform_utils::ProxyConfig::from);
        // Relay connections run on browser WebSockets here, which cannot be
        // proxied. Refuse rather than connect directly behind the caller's back.
        #[cfg(all(target_family = "wasm", target_os = "unknown"))]
        let proxy = match self.proxy {
            Some(_) => {
                return Err(PasskeyError::Generic(
                    "a SOCKS5 proxy cannot be honoured on WASM: the browser owns connection setup"
                        .to_string(),
                ));
            }
            None => None,
        };

        Ok(RelayOptions { auth_keys, proxy })
    }

    fn public_key(&self) -> XOnlyPublicKey {
        self.keys.x_only_public_key().0
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
    fn signing_keys(&self) -> Keypair;
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
    fn signing_keys(&self) -> Keypair {
        self.keys
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::rand;

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn test_keys() -> Keypair {
        Keypair::new(
            &bitcoin::secp256k1::Secp256k1::new(),
            &mut rand::thread_rng(),
        )
    }

    #[macros::test_all]
    fn test_nostr_salt_client_new_default() {
        let client = NostrSaltClient::new(test_keys(), None, None);

        assert!(client.breez_api_key.is_none());
    }

    #[macros::test_all]
    fn test_nostr_salt_client_with_api_key() {
        let client = NostrSaltClient::new(test_keys(), Some("dGVzdC1hcGkta2V5".to_string()), None);

        assert!(client.breez_api_key.is_some());
    }

    #[macros::test_all]
    fn test_relay_sync_state_shared_across_clones() {
        let client1 = NostrSaltClient::new(test_keys(), None, None);
        let client2 = client1.clone();

        assert!(Arc::ptr_eq(
            &client1.relay_sync_triggered,
            &client2.relay_sync_triggered
        ));
        assert!(Arc::ptr_eq(&client1.server_relays, &client2.server_relays));
    }

    #[macros::test_all]
    fn test_breez_nip65_pubkey_is_valid_hex() {
        let result = XOnlyPublicKey::from_str(BREEZ_NIP65_PUBKEY);
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
        let client = NostrSaltClient::new(test_keys(), None, None);
        let candidates = client.read_relay_candidates();

        assert_eq!(candidates.len(), STATIC_RELAYS.len());
        assert_eq!(candidates[0], STATIC_RELAYS[0]);
        assert!(!candidates.contains(&BREEZ_RELAY.to_string()));
    }

    #[macros::test_all]
    fn test_read_relay_candidates_with_api_key() {
        let client = NostrSaltClient::new(test_keys(), Some("dGVzdC1hcGkta2V5".to_string()), None);
        let candidates = client.read_relay_candidates();

        // Breez relay should be first
        assert_eq!(candidates[0], BREEZ_RELAY);
        assert_eq!(candidates.len(), STATIC_RELAYS.len() + 1);
    }

    /// Stores and lists a label for a throwaway identity on the live relays,
    /// then publishes its NIP-65 list. With `BREEZ_API_KEY` set, the Breez
    /// relay (NIP-42) is used too.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    #[tokio::test]
    #[ignore = "publishes to the live Nostr relays"]
    async fn test_live_relays_label_roundtrip() {
        let api_key = std::env::var("BREEZ_API_KEY").ok();
        let client = NostrSaltClient::new(test_keys(), api_key, None);
        let label = format!("interop-test-{:08x}", rand::random::<u32>());

        client.store_label(&label).await.unwrap();
        assert!(client.list_labels().await.unwrap().contains(&label));

        let server_relays = client.ensure_server_relays().await;
        assert!(!server_relays.is_empty());
        let filter = Filter::new(client.public_key(), KIND_TEXT_NOTE, None);
        let events = client.read_events(&filter).await.unwrap();
        client.sync_relay_list(events).await.unwrap();
        let published = client.query_nip65_relay_list().await.unwrap().unwrap();
        let published: HashSet<String> = published.into_iter().collect();
        assert_eq!(published, server_relays.into_iter().collect());
    }
}
