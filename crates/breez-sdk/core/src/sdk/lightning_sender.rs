//! Shared lightning-send helper used by both `BreezSdk::send_bolt11_invoice`
//! and cross-chain providers that pay an LN invoice as part of a larger
//! flow (e.g. Boltz reverse-swap hold invoices).
//!
//! Encapsulates the "pay the invoice, build the Payment row, persist it,
//! and poll the SSP until the status settles" sequence so callers don't
//! have to duplicate it — and so every LN-send path consistently benefits
//! from SSP-side polling and event emission.

use std::sync::Arc;

use platform_utils::time::Duration;
use platform_utils::tokio;
use spark_wallet::{SparkWallet, TransferId};
use tokio::select;
use tokio::sync::watch;
use tracing::{Instrument, error, info};

use crate::{
    Payment, PaymentDetails, PaymentStatus, Storage, error::SdkError, events::EventEmitter,
    utils::payments::get_payment_and_emit_event,
};

use super::{SyncCoordinator, SyncType};

/// Reusable helper that owns the dependencies needed to pay a BOLT11
/// invoice, persist the resulting [`Payment`] row, and reconcile its status
/// with the SSP via background polling.
///
/// Held behind `Arc` and shared between `BreezSdk` and any cross-chain
/// provider that pays LN invoices (currently: Boltz reverse swap).
pub(crate) struct LightningSender {
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    sync_coordinator: SyncCoordinator,
    event_emitter: Arc<EventEmitter>,
    shutdown_sender: watch::Sender<()>,
}

impl LightningSender {
    pub(crate) fn new(
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        sync_coordinator: SyncCoordinator,
        event_emitter: Arc<EventEmitter>,
        shutdown_sender: watch::Sender<()>,
    ) -> Self {
        Self {
            spark_wallet,
            storage,
            sync_coordinator,
            event_emitter,
            shutdown_sender,
        }
    }

    /// Pay a BOLT11 invoice, build the [`Payment`] row, persist it, and
    /// kick off SSP-side polling so the stored status is reconciled with
    /// the service provider's view as soon as the invoice settles.
    ///
    /// Callers attach any provider-specific metadata via
    /// `insert_payment_metadata` afterwards.
    pub(crate) async fn pay_and_persist_lightning_invoice(
        &self,
        invoice: &str,
        amount_sats: Option<u64>,
        fee_sats: u64,
        prefer_spark: bool,
        displayed_amount: u128,
        transfer_id: Option<TransferId>,
    ) -> Result<Payment, SdkError> {
        let payment_response = Box::pin(self.spark_wallet.pay_lightning_invoice(
            invoice,
            amount_sats,
            Some(fee_sats),
            prefer_spark,
            transfer_id,
        ))
        .await?;
        let payment = match payment_response.lightning_payment {
            Some(lightning_payment) => {
                let ssp_id = lightning_payment.id.clone();
                let htlc_details = payment_response
                    .transfer
                    .htlc_preimage_request
                    .ok_or_else(|| {
                        SdkError::Generic(
                            "Missing HTLC details for Lightning send payment".to_string(),
                        )
                    })?
                    .try_into()?;
                let payment = Payment::from_lightning(
                    lightning_payment,
                    displayed_amount,
                    payment_response.transfer.id.to_string(),
                    htlc_details,
                )?;
                self.spawn_poll(&payment, ssp_id);
                payment
            }
            None => payment_response.transfer.try_into()?,
        };
        self.storage.insert_payment(payment.clone()).await?;
        Ok(payment)
    }

    /// Poll the SSP until the lightning send payment reaches a non-pending
    /// state, then update storage and emit a payment event.
    fn spawn_poll(&self, payment: &Payment, ssp_id: String) {
        const MAX_POLL_ATTEMPTS: u32 = 20;
        let payment_id = payment.id.clone();
        info!("Polling lightning send payment {}", payment_id);

        let Some(htlc_details) = payment.details.as_ref().and_then(|d| match d {
            PaymentDetails::Lightning { htlc_details, .. } => Some(htlc_details.clone()),
            _ => None,
        }) else {
            error!(
                "Missing HTLC details for lightning send payment {payment_id}, skipping polling"
            );
            return;
        };
        let spark_wallet = self.spark_wallet.clone();
        let storage = self.storage.clone();
        let sync_coordinator = self.sync_coordinator.clone();
        let event_emitter = self.event_emitter.clone();
        let payment = payment.clone();
        let payment_id = payment_id.clone();
        let mut shutdown = self.shutdown_sender.subscribe();
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                for i in 0..MAX_POLL_ATTEMPTS {
                    info!(
                        "Polling lightning send payment {} attempt {}",
                        payment_id, i
                    );
                    select! {
                        _ = shutdown.changed() => {
                            info!("Shutdown signal received");
                            return;
                        },
                        p = spark_wallet.fetch_lightning_send_payment(&ssp_id) => {
                            if let Ok(Some(p)) = p && let Ok(payment) = Payment::from_lightning(p.clone(), payment.amount, payment.id.clone(), htlc_details.clone()) {
                                info!("Polling payment status = {} {:?}", payment.status, p.status);
                                if payment.status != PaymentStatus::Pending {
                                    info!("Polling payment completed status = {}", payment.status);
                                    // Update storage before emitting event so that
                                    // get_payment returns the correct status immediately.
                                    if let Err(e) = storage.insert_payment(payment.clone()).await {
                                        error!("Failed to update payment in storage: {e:?}");
                                    }
                                    // Fetch the payment to include already stored metadata
                                    get_payment_and_emit_event(&storage, &event_emitter, payment.clone()).await;
                                    sync_coordinator
                                        .trigger_sync_no_wait(SyncType::WalletState, true)
                                        .await;
                                    return;
                                }
                            }

                            let sleep_time = if i < 5 {
                                Duration::from_secs(1)
                            } else {
                                Duration::from_secs(i.into())
                            };
                            tokio::time::sleep(sleep_time).await;
                        }
                    }
                }
            }
            .instrument(span),
        );
    }
}
