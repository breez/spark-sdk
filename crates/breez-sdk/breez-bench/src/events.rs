//! Event waiting utilities for benchmarks.
//!
//! Provides helper functions to wait for specific SDK events with timeout handling.

use anyhow::{Result, bail};
use breez_sdk_spark::SdkEvent;
use tokio::sync::mpsc;
use tracing::info;

/// Wait for SDK sync event.
///
/// Blocks until a `Synced` event is received or timeout expires.
pub async fn wait_for_synced_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<()> {
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!(
                "Timeout waiting for Synced event after {} seconds",
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(SdkEvent::Synced)) => {
                return Ok(());
            }
            Ok(Some(_)) => continue,
            Ok(None) => bail!("Event channel closed"),
            Err(_) => bail!(
                "Timeout waiting for Synced event after {} seconds",
                timeout_secs
            ),
        }
    }
}

/// Wait for deposit claim event.
///
/// Blocks until a `ClaimedDeposits` event is received or timeout expires.
/// Returns an error if `UnclaimedDeposits` is received instead.
pub async fn wait_for_claimed_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<()> {
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!(
                "Timeout waiting for ClaimedDeposits event after {} seconds",
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(SdkEvent::ClaimedDeposits { claimed_deposits })) => {
                info!("Claimed {} deposits", claimed_deposits.len());
                return Ok(());
            }
            Ok(Some(SdkEvent::UnclaimedDeposits { unclaimed_deposits })) => {
                bail!(
                    "Deposit claim failed: {} unclaimed deposits",
                    unclaimed_deposits.len()
                );
            }
            Ok(Some(_)) => continue,
            Ok(None) => bail!("Event channel closed"),
            Err(_) => bail!(
                "Timeout waiting for ClaimedDeposits event after {} seconds",
                timeout_secs
            ),
        }
    }
}
