use std::time::Duration;

use anyhow::Result;
use breez_sdk_spark::*;
use tokio::sync::mpsc;
use tracing::info;

use crate::SdkInstance;
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
/// * `temp_dir` - Optional TempDir to keep alive (prevents premature deletion)
///
/// # Returns
/// An SdkInstance containing the SDK, event channel, and optional TempDir
pub async fn build_sdk_with_dir(
    storage_dir: String,
    seed_bytes: [u8; 32],
    temp_dir: Option<tempdir::TempDir>,
) -> Result<SdkInstance> {
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

    Ok(SdkInstance {
        sdk,
        events: rx,
        temp_dir,
    })
}

/// Build and initialize a BreezSDK instance for testing (without TempDir management)
///
/// # Arguments
/// * `storage_dir` - Directory path for SDK storage
/// * `seed_bytes` - 32-byte seed for deterministic wallet generation
///
/// # Returns
/// An SdkInstance containing the SDK and event channel
pub async fn build_sdk(storage_dir: String, seed_bytes: [u8; 32]) -> Result<SdkInstance> {
    build_sdk_with_dir(storage_dir, seed_bytes, None).await
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

/// Get a deposit address and fund it from the faucet in one operation
///
/// This helper generates a deposit address, funds it, and waits for the claim event.
///
/// # Arguments
/// * `sdk_instance` - The SdkInstance with SDK and event channel
/// * `amount_sats` - Amount to request from faucet
///
/// # Returns
/// Tuple of (deposit_address, funding_txid)
pub async fn receive_and_fund(
    sdk_instance: &mut SdkInstance,
    amount_sats: u64,
) -> Result<(String, String)> {
    // Get a static deposit address
    let receive = sdk_instance
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress,
        })
        .await?;

    let deposit_address = receive.payment_request;
    info!("Generated deposit address: {}", deposit_address);

    // Fund the address
    let faucet = RegtestFaucet::new()?;
    info!(
        "Funding address {} with {} sats from faucet",
        deposit_address, amount_sats
    );
    let txid = faucet.fund_address(&deposit_address, amount_sats).await?;

    info!(
        "Faucet sent funds in txid: {}, waiting for claim event...",
        txid
    );

    // Wait for the ClaimDepositsSucceeded event
    wait_for_claim_event(&mut sdk_instance.events, 180).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;

    Ok((deposit_address, txid))
}

/// Result of waiting for a specific SDK event
pub enum EventResult {
    /// Deposit claim succeeded
    ClaimSucceeded,
    /// Payment succeeded with details
    PaymentSucceeded(Box<Payment>),
}

/// Generic event waiter with timeout
///
/// # Arguments
/// * `event_rx` - Event receiver channel
/// * `timeout_secs` - Maximum time to wait in seconds
/// * `matcher` - Function that matches and extracts the desired event
///
/// # Returns
/// The matched event result or error on timeout/failure
async fn wait_for_event<F>(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
    event_name: &str,
    mut matcher: F,
) -> Result<EventResult>
where
    F: FnMut(SdkEvent) -> Result<Option<EventResult>>,
{
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!(
                "Timeout waiting for {} event after {} seconds",
                event_name,
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(event)) => {
                match matcher(event) {
                    Ok(Some(result)) => return Ok(result),
                    Ok(None) => {
                        // Not the event we're looking for, keep waiting
                        continue;
                    }
                    Err(e) => {
                        // Matcher returned an error (e.g., failure event)
                        return Err(e);
                    }
                }
            }
            Ok(None) => {
                anyhow::bail!("Event channel closed unexpectedly");
            }
            Err(_) => {
                anyhow::bail!(
                    "Timeout waiting for {} event after {} seconds",
                    event_name,
                    timeout_secs
                );
            }
        }
    }
}

/// Wait for a deposit claim to succeed by listening to SDK events
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `timeout_secs` - Maximum time to wait in seconds
///
/// # Returns
/// Ok if claim succeeded, Error if timeout or failure
pub async fn wait_for_claim_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<()> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "ClaimDeposits",
        |event| match event {
            SdkEvent::ClaimDepositsSucceeded { claimed_deposits } => {
                info!(
                    "Received ClaimDepositsSucceeded event: {} deposits claimed",
                    claimed_deposits.len()
                );
                Ok(Some(EventResult::ClaimSucceeded))
            }
            SdkEvent::ClaimDepositsFailed { unclaimed_deposits } => Err(anyhow::anyhow!(
                "Deposit claim failed: {} deposits unclaimed",
                unclaimed_deposits.len()
            )),
            other => {
                info!("Received SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .map(|_| ())
}

/// Wait for a payment to succeed by listening to SDK events
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `timeout_secs` - Maximum time to wait in seconds
///
/// # Returns
/// The payment details from the PaymentSucceeded event
pub async fn wait_for_payment_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<Payment> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "PaymentSucceeded",
        |event| match event {
            SdkEvent::PaymentSucceeded { payment } => {
                info!(
                    "Received PaymentSucceeded event: {} sats, type: {:?}",
                    payment.amount, payment.payment_type
                );
                Ok(Some(EventResult::PaymentSucceeded(Box::new(payment))))
            }
            other => {
                info!("Received SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .and_then(|result| match result {
        EventResult::PaymentSucceeded(payment) => Ok(*payment),
        _ => Err(anyhow::anyhow!("Unexpected event result")),
    })
}
