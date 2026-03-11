//! Per-receive conversion queue and logic.
//!
//! Handles converting individual received payments that meet the conversion minimum.

use spark_wallet::TransferId;
use tokio::sync::{mpsc, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, info, warn};

use crate::models::{ConversionStatus, PaymentDetails};
use crate::persist::PaymentMetadata;
use crate::token_conversion::{
    ConversionAmount, ConversionError, ConversionOptions, ConversionPurpose, ConversionType,
};

use super::StableBalance;

impl StableBalance {
    /// Spawns the background task that processes per-receive conversion queue.
    ///
    /// The task:
    /// 1. Waits for initial sync to complete
    /// 2. Processes queued conversion tasks one at a time
    pub(super) fn spawn_receive_convert_task(
        &self,
        mut rx: mpsc::UnboundedReceiver<String>,
        mut shutdown_receiver: watch::Receiver<()>,
    ) {
        let stable_balance = self.clone();

        tokio::spawn(async move {
            // Pre-warm effective values cache so process() min_from_amount checks are cache hits
            if let Some(token_id) = stable_balance.get_active_token_identifier().await {
                if let Err(e) = stable_balance.get_or_init_effective_values(&token_id).await {
                    warn!("Failed to pre-warm effective values: {e:?}");
                }
            }

            // Wait for initial sync before processing any tasks
            tokio::select! {
                _ = shutdown_receiver.changed() => {
                    info!("Per-receive conversion task shutdown signal received (before sync)");
                    return;
                }
                () = stable_balance.synced_notify.notified() => {
                    debug!("Per-receive conversion task: initial sync completed, starting queue processing");
                }
            }

            loop {
                tokio::select! {
                    _ = shutdown_receiver.changed() => {
                        info!("Per-receive conversion task shutdown signal received");
                        return;
                    }
                    payment_id = rx.recv() => {
                        let Some(payment_id) = payment_id else {
                            info!("Per-receive conversion queue closed");
                            return;
                        };

                        match stable_balance.per_receive_convert(&payment_id).await {
                            Ok(_) => {
                                // Update persisted status from Pending → Completed
                                // whether conversion was executed or skipped
                                if let Err(e) = stable_balance.storage.insert_payment_metadata(
                                    payment_id.clone(),
                                    PaymentMetadata {
                                        conversion_status: Some(ConversionStatus::Completed),
                                        ..Default::default()
                                    },
                                ).await {
                                    warn!("Failed to persist Completed status for {payment_id}: {e:?}");
                                }
                            }
                            Err(e) => {
                                // Check if failure is due to duplicate deterministic ID
                                // (another instance already handled this conversion)
                                if e.is_duplicate_transfer() {
                                    info!("Per-receive conversion for {payment_id}: already handled by another instance");
                                    // Don't mark as Failed — next sync will reconcile
                                } else {
                                    warn!("Per-receive conversion failed for {payment_id}: {e:?}");
                                    // Persist Failed status
                                    if let Err(e) = stable_balance.storage.insert_payment_metadata(
                                        payment_id.clone(),
                                        PaymentMetadata {
                                            conversion_status: Some(ConversionStatus::Failed),
                                            ..Default::default()
                                        },
                                    ).await {
                                        warn!("Failed to persist Failed status for {payment_id}: {e:?}");
                                    }
                                }
                            }
                        }
                        stable_balance.pending_receives.decrement();
                        stable_balance.trigger_sync().await;
                    }
                }
            }
        });
    }

    /// Converts a single received payment if it meets the minimum threshold.
    /// Returns `true` if conversion was performed, `false` if skipped.
    async fn per_receive_convert(&self, parent_payment_id: &str) -> Result<bool, ConversionError> {
        // Get the active token, skip if stable balance is inactive
        let Some(active_token_identifier) = self.get_active_token_identifier().await else {
            debug!("Per-receive conversion skipped: stable balance is inactive");
            return Ok(false);
        };

        // Acquire payment lock guard to block auto-convert while we convert
        let _lock_guard = self.create_payment_lock_guard();

        // Fetch payment from storage to get latest metadata and amount
        // This handles multi-instance race conditions where payment syncs before metadata arrives
        let payment = self
            .storage
            .get_payment_by_id(parent_payment_id.to_string())
            .await?;

        // Skip if this spark payment has conversion info
        if let Some(PaymentDetails::Spark {
            conversion_info: Some(_),
            ..
        }) = &payment.details
        {
            debug!(
                "Per-receive conversion skipped: {} is a conversion receive",
                parent_payment_id
            );
            return Ok(false);
        }

        // Check minimum threshold
        let amount_sats = payment.amount;
        let (_, _, min_from_amount) = self
            .get_or_init_effective_values(&active_token_identifier)
            .await?;
        let amount_sats_u64 = u64::try_from(amount_sats).unwrap_or(u64::MAX);
        if amount_sats_u64 < min_from_amount {
            debug!("Per-receive conversion skipped: amount {amount_sats} < min {min_from_amount}");
            return Ok(false);
        }

        // Generate deterministic transfer ID for idempotency
        // Use prefixed name to avoid collisions with other uses of TransferId::from_name
        let transfer_id = TransferId::from_name(&format!("receive_conversion:{parent_payment_id}"));
        debug!(
            "Per-receive deterministic id: {transfer_id} for payment id: {}",
            parent_payment_id
        );

        // Check if payment with this transfer_id already exists (already converted)
        if self
            .storage
            .get_payment_by_id(transfer_id.to_string())
            .await
            .is_ok()
        {
            debug!(
                "Per-receive conversion skipped: payment {} already exists",
                transfer_id
            );
            return Ok(false);
        }

        info!(
            "Per-receive conversion triggered: converting {amount_sats} sats to {active_token_identifier} for payment {parent_payment_id}",
        );

        // Perform conversion with deterministic transfer_id for idempotency
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
                ConversionAmount::AmountIn(amount_sats),
                Some(transfer_id),
            )
            .await?;

        // Link both conversion payments to the received parent payment
        self.storage
            .insert_payment_metadata(
                response.sent_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(parent_payment_id.to_string()),
                    ..Default::default()
                },
            )
            .await?;
        self.storage
            .insert_payment_metadata(
                response.received_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(parent_payment_id.to_string()),
                    ..Default::default()
                },
            )
            .await?;

        info!(
            "Per-receive conversion completed: converted {amount_sats} sats for {parent_payment_id} (sent={}, received={})",
            response.sent_payment_id, response.received_payment_id
        );

        Ok(true)
    }

    /// Queues a received payment for conversion if not already queued.
    pub fn queue_per_receive_convert(&self, payment_id: &str) {
        self.pending_receives.increment();
        // Queue the task (dedup will be handled by the processor checking storage)
        if let Err(e) = self.per_receive_tx.send(payment_id.to_string()) {
            self.pending_receives.decrement();
            warn!("Failed to queue per-receive conversion for {payment_id}: {e}");
        }
    }
}
