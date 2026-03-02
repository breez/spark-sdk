use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bitcoin::hashes::{Hash, sha256};
use clap::Parser;
use nostr::RelayUrl;
use nostr::nips::nip65;
use nostr_sdk::Client;
use tracing::{info, warn};

/// Breez-operated relay URL (requires NIP-42 authentication).
const BREEZ_RELAY: &str = "wss://nr1.breez.technology";

/// Publish a NIP-65 relay list for the Breez identity.
///
/// This tool signs and broadcasts a kind-10002 (NIP-65 relay list metadata)
/// event to the Breez relay and all specified recommended relays.
#[derive(Parser)]
#[command(name = "nip65-publisher")]
struct Cli {
    /// Breez identity private key (hex or nsec1 bech32).
    /// Signs the NIP-65 event.
    #[arg(long, env = "NIP65_PRIVATE_KEY")]
    private_key: String,

    /// Breez API key (base64-encoded) for NIP-42 authentication with the
    /// Breez relay.
    #[arg(long, env = "NIP65_API_KEY")]
    api_key: String,

    /// Recommended relay URLs to include in the NIP-65 list.
    /// Can be specified multiple times. The Breez relay is always included
    /// first automatically.
    #[arg(long = "relay")]
    relays: Vec<String>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // Load .env file if present (before clap parses env vars)
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    // Parse the identity keys (used to sign the NIP-65 event)
    let signing_keys = nostr::Keys::parse(&cli.private_key)
        .context("invalid private key (expected 32-byte hex or nsec1 bech32)")?;

    info!(
        pubkey = %signing_keys.public_key(),
        "Identity"
    );

    // Derive NIP-42 auth keys from the API key: sha256(base64_decode(api_key))
    let auth_keys = derive_nip42_keypair(&cli.api_key)
        .context("failed to derive NIP-42 keypair from API key")?;

    // Build the relay list: Breez relay first, then user-specified relays
    let mut relay_urls: Vec<String> = Vec::with_capacity(cli.relays.len().saturating_add(1));
    relay_urls.push(BREEZ_RELAY.to_string());
    for url in &cli.relays {
        if url != BREEZ_RELAY {
            relay_urls.push(url.clone());
        }
    }

    // Build the NIP-65 event
    let relay_entries: Vec<(RelayUrl, Option<nip65::RelayMetadata>)> = relay_urls
        .iter()
        .filter_map(|url| {
            RelayUrl::parse(url)
                .inspect_err(|e| warn!("skipping invalid relay URL {url}: {e}"))
                .ok()
                .map(|r| (r, None))
        })
        .collect();

    if relay_entries.is_empty() {
        bail!("no valid relay URLs to publish");
    }

    let event = nostr::EventBuilder::relay_list(relay_entries)
        .sign_with_keys(&signing_keys)
        .context("failed to sign NIP-65 event")?;

    info!(
        "Publishing NIP-65 event id {} to {} relay(s)...",
        event.id,
        relay_urls.len()
    );

    // Create the client with NIP-42 auth keys (for Breez relay authentication)
    let client = Client::new(auth_keys);

    let mut added = 0usize;
    for url in &relay_urls {
        match client.add_relay(url.as_str()).await {
            #[allow(clippy::arithmetic_side_effects)]
            Ok(_) => added += 1,
            Err(e) => warn!("failed to add relay {url}: {e}"),
        }
    }

    if added == 0 {
        bail!("failed to add any relays");
    }

    client.connect().await;

    // Wait for relay connections and NIP-42 auth handshake to complete
    info!("Waiting for relay connections and NIP-42 authentication...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let output = client
        .send_event(&event)
        .await
        .context("failed to send NIP-65 event")?;

    info!(
        success = output.success.len(),
        failed = output.failed.len(),
        "Published NIP-65 event id {}",
        output.id()
    );

    for url in &output.success {
        info!("  success: {url}");
    }
    for (url, err) in &output.failed {
        warn!("  failed:  {url} — {err:?}");
    }

    client.disconnect().await;

    // Wait briefly for relay connections to flush
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(())
}

/// Derive a Nostr keypair for NIP-42 authentication from a Breez API key.
///
/// Derivation: `sha256(base64_decode(api_key))` → 32-byte secret key → Nostr keypair.
fn derive_nip42_keypair(api_key: &str) -> Result<nostr::Keys> {
    let decoded = STANDARD
        .decode(api_key)
        .context("API key is not valid base64")?;

    let hash = sha256::Hash::hash(&decoded);

    let secret_key = nostr::SecretKey::from_slice(hash.as_byte_array())
        .context("failed to create secret key from hash")?;

    Ok(nostr::Keys::new(secret_key))
}
