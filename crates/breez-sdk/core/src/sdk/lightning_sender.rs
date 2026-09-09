//! Shared lightning-send helper used by both `BreezSdk::send_bolt11_invoice`
//! and cross-chain providers that pay an LN invoice as part of a larger
//! flow (e.g. Boltz reverse-swap hold invoices).
//!
//! Encapsulates the "pay the invoice, build the Payment row, persist it,
//! and poll the SSP until the status settles" sequence so callers don't
//! have to duplicate it — and so every LN-send path consistently benefits
//! from SSP-side polling and event emission.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};

use platform_utils::time::Duration;
use platform_utils::tokio;
use spark_wallet::{PayLightningInvoiceResult, SparkWallet, TransferId};
use tokio::select;
use tokio::sync::{oneshot, watch};
use tracing::{Instrument, error, info, warn};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{
    Payment, PaymentDetails, PaymentStatus, Storage, error::SdkError, events::EventEmitter,
    persist::ObjectCacheRepository, utils::payments::record_payment_update,
};

/// A Lightning send that has been recorded but not yet handed to the SSP.
///
/// After the operators commit the leaves under a preimage condition, neither
/// they nor the SSP can say what the send was supposed to pay, so an
/// interrupted send could not otherwise be finished.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingLightningSend {
    pub transfer_id: String,
    pub invoice: String,
    pub amount_sats: Option<u64>,
    pub displayed_amount: u128,
}

/// Marks a send as running in this process for as long as it is held.
///
/// A guard rather than a paired call because the mark has to come off on every
/// exit: an early `?` between recording and sending, and a caller that drops the
/// send future part-way through. A mark left behind makes
/// [`LightningSender::resume_pending_sends`] skip the very send it exists to
/// finish, for the life of the process.
#[must_use = "dropping this immediately unmarks the send"]
pub(crate) struct InFlightSend {
    in_flight: Arc<StdMutex<HashSet<String>>>,
    transfer_id: String,
}

impl InFlightSend {
    /// Marking and unmarking both belong to the guard, so a send cannot be
    /// marked without something responsible for unmarking it.
    fn mark(in_flight: &Arc<StdMutex<HashSet<String>>>, transfer_id: &str) -> Self {
        lock_in_flight(in_flight).insert(transfer_id.to_string());
        Self {
            in_flight: in_flight.clone(),
            transfer_id: transfer_id.to_string(),
        }
    }
}

impl Drop for InFlightSend {
    fn drop(&mut self) {
        lock_in_flight(&self.in_flight).remove(&self.transfer_id);
    }
}

/// The set is only ever inserted into, removed from and queried, never held
/// across an await, so a poisoned lock carries no half-written state.
fn lock_in_flight(in_flight: &StdMutex<HashSet<String>>) -> MutexGuard<'_, HashSet<String>> {
    in_flight.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reusable helper that owns the dependencies needed to pay a BOLT11
/// invoice, persist the resulting [`Payment`] row, and reconcile its status
/// with the SSP via background polling.
///
/// Held behind `Arc` and shared between `BreezSdk` and any cross-chain
/// provider that pays LN invoices (currently: Boltz reverse swap).
pub(crate) struct LightningSender {
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    event_emitter: Arc<EventEmitter>,
    shutdown_sender: watch::Sender<()>,
    /// Serializes the read-modify-write of the pending-send list, which is one
    /// cache entry shared by every concurrent send.
    pending_lock: Mutex<()>,
    /// Transfer ids this process is currently sending. A resume skips these: the
    /// send that recorded them is still running and will finish or fail on its
    /// own, and resuming underneath it would persist the payment and start a
    /// second completion poll for the same send. Empty after a restart, which is
    /// exactly when the recorded sends do need resuming.
    in_flight: Arc<StdMutex<HashSet<String>>>,
}

impl LightningSender {
    pub(crate) fn new(
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        event_emitter: Arc<EventEmitter>,
        shutdown_sender: watch::Sender<()>,
    ) -> Self {
        Self {
            spark_wallet,
            storage,
            event_emitter,
            shutdown_sender,
            pending_lock: Mutex::new(()),
            in_flight: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    /// Pay a BOLT11 invoice, build the [`Payment`] row, persist it, and
    /// kick off SSP-side polling so the stored status is reconciled with
    /// the service provider's view as soon as the invoice settles.
    ///
    /// When `completion_timeout_secs` is non-zero, waits up to that long for
    /// the background poll to report a terminal status before returning; the
    /// poll keeps running (and still emits `PaymentSucceeded`) regardless, so
    /// a timeout simply returns the pre-confirmation payment. Pass `0` for
    /// fire-and-forget (return the pending payment immediately).
    ///
    /// Callers attach any provider-specific metadata via
    /// `insert_payment_metadata` afterwards.
    #[expect(clippy::too_many_arguments)]
    pub(crate) async fn pay_and_persist_lightning_invoice(
        &self,
        invoice: &str,
        amount_sats: Option<u64>,
        fee_sats: u64,
        prefer_spark: bool,
        displayed_amount: u128,
        transfer_id: Option<TransferId>,
        completion_timeout_secs: u64,
    ) -> Result<Payment, SdkError> {
        // Choose the transfer id here rather than letting the wallet generate one,
        // so the send can be recorded under the same id the operators will know it
        // by. Recorded before the call because the operator commit inside it is the
        // first step that moves funds.
        let transfer_id = transfer_id.unwrap_or_else(TransferId::generate);
        let _in_flight = self
            .record_pending_send(&PendingLightningSend {
                transfer_id: transfer_id.to_string(),
                invoice: invoice.to_string(),
                amount_sats,
                displayed_amount,
            })
            .await;

        let payment_response = Box::pin(self.spark_wallet.pay_lightning_invoice(
            invoice,
            amount_sats,
            Some(fee_sats),
            prefer_spark,
            Some(transfer_id.clone()),
        ))
        .await?;
        self.forget_pending_send(&transfer_id.to_string()).await;
        self.payment_from_pay_result(payment_response, displayed_amount, completion_timeout_secs)
            .await
    }

    /// Finishes any send that committed leaves with the operators but was
    /// interrupted before the SSP was asked to pay.
    ///
    /// Each attempt is idempotent: the SSP keys the send request on the transfer
    /// id, so a resume that raced a successful send returns the same request
    /// rather than opening a second one.
    pub(crate) async fn resume_pending_sends(&self) {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let pending = match cache.fetch_pending_lightning_sends().await {
            Ok(pending) => pending,
            Err(e) => {
                warn!("Failed to load pending lightning sends: {e:?}");
                return;
            }
        };

        if !pending.is_empty() {
            info!(
                "Found {} pending lightning send(s) to resume",
                pending.len()
            );
        }
        for entry in pending {
            let still_sending = { lock_in_flight(&self.in_flight).contains(&entry.transfer_id) };
            if still_sending {
                continue;
            }
            let Ok(transfer_id) = TransferId::from_str(&entry.transfer_id) else {
                error!("Discarding pending lightning send with unparsable transfer id");
                self.forget_pending_send(&entry.transfer_id).await;
                continue;
            };
            match self
                .spark_wallet
                .resume_lightning_send(&transfer_id, &entry.invoice, entry.amount_sats)
                .await
            {
                Ok(Some(payment_response)) => {
                    info!("Resumed lightning send {}", entry.transfer_id);
                    self.forget_pending_send(&entry.transfer_id).await;
                    if let Err(e) = self
                        .payment_from_pay_result(payment_response, entry.displayed_amount, 0)
                        .await
                    {
                        error!("Failed to persist resumed lightning send: {e:?}");
                    }
                }
                // Nothing left to pay: the transfer settled, was released, or was
                // never a Lightning send in the first place.
                Ok(None) => {
                    info!(
                        "Dropping pending lightning send {}: nothing left to pay",
                        entry.transfer_id
                    );
                    self.forget_pending_send(&entry.transfer_id).await;
                }
                // Keep the entry so the next sync tries again.
                Err(e) => warn!(
                    "Failed to resume lightning send {}: {e:?}",
                    entry.transfer_id
                ),
            }
        }
    }

    /// Records the send durably and marks it in flight until the returned guard
    /// is dropped. The record outlives the guard: that is what lets a send that
    /// failed before reaching the SSP be resumed later.
    pub(crate) async fn record_pending_send(&self, entry: &PendingLightningSend) -> InFlightSend {
        let guard = InFlightSend::mark(&self.in_flight, &entry.transfer_id);
        let _lock = self.pending_lock.lock().await;
        record_pending(&ObjectCacheRepository::new(self.storage.clone()), entry).await;
        guard
    }

    pub(crate) async fn forget_pending_send(&self, transfer_id: &str) {
        let _guard = self.pending_lock.lock().await;
        forget_pending(
            &ObjectCacheRepository::new(self.storage.clone()),
            transfer_id,
        )
        .await;
    }

    pub(crate) async fn payment_from_pay_result(
        &self,
        payment_response: PayLightningInvoiceResult,
        displayed_amount: u128,
        completion_timeout_secs: u64,
    ) -> Result<Payment, SdkError> {
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
                let completion_rx = self.spawn_poll(&payment, ssp_id);
                if completion_timeout_secs == 0 {
                    payment
                } else {
                    // Wait up to the caller's timeout for the background
                    // poll to signal completion. The poll keeps running in
                    // either branch — it still emits `PaymentSucceeded`
                    // when terminal — so dropping the receiver on timeout
                    // is harmless. We fall back to the pre-confirmation
                    // payment if the wait times out or the channel closes
                    // (e.g. missing HTLC details).
                    tokio::time::timeout(
                        Duration::from_secs(completion_timeout_secs),
                        completion_rx,
                    )
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or(payment)
                }
            }
            // Spark-routed Lightning sends complete synchronously inside
            // `pay_lightning_invoice` — there is no SSP-side state to poll,
            // so `completion_timeout_secs` is ignored for this branch and
            // the payment is returned with whatever status the transfer
            // already has.
            None => payment_response.transfer.try_into()?,
        };
        self.storage.apply_payment_update(payment.clone()).await?;
        Ok(payment)
    }

    /// Spawns the background poll that watches an outgoing Lightning send to
    /// completion. Returns a receiver that resolves to the terminal `Payment`
    /// when the SSP reports a non-`Pending` status, so callers can `await`
    /// completion synchronously with their own timeout.
    fn spawn_poll(&self, payment: &Payment, ssp_id: String) -> oneshot::Receiver<Payment> {
        const MAX_POLL_ATTEMPTS: u32 = 20;
        let payment_id = payment.id.clone();
        let (tx, rx) = oneshot::channel();
        info!("Polling lightning send payment {}", payment_id);

        let Some(htlc_details) = payment.details.as_ref().and_then(|d| match d {
            PaymentDetails::Lightning { htlc_details, .. } => Some(htlc_details.clone()),
            _ => None,
        }) else {
            error!(
                "Missing HTLC details for lightning send payment {payment_id}, skipping polling"
            );
            return rx;
        };
        let spark_wallet = self.spark_wallet.clone();
        let storage = self.storage.clone();
        let event_emitter = self.event_emitter.clone();
        let payment = payment.clone();
        let payment_id = payment_id.clone();
        let mut shutdown = self.shutdown_sender.subscribe();
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                // Drive the poll loop until we either reach a terminal status,
                // hit the attempt cap, or get a shutdown signal.
                let terminal_payment: Option<Payment> = 'poll: {
                    for i in 0..MAX_POLL_ATTEMPTS {
                        info!(
                            "Polling lightning send payment {} attempt {}",
                            payment_id, i
                        );
                        select! {
                            _ = shutdown.changed() => {
                                info!("Shutdown signal received");
                                break 'poll None;
                            },
                            p = spark_wallet.fetch_lightning_send_payment(&ssp_id) => {
                                if let Ok(Some(p)) = p && let Ok(payment) = Payment::from_lightning(p.clone(), payment.amount, payment.id.clone(), htlc_details.clone()) {
                                    info!("Polling payment status = {} {:?}", payment.status, p.status);
                                    if payment.status != PaymentStatus::Pending {
                                        info!("Polling payment completed status = {}", payment.status);
                                        break 'poll Some(payment);
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
                    None
                };

                let Some(payment) = terminal_payment else {
                    return;
                };

                let _ = tx.send(payment.clone());
                record_payment_update(&storage, &event_emitter, payment, true).await;
            }
            .instrument(span),
        );

        rx
    }
}

/// Callers hold `LightningSender::pending_lock` across this: the pending list is
/// one cache entry shared by every concurrent send.
async fn record_pending(cache: &ObjectCacheRepository, entry: &PendingLightningSend) {
    let mut pending = cache
        .fetch_pending_lightning_sends()
        .await
        .unwrap_or_default();
    // A retry reusing the caller's idempotency key sends under the same transfer
    // id, so replace rather than append.
    pending.retain(|p| p.transfer_id != entry.transfer_id);
    pending.push(entry.clone());
    if let Err(e) = cache.save_pending_lightning_sends(&pending).await {
        // The send still goes ahead: losing the record costs recoverability for
        // this one send, while refusing to send would be worse.
        warn!("Failed to record pending lightning send: {e:?}");
    }
}

async fn forget_pending(cache: &ObjectCacheRepository, transfer_id: &str) {
    let Ok(mut pending) = cache.fetch_pending_lightning_sends().await else {
        return;
    };
    pending.retain(|p| p.transfer_id != transfer_id);
    if let Err(e) = cache.save_pending_lightning_sends(&pending).await {
        warn!("Failed to clear pending lightning send {transfer_id}: {e:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guard_unmarks_on_drop() {
        let in_flight = Arc::new(StdMutex::new(HashSet::new()));
        {
            let _guard = InFlightSend::mark(&in_flight, "a");
            assert!(lock_in_flight(&in_flight).contains("a"));
        }
        assert!(!lock_in_flight(&in_flight).contains("a"));
    }

    /// The case that a paired release call got wrong: a `?` between marking the
    /// send and starting it. A mark left behind would make every later resume
    /// skip this transfer for the life of the process.
    #[test]
    fn the_guard_unmarks_when_the_send_never_starts() {
        fn fails_before_sending(
            in_flight: &Arc<StdMutex<HashSet<String>>>,
        ) -> Result<(), &'static str> {
            let _guard = InFlightSend::mark(in_flight, "a");
            Err("prepared transfer was unusable")?;
            unreachable!()
        }

        let in_flight = Arc::new(StdMutex::new(HashSet::new()));
        assert!(fails_before_sending(&in_flight).is_err());
        assert!(!lock_in_flight(&in_flight).contains("a"));
    }

    #[test]
    fn concurrent_sends_are_marked_independently() {
        let in_flight = Arc::new(StdMutex::new(HashSet::new()));
        let first = InFlightSend::mark(&in_flight, "a");
        let _second = InFlightSend::mark(&in_flight, "b");

        drop(first);

        assert!(!lock_in_flight(&in_flight).contains("a"));
        assert!(lock_in_flight(&in_flight).contains("b"));
    }
}

/// The cache-backed half needs a real `Storage`, and the only one compiled into
/// this crate is behind `sqlite`, which is off for wasm.
#[cfg(all(test, feature = "sqlite"))]
mod storage_tests {
    use super::*;
    use crate::persist::sqlite::SqliteStorage;

    fn cache(name: &str) -> (ObjectCacheRepository, std::path::PathBuf) {
        let mut dir = std::env::temp_dir();
        dir.push(format!("breez-test-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = SqliteStorage::new(&dir).expect("Failed to create storage");
        (
            ObjectCacheRepository::new(Arc::new(storage) as Arc<dyn Storage>),
            dir,
        )
    }

    fn entry(transfer_id: &str, invoice: &str) -> PendingLightningSend {
        PendingLightningSend {
            transfer_id: transfer_id.to_string(),
            invoice: invoice.to_string(),
            amount_sats: Some(1_000),
            displayed_amount: 1_000,
        }
    }

    #[tokio::test]
    async fn no_records_before_any_send() {
        let (cache, _dir) = cache("no_records");
        assert!(
            cache
                .fetch_pending_lightning_sends()
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// The failure path leaves the record in place: this is what lets a send that
    /// never reached the SSP be resumed after a restart.
    #[tokio::test]
    async fn a_recorded_send_survives_until_forgotten() {
        let (cache, _dir) = cache("survives");
        record_pending(&cache, &entry("a", "lnbc-a")).await;

        let pending = cache.fetch_pending_lightning_sends().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].invoice, "lnbc-a");

        forget_pending(&cache, "a").await;
        assert!(
            cache
                .fetch_pending_lightning_sends()
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn concurrent_sends_are_recorded_independently() {
        let (cache, _dir) = cache("independent");
        record_pending(&cache, &entry("a", "lnbc-a")).await;
        record_pending(&cache, &entry("b", "lnbc-b")).await;

        forget_pending(&cache, "a").await;

        let pending = cache.fetch_pending_lightning_sends().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].transfer_id, "b");
    }

    /// A retry reusing the caller's idempotency key sends under the same transfer
    /// id. A duplicated record would be resumed twice.
    #[tokio::test]
    async fn re_recording_one_transfer_id_replaces_it() {
        let (cache, _dir) = cache("replaces");
        record_pending(&cache, &entry("a", "lnbc-first")).await;
        record_pending(&cache, &entry("a", "lnbc-second")).await;

        let pending = cache.fetch_pending_lightning_sends().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].invoice, "lnbc-second");
    }

    #[tokio::test]
    async fn forgetting_an_unknown_send_leaves_the_rest() {
        let (cache, _dir) = cache("unknown");
        record_pending(&cache, &entry("a", "lnbc-a")).await;
        forget_pending(&cache, "does-not-exist").await;
        assert_eq!(
            cache.fetch_pending_lightning_sends().await.unwrap().len(),
            1
        );
    }
}
