//! Sync-triggered batch auto-conversion logic.
//!
//! Handles converting accumulated BTC balance above the threshold to the stable token.

use tokio::sync::watch;
use tokio_with_wasm::alias as tokio;
use tracing::{Instrument, debug, info, warn};

use crate::persist::PaymentMetadata;
use crate::realtime_sync::sync_lock::SyncLockGuard;
use crate::token_conversion::{
    ConversionAmount, ConversionError, ConversionOptions, ConversionPurpose, ConversionType,
};

use super::{AUTO_CONVERT_LOCK_NAME, PAYMENT_LOCK_NAME, StableBalance};

impl StableBalance {
    /// Spawns the background task that handles auto-conversion triggers.
    ///
    /// The task:
    /// 1. Waits for a trigger signal
    /// 2. Executes auto-conversion if conditions are met
    pub(super) fn spawn_auto_convert_task(&self, mut shutdown_receiver: watch::Receiver<()>) {
        let stable_balance = self.clone();
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                loop {
                    // Wait for a trigger or shutdown
                    tokio::select! {
                        _ = shutdown_receiver.changed() => {
                            info!("Auto-conversion task shutdown signal received");
                            return;
                        }
                        () = stable_balance.auto_convert_trigger.notified() => {}
                    }

                    if let Err(e) = stable_balance.auto_convert().await {
                        warn!("Auto-conversion failed: {e:?}");
                    }
                }
            }
            .instrument(span),
        );
    }

    /// Triggers the auto-conversion task.
    ///
    /// This is a non-blocking operation that sends a signal to the background task.
    /// The actual conversion will wait for any active conversions to complete.
    pub fn trigger_auto_convert(&self) {
        self.auto_convert_trigger.notify_one();
    }

    /// Executes auto-conversion if the balance exceeds the threshold.
    async fn auto_convert(&self) -> Result<bool, ConversionError> {
        // Get the active token, skip if stable balance is inactive
        let Some(active_token_identifier) = self.get_active_token_identifier().await else {
            debug!("Auto-conversion skipped: stable balance is inactive");
            return Ok(false);
        };

        // Check no payments are ongoing
        let ongoing = self.ongoing_payments.get();
        if ongoing > 0 {
            debug!("Auto-conversion skipped: {ongoing} payment(s) in progress");
            return Ok(false);
        }

        // Check if balance exceeds the trigger amount
        let (threshold, reserved) = self
            .get_or_init_effective_values(&active_token_identifier)
            .await?;
        let balance_sats = self.spark_wallet.get_balance().await?;
        let trigger_amount = reserved.saturating_add(threshold);
        if balance_sats < trigger_amount {
            debug!(
                "Auto-conversion skipped: balance {balance_sats} < reserved {reserved} + threshold {threshold}"
            );
            return Ok(false);
        }

        // Check if payment conversions are in progress on other instances
        if let Some(client) = &self.signing_client {
            match client.get_lock(PAYMENT_LOCK_NAME).await {
                Ok(true) => {
                    debug!("Auto-conversion skipped: payments lock held on another instance");
                    return Ok(false);
                }
                Ok(false) => {}
                Err(e) => {
                    debug!("Auto-conversion skipped: failed to check payments lock: {e:?}");
                    return Ok(false);
                }
            }
        }

        // Acquire exclusive auto-conversion lock — skip if another instance holds it
        let _lock_guard = match SyncLockGuard::new_exclusive(
            AUTO_CONVERT_LOCK_NAME.to_string(),
            self.signing_client.clone(),
        )
        .await
        {
            Ok(guard) => guard,
            Err(e) => {
                debug!("Auto-conversion skipped: failed to acquire exclusive lock: {e:?}");
                return Ok(false);
            }
        };

        // Convert the amount above the reserve
        let amount_to_convert = balance_sats.saturating_sub(reserved);

        info!(
            "Auto-conversion triggered: converting {amount_to_convert} sats to {active_token_identifier} (keeping {reserved} sats reserved)",
        );

        let options = ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: self.config.max_slippage_bps,
            completion_timeout_secs: None,
        };

        let response = self
            .token_converter
            .convert(
                &options,
                &ConversionPurpose::AutoConversion,
                Some(&active_token_identifier),
                ConversionAmount::AmountIn(u128::from(amount_to_convert)),
                None,
            )
            .await?;

        // Link sent payment as child of received payment
        self.storage
            .insert_payment_metadata(
                response.sent_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(response.received_payment_id.clone()),
                    ..Default::default()
                },
            )
            .await?;

        info!(
            "Auto-conversion completed: converted {} sats (sent_payment_id={}, received_payment_id={})",
            amount_to_convert, response.sent_payment_id, response.received_payment_id
        );

        // _lock_guard drops here, releasing the distributed lock

        Ok(true)
    }
}
