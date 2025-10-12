use anyhow::Result;
use breez_sdk_spark::*;
use tokio::sync::mpsc;
use tokio_with_wasm::alias as tokio;
use tracing::info;

use crate::faucet::RegtestFaucet;

/// Event listener that forwards events to a channel
struct ChannelEventListener {
    tx: mpsc::Sender<SdkEvent>,
}

#[async_trait::async_trait]
impl EventListener for ChannelEventListener {
    async fn on_event(&self, event: SdkEvent) {
        let _ = self.tx.send(event).await;
    }
}

/// Build and initialize a BreezSDK instance for testing
///
/// # Arguments
/// * `storage_dir` - Directory path for SDK storage
/// * `seed_bytes` - 32-byte seed for deterministic wallet generation
///
/// # Returns
/// A tuple of (SDK instance, event receiver channel)
pub async fn build_sdk(
    storage_dir: String,
    seed_bytes: [u8; 32],
) -> Result<(BreezSdk, mpsc::Receiver<SdkEvent>)> {
    let mut config = default_config(Network::Regtest);
    config.api_key = None; // Regtest: no API key needed
    config.lnurl_domain = None; // Avoid lnurl server in tests
    config.prefer_spark_over_lightning = true; // prefer spark transfers when possible
    config.sync_interval_secs = 5; // Faster syncing for tests

    let storage = default_storage(storage_dir)?;
    let seed = Seed::Entropy(seed_bytes.to_vec());

    let builder = SdkBuilder::new(config, seed, storage);
    let sdk = builder.build().await?;

    // Set up event listener
    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // Ensure initial sync completes
    let _ = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok((sdk, rx))
}

/// Wait for SDK wallet balance to reach at least the specified amount
///
/// This helper polls the wallet balance periodically until it reaches the minimum
/// required amount or times out.
///
/// # Arguments
/// * `sdk` - The BreezSDK instance to check
/// * `min_balance` - Minimum balance in satoshis to wait for
/// * `timeout_secs` - Maximum time to wait in seconds before giving up
///
/// # Returns
/// The current balance once it reaches the minimum, or error if timeout
pub async fn wait_for_balance(sdk: &BreezSdk, min_balance: u64, timeout_secs: u64) -> Result<u64> {
    let start = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_secs(3);

    loop {
        // Sync wallet to get latest state
        let _ = sdk.sync_wallet(SyncWalletRequest {}).await?;

        // Check current balance
        let info = sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?;

        if info.balance_sats >= min_balance {
            info!(
                "Balance requirement met: {} sats (required: {} sats)",
                info.balance_sats, min_balance
            );
            return Ok(info.balance_sats);
        }

        // Check timeout
        if start.elapsed().as_secs() > timeout_secs {
            anyhow::bail!(
                "Timeout waiting for balance >= {} sats after {} seconds. Current balance: {} sats",
                min_balance,
                timeout_secs,
                info.balance_sats
            );
        }

        info!(
            "Waiting for balance... current: {} sats, target: {} sats",
            info.balance_sats, min_balance
        );

        // Wait before next poll
        tokio::time::sleep(poll_interval).await;
    }
}

/// Fund an address using the regtest faucet and wait for funds to appear in SDK balance
///
/// This is a high-level helper that combines faucet funding with balance waiting.
///
/// # Arguments
/// * `sdk` - The BreezSDK instance
/// * `address` - Bitcoin address to fund
/// * `amount_sats` - Amount to request from faucet
/// * `min_expected_balance` - Minimum balance to wait for after funding
///
/// # Returns
/// The transaction ID from the faucet
pub async fn fund_address_and_wait(
    sdk: &BreezSdk,
    address: &str,
    amount_sats: u64,
    min_expected_balance: u64,
) -> Result<String> {
    let faucet = RegtestFaucet::new()?;

    info!(
        "Funding address {} with {} sats from faucet",
        address, amount_sats
    );

    let txid = faucet.fund_and_wait(address, amount_sats).await?;

    info!(
        "Faucet sent funds in txid: {}, waiting for balance...",
        txid
    );

    // Wait for balance to update (SDK auto-claims deposits in background)
    wait_for_balance(sdk, min_expected_balance, 180).await?;

    Ok(txid)
}

/// Get a deposit address and fund it from the faucet in one operation
///
/// This helper generates a deposit address, funds it, and waits for the balance to appear.
///
/// # Arguments
/// * `sdk` - The BreezSDK instance
/// * `amount_sats` - Amount to request from faucet
///
/// # Returns
/// Tuple of (deposit_address, funding_txid)
pub async fn receive_and_fund(sdk: &BreezSdk, amount_sats: u64) -> Result<(String, String)> {
    // Get a static deposit address
    let receive = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress,
        })
        .await?;

    let deposit_address = receive.payment_request;
    info!("Generated deposit address: {}", deposit_address);

    // Fund it
    let info = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let txid =
        fund_address_and_wait(sdk, &deposit_address, amount_sats, info.balance_sats + 1).await?;

    Ok((deposit_address, txid))
}

/// Wait for a payment to succeed by listening to SDK events
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `timeout_secs` - Maximum time to wait in seconds
///
/// # Returns
/// The payment details from the PaymentSucceed event
pub async fn wait_for_payment_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<Payment> {
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!(
                "Timeout waiting for payment event after {} seconds",
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(SdkEvent::PaymentSucceeded { payment })) => {
                info!(
                    "Received PaymentSucceeded event: {} sats, type: {:?}",
                    payment.amount, payment.payment_type
                );
                return Ok(payment);
            }
            Ok(Some(event)) => {
                // Log other events but keep waiting
                info!("Received SDK event: {:?}", event);
            }
            Ok(None) => {
                anyhow::bail!("Event channel closed unexpectedly");
            }
            Err(_) => {
                anyhow::bail!(
                    "Timeout waiting for payment event after {} seconds",
                    timeout_secs
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Ignore by default since it requires regtest infrastructure
    async fn test_build_sdk() {
        let data_dir = tempdir::TempDir::new("test-sdk").unwrap();
        let result = build_sdk(data_dir.path().to_string_lossy().to_string(), [1u8; 32]).await;
        assert!(result.is_ok(), "SDK should build successfully");
        let (_sdk, _rx) = result.unwrap();
    }
}
