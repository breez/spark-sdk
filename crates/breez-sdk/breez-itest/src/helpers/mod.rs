//! Shared test helpers split by environment:
//! - regtest-only helpers (builders, docker tree stores, faucet) live in `regtest`
//! - mainnet-only helpers (env-gated builders, conversion-pool quotes) live in `mainnet`
//! - the items in this file are common to both (event waiters, balance polling,
//!   preimage utilities, and the [`ChannelEventListener`] both submodules use)
//!
//! Submodule items are re-exported from `super::*`, so existing call sites keep
//! using `crate::helpers::*` without referencing the submodule path.

use anyhow::Result;
use bitcoin::hashes::{Hash as _, sha256};
use breez_sdk_spark::*;
use rand::RngCore;
use tokio::sync::mpsc;
use tracing::{debug, info};

pub mod mainnet;
pub mod regtest;
pub use mainnet::*;
pub use regtest::*;

/// Event listener that forwards events to a channel
pub(crate) struct ChannelEventListener {
    pub(crate) tx: mpsc::Sender<SdkEvent>,
}

#[async_trait::async_trait]
impl EventListener for ChannelEventListener {
    async fn on_event(&self, event: SdkEvent) {
        info!("Received SDK event: {event}");
        let _ = self.tx.try_send(event);
    }
}

pub async fn wait_for<F, Fut, T>(mut check_fn: F, timeout_secs: u64) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);

    loop {
        match check_fn().await {
            Ok(value) => {
                debug!(
                    "Condition met after {:?}, returning result",
                    start.elapsed()
                );
                return Ok(value);
            }
            Err(e) => {
                if start.elapsed() >= timeout {
                    return Err(anyhow::anyhow!(
                        "Timeout after {} seconds waiting for condition: {}",
                        timeout_secs,
                        e
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
}

/// Wait for SDK wallet balance to reach at least the specified amount
///
/// This helper polls the wallet balance periodically until it reaches the minimum
/// required amount or times out.
///
/// # Arguments
/// * `sdk` - The BreezSDK instance to check
/// * `min_balance` - Minimum balance in satoshis to wait for
/// * `max_balance` - Maximum balance in satoshis to wait for
/// * `timeout_secs` - Maximum time to wait in seconds before giving up
///
/// # Returns
/// The current balance once it reaches the minimum, or error if timeout
pub async fn wait_for_balance(
    sdk: &BreezSdk,
    min_balance: Option<u64>,
    max_balance: Option<u64>,
    timeout_secs: u64,
) -> Result<u64> {
    wait_for(
        || async {
            // Sync wallet to ensure we have the latest balance
            sdk.sync_wallet(SyncWalletRequest {}).await?;
            let info = sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(false),
                })
                .await?;

            if let Some(min_balance) = min_balance
                && info.balance_sats >= min_balance
            {
                info!(
                    "Balance requirement met: {} sats (required: {} sats)",
                    info.balance_sats, min_balance
                );
                return Ok(info.balance_sats);
            }

            if let Some(max_balance) = max_balance
                && info.balance_sats >= max_balance
            {
                info!(
                    "Balance requirement met: {} sats (required: {} sats)",
                    info.balance_sats, max_balance
                );
                return Ok(info.balance_sats);
            }

            info!(
                "Waiting for balance... current: {} sats, target min: {} sats or max: {} sats",
                info.balance_sats,
                min_balance.unwrap_or_default(),
                max_balance.unwrap_or_default()
            );

            anyhow::bail!(
                "Balance not yet reached. Current: {} sats, target min: {:?} sats, max: {:?} sats",
                info.balance_sats,
                min_balance,
                max_balance
            )
        },
        timeout_secs,
    )
    .await
}

/// Wait for a token balance to increase above a previous value.
///
/// Polls the SDK until the token balance for the given identifier exceeds `previous_balance`.
/// Syncs the wallet on each poll iteration.
///
/// # Arguments
/// * `sdk` - The SDK instance to query
/// * `token_identifier` - The token identifier to check balance for
/// * `previous_balance` - The balance threshold that must be exceeded
/// * `timeout_secs` - Maximum time to wait in seconds
///
/// # Returns
/// The new token balance once it exceeds `previous_balance`, or error if timeout
pub async fn wait_for_token_balance_increase(
    sdk: &BreezSdk,
    token_identifier: &str,
    previous_balance: u128,
    timeout_secs: u64,
) -> Result<u128> {
    let token_id = token_identifier.to_string();
    wait_for(
        || {
            let sdk = sdk.clone();
            let token_id = token_id.clone();
            async move {
                sdk.sync_wallet(SyncWalletRequest {}).await?;
                let info = sdk
                    .get_info(GetInfoRequest {
                        ensure_synced: Some(false),
                    })
                    .await?;
                let token_balance = info
                    .token_balances
                    .get(&token_id)
                    .map(|b| b.balance)
                    .unwrap_or(0);
                if token_balance > previous_balance {
                    Ok(token_balance)
                } else {
                    anyhow::bail!(
                        "Token balance not yet increased: {} (was {})",
                        token_balance,
                        previous_balance
                    )
                }
            }
        },
        timeout_secs,
    )
    .await
}

/// Wait for a token balance to reach an exact expected value.
///
/// Polls the SDK until the token balance for the given identifier equals `expected_balance`.
/// Syncs the wallet on each poll iteration.
///
/// # Arguments
/// * `sdk` - The SDK instance to query
/// * `token_identifier` - The token identifier to check balance for
/// * `expected_balance` - The exact balance to wait for
/// * `timeout_secs` - Maximum time to wait in seconds
///
/// # Returns
/// The token balance once it equals `expected_balance`, or error if timeout
pub async fn wait_for_token_balance(
    sdk: &BreezSdk,
    token_identifier: &str,
    expected_balance: u128,
    timeout_secs: u64,
) -> Result<u128> {
    let token_id = token_identifier.to_string();
    wait_for(
        || {
            let sdk = sdk.clone();
            let token_id = token_id.clone();
            async move {
                sdk.sync_wallet(SyncWalletRequest {}).await?;
                let info = sdk
                    .get_info(GetInfoRequest {
                        ensure_synced: Some(false),
                    })
                    .await?;
                let token_balance = info
                    .token_balances
                    .get(&token_id)
                    .map(|b| b.balance)
                    .unwrap_or(0);
                if token_balance == expected_balance {
                    Ok(token_balance)
                } else {
                    anyhow::bail!(
                        "Token balance not yet reached: {} (expected {})",
                        token_balance,
                        expected_balance
                    )
                }
            }
        },
        timeout_secs,
    )
    .await
}

/// Result of waiting for a specific SDK event
pub enum EventResult {
    /// Deposit claim succeeded
    ClaimSucceeded,
    /// Payment succeeded with details
    PaymentSucceeded(Box<Payment>),
    /// Payment pending with details
    PaymentPending(Box<Payment>),
    /// Payment failed with details
    PaymentFailed(Box<Payment>),
    /// Synced event occurred
    Synced,
    /// Lightning address changed
    LightningAddressChanged(Option<LightningAddressInfo>),
}

pub async fn clear_event_receiver(event_rx: &mut mpsc::Receiver<SdkEvent>) {
    while let Ok(event) = event_rx.try_recv() {
        info!("Clearing event from channel: {:?}", event);
    }
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
pub async fn wait_for_claimed_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<()> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "ClaimDeposits",
        |event| match event {
            SdkEvent::ClaimedDeposits { claimed_deposits } => {
                info!(
                    "Received ClaimedDeposits event: {} deposits claimed",
                    claimed_deposits.len()
                );
                Ok(Some(EventResult::ClaimSucceeded))
            }
            SdkEvent::UnclaimedDeposits { unclaimed_deposits } => Err(anyhow::anyhow!(
                "Received UnclaimedDeposits event: {} deposits unclaimed",
                unclaimed_deposits.len()
            )),
            other => {
                info!("Ignored SDK event: {:?}", other);
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
pub async fn wait_for_payment_succeeded_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_type: PaymentType,
    timeout_secs: u64,
) -> Result<Payment> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "PaymentSucceeded",
        |event| match event {
            SdkEvent::PaymentSucceeded { payment } if payment.payment_type == payment_type => {
                info!(
                    "Received PaymentSucceeded event: {} sats, type: {:?}",
                    payment.amount, payment.payment_type
                );
                Ok(Some(EventResult::PaymentSucceeded(Box::new(payment))))
            }
            other => {
                info!("Ignored SDK event: {:?}", other);
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

/// Wait for a PaymentSucceeded event matching both payment type and method.
/// This is more specific than `wait_for_payment_succeeded_event` and should be
/// used when multiple payments of the same type but different methods might arrive.
pub async fn wait_for_payment_succeeded_event_with_method(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_type: PaymentType,
    payment_method: PaymentMethod,
    timeout_secs: u64,
) -> Result<Payment> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "PaymentSucceeded",
        |event| match event {
            SdkEvent::PaymentSucceeded { payment }
                if payment.payment_type == payment_type && payment.method == payment_method =>
            {
                info!(
                    "Received PaymentSucceeded event: {} sats, type: {:?}, method: {:?}",
                    payment.amount, payment.payment_type, payment.method
                );
                Ok(Some(EventResult::PaymentSucceeded(Box::new(payment))))
            }
            SdkEvent::PaymentSucceeded { payment } => {
                info!(
                    "Ignored PaymentSucceeded event (wrong method): {} sats, type: {:?}, method: {:?}",
                    payment.amount, payment.payment_type, payment.method
                );
                Ok(None)
            }
            other => {
                info!("Ignored SDK event: {:?}", other);
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

pub async fn wait_for_payment_pending_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_type: PaymentType,
    timeout_secs: u64,
) -> Result<Payment> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "PaymentPending",
        |event| match event {
            SdkEvent::PaymentPending { payment } if payment.payment_type == payment_type => {
                info!(
                    "Received PaymentPending event: {} sats, type: {:?}",
                    payment.amount, payment.payment_type
                );
                Ok(Some(EventResult::PaymentPending(Box::new(payment))))
            }
            other => {
                info!("Ignored SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .and_then(|result| match result {
        EventResult::PaymentPending(payment) => Ok(*payment),
        _ => Err(anyhow::anyhow!("Unexpected event result")),
    })
}

pub async fn wait_for_payment_failed_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_type: PaymentType,
    timeout_secs: u64,
) -> Result<Payment> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "PaymentFailed",
        |event| match event {
            SdkEvent::PaymentFailed { payment } if payment.payment_type == payment_type => {
                info!(
                    "Received PaymentFailed event: {} sats, type: {:?}",
                    payment.amount, payment.payment_type
                );
                Ok(Some(EventResult::PaymentFailed(Box::new(payment))))
            }
            other => {
                info!("Ignored SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .and_then(|result| match result {
        EventResult::PaymentFailed(payment) => Ok(*payment),
        _ => Err(anyhow::anyhow!("Unexpected event result")),
    })
}

/// Wait for a synced SDK events
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `timeout_secs` - Maximum time to wait in seconds
pub async fn wait_for_synced_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<()> {
    wait_for_event(event_rx, timeout_secs, "Synced", |event| match event {
        SdkEvent::Synced => Ok(Some(EventResult::Synced)),
        other => {
            info!("Ignored SDK event: {:?}", other);
            Ok(None)
        }
    })
    .await
    .map(|_| ())
}

pub async fn wait_for_lightning_address_changed_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<Option<LightningAddressInfo>> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "LightningAddressChanged",
        |event| match event {
            SdkEvent::LightningAddressChanged {
                lightning_address, ..
            } => {
                info!(
                    "Received LightningAddressChanged event: {:?}",
                    lightning_address
                );
                Ok(Some(EventResult::LightningAddressChanged(
                    lightning_address,
                )))
            }
            other => {
                info!("Ignored SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .and_then(|result| match result {
        EventResult::LightningAddressChanged(addr) => Ok(addr),
        _ => Err(anyhow::anyhow!("Unexpected event result")),
    })
}

/// Wait for a set of payment events in any order.
///
/// Collects PaymentSucceeded events and marks them off from the expected list.
/// Returns Ok(()) when all expected events have been received.
/// Ignores non-matching events (e.g., Synced) and continues waiting.
///
/// Each expected event is specified as a (PaymentType, PaymentMethod) tuple.
async fn wait_for_payment_events_unordered(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    expected: Vec<(PaymentType, PaymentMethod)>,
    timeout_secs: u64,
) -> Result<()> {
    let mut remaining = expected;
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    while !remaining.is_empty() {
        let time_left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if time_left.is_zero() {
            anyhow::bail!(
                "Timeout after {} seconds. Still waiting for: {:?}",
                timeout_secs,
                remaining
            );
        }

        match tokio::time::timeout(time_left, event_rx.recv()).await {
            Ok(Some(SdkEvent::PaymentSucceeded { payment })) => {
                // Find and remove the first matching expected event
                if let Some(pos) = remaining
                    .iter()
                    .position(|(pt, pm)| *pt == payment.payment_type && *pm == payment.method)
                {
                    remaining.swap_remove(pos);
                    info!(
                        "Matched SDK event: {:?}/{:?} ({} remaining)",
                        payment.payment_type,
                        payment.method,
                        remaining.len()
                    );
                } else {
                    info!(
                        "Unmatched PaymentSucceeded event: {:?}/{:?} (still waiting for: {:?})",
                        payment.payment_type, payment.method, remaining
                    );
                }
            }
            Ok(Some(other)) => {
                info!("Ignored SDK event: {:?}", other);
                continue;
            }
            Ok(None) => anyhow::bail!("Event channel closed"),
            Err(_) => anyhow::bail!(
                "Timeout after {} seconds. Still waiting for: {:?}",
                timeout_secs,
                remaining
            ),
        }
    }
    Ok(())
}

/// Wait for and consume all auto-conversion events (BTC → Token) in any order:
/// - Receive payment (incoming BTC that triggered conversion)
/// - Send Spark (BTC to swap service)
/// - Receive Token (tokens from swap service)
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `receive_method` - The payment method of the incoming payment (Spark or Lightning)
/// * `timeout_secs` - Maximum time to wait in seconds
pub async fn wait_for_auto_conversion_events(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    receive_method: PaymentMethod,
    timeout_secs: u64,
) -> Result<()> {
    wait_for_payment_events_unordered(
        event_rx,
        vec![
            (PaymentType::Receive, receive_method),
            (PaymentType::Send, PaymentMethod::Spark),
            (PaymentType::Receive, PaymentMethod::Token),
        ],
        timeout_secs,
    )
    .await
}

/// Wait for and consume all payment conversion events (Token → BTC) in any order:
/// - Send Token (to swap service)
/// - Receive Spark (BTC from swap service)
/// - Send payment (actual outgoing payment)
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `payment_method` - The payment method of the final outgoing payment (Spark or Lightning)
/// * `timeout_secs` - Maximum time to wait in seconds
pub async fn wait_for_payment_conversion_events(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_method: PaymentMethod,
    timeout_secs: u64,
) -> Result<()> {
    wait_for_payment_events_unordered(
        event_rx,
        vec![
            (PaymentType::Send, PaymentMethod::Token),
            (PaymentType::Receive, PaymentMethod::Spark),
            (PaymentType::Send, payment_method),
        ],
        timeout_secs,
    )
    .await
}

pub fn generate_preimage_hash_pair() -> (String, String) {
    let mut preimage_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut preimage_bytes);
    let preimage = hex::encode(preimage_bytes);
    let payment_hash = sha256::Hash::hash(&preimage_bytes).to_string();
    (preimage, payment_hash)
}
