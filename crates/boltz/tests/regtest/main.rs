#![cfg(feature = "regtest")]

mod docker;
mod setup;

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use boltz::api::BoltzApiClient;
use boltz::api::ws::SwapStatusSubscriber;
use boltz::keys::EvmKeyManager;

use self::setup::{regtest_config, regtest_seed};

/// Atomic counter for unique key indices across test runs.
/// Starts from a timestamp-derived offset to avoid collisions with previous runs.
fn next_key_index() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let base = COUNTER.fetch_add(1, Ordering::Relaxed);
    if base == 0 {
        // Initialize offset from current time (truncated to fit u32)
        let offset = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() % 1_000_000) as u32
            * 100;
        COUNTER.store(offset + 1, Ordering::Relaxed);
        return offset;
    }
    base
}

// ─── API Tests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_api_get_pairs() {
    let config = regtest_config();
    let client = BoltzApiClient::new(&config);

    let pairs = client.get_reverse_swap_pairs().await.unwrap();

    // Regtest should have BTC→RBTC pair
    let rbtc = pairs.0.get("BTC").and_then(|m| m.get("RBTC"));
    assert!(rbtc.is_some(), "BTC/RBTC pair not found in regtest");

    let pair = rbtc.unwrap();
    assert!(!pair.hash.is_empty());
    assert!(pair.limits.minimal > 0);
    assert!(pair.limits.maximal > pair.limits.minimal);
    assert!(pair.fees.percentage >= 0.0);
}

#[tokio::test]
async fn test_api_get_contracts() {
    let config = regtest_config();
    let client = BoltzApiClient::new(&config);

    let contracts = client.get_contracts().await.unwrap();

    let has_evm_chain = contracts.0.values().any(|c| {
        !c.swap_contracts.ether_swap.is_empty()
    });
    assert!(has_evm_chain, "Should have at least one EVM chain with EtherSwap");
}

fn create_swap_request(
    config: &boltz::BoltzConfig,
    km: &EvmKeyManager,
    key_index: u32,
    pair_hash: &str,
    invoice_amount: u64,
) -> boltz::api::types::CreateReverseSwapRequest {
    let chain_id = u32::try_from(config.chain_id).unwrap();
    let gas_signer = km.derive_gas_signer(chain_id).unwrap();
    let preimage_hash = km.derive_preimage_hash(chain_id, key_index).unwrap();
    let preimage_key = km.derive_preimage_key(chain_id, key_index).unwrap();

    boltz::api::types::CreateReverseSwapRequest {
        from: "BTC".to_string(),
        to: "RBTC".to_string(),
        preimage_hash: hex::encode(preimage_hash),
        claim_address: gas_signer.address_hex(),
        invoice_amount,
        pair_hash: pair_hash.to_string(),
        referral_id: config.referral_id.clone(),
        claim_public_key: hex::encode(&preimage_key.public_key),
        description: None,
        invoice_expiry: None,
    }
}

#[tokio::test]
async fn test_api_create_reverse_swap() {
    let config = regtest_config();
    let client = BoltzApiClient::new(&config);
    let km = EvmKeyManager::from_seed(&regtest_seed()).unwrap();

    let pairs = client.get_reverse_swap_pairs().await.unwrap();
    let rbtc_pair = &pairs.0["BTC"]["RBTC"];

    let req = create_swap_request(&config, &km, next_key_index(), &rbtc_pair.hash, rbtc_pair.limits.minimal);
    let resp = client.create_reverse_swap(&req).await.unwrap();

    assert!(!resp.id.is_empty());
    assert!(resp.invoice.starts_with("lnbcrt"), "Expected regtest invoice prefix");
    assert!(!resp.lockup_address.is_empty());
    assert!(resp.onchain_amount > 0);
    assert!(resp.timeout_block_height > 0);
}

#[tokio::test]
async fn test_api_get_swap_status() {
    let config = regtest_config();
    let client = BoltzApiClient::new(&config);
    let km = EvmKeyManager::from_seed(&regtest_seed()).unwrap();

    let pairs = client.get_reverse_swap_pairs().await.unwrap();
    let rbtc_pair = &pairs.0["BTC"]["RBTC"];

    let req = create_swap_request(&config, &km, next_key_index(), &rbtc_pair.hash, rbtc_pair.limits.minimal);
    let swap = client.create_reverse_swap(&req).await.unwrap();

    let status = client.get_swap_status(&swap.id).await.unwrap();
    assert_eq!(status.status, "swap.created");
    assert!(status.failure_reason.is_none());
}

// ─── WebSocket Tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_ws_receives_status_updates() {
    let config = regtest_config();
    let client = BoltzApiClient::new(&config);
    let km = EvmKeyManager::from_seed(&regtest_seed()).unwrap();

    let pairs = client.get_reverse_swap_pairs().await.unwrap();
    let rbtc_pair = &pairs.0["BTC"]["RBTC"];

    let req = create_swap_request(&config, &km, next_key_index(), &rbtc_pair.hash, rbtc_pair.limits.minimal);
    let swap = client.create_reverse_swap(&req).await.unwrap();

    // Connect WS and subscribe — should receive initial status
    let ws = SwapStatusSubscriber::connect(&config.ws_url()).await.unwrap();
    let mut rx = ws.subscribe(&swap.id).await.unwrap();

    // The subscription response sends the current status (swap.created)
    let update = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("Timed out waiting for WS subscription response")
        .expect("WS channel closed unexpectedly");

    assert_eq!(update.swap_id, swap.id);
    assert_eq!(update.status, "swap.created");

    ws.close().await;
}

// ─── Full Lifecycle Test ─────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_full_swap_lifecycle() {
    let config = regtest_config();
    let client = BoltzApiClient::new(&config);
    let km = EvmKeyManager::from_seed(&regtest_seed()).unwrap();

    let pairs = client.get_reverse_swap_pairs().await.unwrap();
    let rbtc_pair = &pairs.0["BTC"]["RBTC"];

    let req = create_swap_request(&config, &km, next_key_index(), &rbtc_pair.hash, rbtc_pair.limits.minimal);
    let swap = client.create_reverse_swap(&req).await.unwrap();

    // Subscribe WS
    let ws = SwapStatusSubscriber::connect(&config.ws_url()).await.unwrap();
    let mut rx = ws.subscribe(&swap.id).await.unwrap();

    // Pay invoice in background (fire-and-forget — hold invoice blocks until claim)
    // Wait briefly for WS subscription to be fully established before paying
    tokio::time::sleep(Duration::from_secs(2)).await;

    docker::pay_invoice_lnd_background(&swap.invoice).expect("Failed to start invoice payment");

    // Collect statuses until we see transaction.confirmed (Boltz locked on-chain).
    // We don't claim in this test (no EtherSwap claim call), so the swap won't reach
    // invoice.settled. Reaching transaction.confirmed proves the full flow:
    // API create → LN payment → Boltz lockup → WS status delivery.
    let mut statuses = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut saw_confirmed = false;

    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(update)) => {
                eprintln!("WS update: {} -> {}", update.swap_id, update.status);
                statuses.push(update.status.clone());
                if update.status == "transaction.confirmed" {
                    saw_confirmed = true;
                    break;
                }
                if is_terminal_status(&update.status) {
                    break;
                }
            }
            Ok(None) => panic!("WS channel closed. Statuses: {statuses:?}"),
            Err(e) => panic!("Timed out after 60s ({e}). Statuses: {statuses:?}"),
        }
    }

    assert!(saw_confirmed, "Expected transaction.confirmed. Statuses: {statuses:?}");

    // Verify via REST API too
    let api_status = client.get_swap_status(&swap.id).await.unwrap();
    assert_eq!(
        api_status.status, "transaction.confirmed",
        "API should confirm lockup"
    );
    assert!(
        api_status.transaction.is_some(),
        "Should have lockup transaction"
    );

    ws.close().await;
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "invoice.settled"
            | "transaction.claimed"
            | "invoice.expired"
            | "swap.expired"
            | "invoice.failedToPay"
            | "transaction.refunded"
    )
}
