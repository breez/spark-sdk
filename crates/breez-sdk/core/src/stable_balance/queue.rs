//! Unified conversion queue and worker for stable balance.
//!
//! Serializes all conversion tasks (per-receive, auto-convert, and deactivation)
//! through a single queue to eliminate race conditions between the paths.

use std::sync::Arc;
use std::time::Duration;

use platform_utils::tokio;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify, watch};
use tracing::{Instrument, debug, info, warn};

use crate::events::{SdkEvent, StableBalanceConversionKind};
use crate::models::ConversionStatus;
use crate::persist::{ObjectCacheRepository, PaymentMetadata, Storage};
use crate::utils::time::now_secs;

use super::{StableBalance, per_receive_transfer_id};

/// A conversion task to be processed by the worker.
#[derive(Clone, Debug)]
pub(crate) enum ConversionTask {
    /// Convert a single received payment's sats to the stable token.
    PerReceive(String),
    /// Batch-convert accumulated BTC above the threshold.
    AutoConvert,
    /// Convert all tokens back to BTC on deactivation.
    Deactivation(String),
}

/// State of a pending per-receive conversion in the queue.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub(crate) enum PendingState {
    /// Ready to be processed by the worker.
    #[default]
    Ready,
    /// Failed at least once. Skipped by the worker, waiting for either:
    /// - A `PaymentSucceeded` event matching the deterministic `transfer_id`
    ///   (another instance completed the conversion)
    /// - Timeout expiry (genuine failure)
    Deferred,
}

/// How long to keep a deferred task before marking it as failed (seconds).
const DEFERRED_TASK_TIMEOUT_SECS: u64 = 120;

/// Backoff between attempts of a failed auto-convert or deactivation
/// conversion: 30 s, doubling to a cap of one hour. Without it a failed swap
/// is refunded by the AMM as an incoming sats transfer, that receive re-queues
/// the same conversion, and the loop runs every few seconds for as long as the
/// wallet is open (observed at 127 attempts in 13 minutes on mainnet when the
/// selected pool had no liquidity), leaving a pair of transfers in the history
/// each time. The cap keeps a long outage self-healing: once liquidity is back
/// the next hourly attempt goes through.
const CONVERSION_BACKOFF_BASE_SECS: u64 = 30;
const CONVERSION_BACKOFF_MAX_SECS: u64 = 3_600;

/// Seconds to wait before the `consecutive_failures`-th retry.
fn backoff_secs(consecutive_failures: u32) -> u64 {
    let doublings = consecutive_failures.saturating_sub(1).min(20);
    CONVERSION_BACKOFF_BASE_SECS
        .saturating_mul(1_u64 << doublings)
        .min(CONVERSION_BACKOFF_MAX_SECS)
}

/// A pending per-receive conversion with its processing state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PendingConversion {
    payment_id: String,
    #[serde(default)]
    state: PendingState,
    /// Unix timestamp when this task was first created.
    #[serde(default)]
    created_at: u64,
}

/// Result of processing a per-receive conversion task.
enum PerReceiveResult {
    /// Conversion succeeded or was already handled.
    Done { converted: bool },
    /// Conversion failed — defer until resolved by event or timeout.
    Retry,
}

/// Internal state of the conversion queue.
struct ConversionQueueState {
    /// Ordered list of pending per-receive conversions.
    per_receive: Vec<PendingConversion>,
    /// A pending non-per-receive task (auto-convert or deactivation).
    pending_task: Option<ConversionTask>,
    /// How many times `pending_task` has failed in a row, and the unix time
    /// before which it must not run again. Both reset when the task completes
    /// or is replaced.
    task_failures: u32,
    task_not_before: Option<u64>,
}

impl ConversionQueueState {
    fn reset_backoff(&mut self) {
        self.task_failures = 0;
        self.task_not_before = None;
    }
}

/// A priority queue that serializes conversion tasks.
///
/// Per-receive tasks always execute before auto-convert. Items remain in the
/// queue while being processed (dequeue after completion) so that dedup and
/// collapse continue to work during processing.
pub(crate) struct ConversionQueue {
    state: Mutex<ConversionQueueState>,
    pub(crate) notify: Arc<Notify>,
    storage: Arc<dyn Storage>,
}

impl ConversionQueue {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            state: Mutex::new(ConversionQueueState {
                per_receive: Vec::new(),
                pending_task: None,
                task_failures: 0,
                task_not_before: None,
            }),
            notify: Arc::new(Notify::new()),
            storage,
        }
    }

    /// Queue a per-receive conversion task. Deduplicates by `payment_id`.
    /// Persists the pending list for restart recovery.
    pub async fn push_per_receive(&self, payment_id: String) {
        let mut state = self.state.lock().await;
        if !state.per_receive.iter().any(|p| p.payment_id == payment_id) {
            state.per_receive.push(PendingConversion {
                payment_id,
                state: PendingState::Ready,
                created_at: now_secs(),
            });
            self.persist_pending(&state).await;
            self.notify.notify_one();
        }
    }

    /// Queue an auto-convert task. Collapses multiple triggers into one.
    /// Does not override a pending deactivation task.
    pub async fn push_auto_convert(&self) {
        let mut state = self.state.lock().await;
        if state.pending_task.is_none() {
            state.pending_task = Some(ConversionTask::AutoConvert);
            self.notify.notify_one();
        }
    }

    /// Queue a deactivation conversion task. Overrides any pending auto-convert
    /// and any backoff it was under: this is the user acting, so it runs now.
    /// Persisted so a conversion that has not yet succeeded survives a restart.
    pub async fn push_deactivation(&self, token_identifier: String) {
        let mut state = self.state.lock().await;
        debug!("Queuing deactivation conversion for token {token_identifier}");
        let cache = ObjectCacheRepository::new(self.storage.clone());
        if let Err(e) = cache.save_pending_deactivation(&token_identifier).await {
            warn!("Failed to persist pending deactivation: {e:?}");
        }
        state.pending_task = Some(ConversionTask::Deactivation(token_identifier));
        state.reset_backoff();
        self.notify.notify_one();
    }

    /// Clear a pending auto-convert task if one exists.
    /// Used after a successful per-receive conversion to prevent auto-convert
    /// from running with a stale balance before the next sync completes.
    pub async fn clear_pending_auto_convert(&self) {
        let mut state = self.state.lock().await;
        if matches!(state.pending_task, Some(ConversionTask::AutoConvert)) {
            state.pending_task = None;
        }
    }

    /// Returns `true` if there are any per-receive tasks in the queue.
    /// Used by auto-convert to yield to per-receive tasks that arrived while it was preparing.
    /// Matches `next_task()` semantics: auto-convert should not run while any per-receive
    /// tasks exist (ready or deferred), since deferred tasks may still need those sats.
    pub async fn has_per_receive(&self) -> bool {
        let state = self.state.lock().await;
        !state.per_receive.is_empty()
    }

    /// Clear all pending tasks from the queue.
    /// Returns the payment IDs of any cleared per-receive tasks (for status updates).
    pub async fn clear_queue(&self) -> Vec<String> {
        let mut state = self.state.lock().await;
        let cleared: Vec<String> = state.per_receive.drain(..).map(|p| p.payment_id).collect();
        if matches!(state.pending_task, Some(ConversionTask::Deactivation(_))) {
            self.forget_pending_deactivation().await;
        }
        state.pending_task = None;
        state.reset_backoff();
        self.persist_pending(&state).await;
        cleared
    }

    /// Mark a per-receive task as deferred (waiting for resolution).
    pub async fn defer_task(&self, payment_id: &str) {
        let mut state = self.state.lock().await;
        if let Some(pending) = state
            .per_receive
            .iter_mut()
            .find(|p| p.payment_id == payment_id)
        {
            pending.state = PendingState::Deferred;
            self.persist_pending(&state).await;
        }
    }

    /// Returns the next task to process without removing it.
    /// Per-receive tasks take priority over auto-convert/deactivation.
    /// Skips deferred per-receive tasks.
    pub async fn next_task(&self) -> Option<ConversionTask> {
        let state = self.state.lock().await;
        if let Some(pending) = state
            .per_receive
            .iter()
            .find(|p| p.state != PendingState::Deferred)
        {
            Some(ConversionTask::PerReceive(pending.payment_id.clone()))
        } else if state.per_receive.is_empty() {
            // Only run auto-convert/deactivation when no per-receive tasks exist (including
            // deferred). Deferred tasks may still be resolved by a PaymentSucceeded event
            // and need those sats.
            //
            // A task under backoff stays queued but is not due yet; the worker
            // sleeps until `retry_delay` says it is.
            let due = state
                .task_not_before
                .is_none_or(|not_before| now_secs() >= not_before);
            if due {
                state.pending_task.clone()
            } else {
                None
            }
        } else {
            None
        }
    }

    /// How long until the pending auto-convert / deactivation task may run
    /// again, when it is under backoff. `None` when nothing is waiting on time.
    pub async fn retry_delay(&self) -> Option<Duration> {
        let state = self.state.lock().await;
        let not_before = state.task_not_before?;
        state.pending_task.as_ref()?;
        Some(Duration::from_secs(not_before.saturating_sub(now_secs())))
    }

    /// Keep the pending task queued after a failure and schedule its next
    /// attempt. Returns the delay in seconds and the failure count so far.
    pub async fn retry_later(&self, task: &ConversionTask) -> (u64, u32) {
        let mut state = self.state.lock().await;
        // The task may have been replaced while it ran (a deactivation
        // arriving during an auto-convert); only back off the task that failed.
        let same_task = match (&state.pending_task, task) {
            (Some(ConversionTask::AutoConvert), ConversionTask::AutoConvert) => true,
            (Some(ConversionTask::Deactivation(a)), ConversionTask::Deactivation(b)) => a == b,
            _ => false,
        };
        if !same_task {
            return (0, 0);
        }
        state.task_failures = state.task_failures.saturating_add(1);
        let delay = backoff_secs(state.task_failures);
        state.task_not_before = Some(now_secs().saturating_add(delay));
        (delay, state.task_failures)
    }

    async fn forget_pending_deactivation(&self) {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        if let Err(e) = cache.delete_pending_deactivation().await {
            warn!("Failed to clear persisted pending deactivation: {e:?}");
        }
    }

    /// Remove a completed task from the queue.
    /// Persists the updated pending list for per-receive tasks.
    pub async fn complete_task(&self, task: &ConversionTask) {
        let mut state = self.state.lock().await;
        match task {
            ConversionTask::PerReceive(id) => {
                state.per_receive.retain(|p| p.payment_id != *id);
                self.persist_pending(&state).await;
            }
            ConversionTask::AutoConvert => {
                state.pending_task = None;
                state.reset_backoff();
            }
            ConversionTask::Deactivation(_) => {
                state.pending_task = None;
                state.reset_backoff();
                self.forget_pending_deactivation().await;
            }
        }
    }

    /// Check if an incoming payment is the conversion result for a deferred task.
    ///
    /// Computes the deterministic `transfer_id` for each deferred task and compares
    /// it to the incoming payment ID. If a match is found, the task is removed
    /// from the queue and its parent `payment_id` is returned.
    pub async fn resolve_by_conversion_payment(&self, incoming_payment_id: &str) -> Option<String> {
        let mut state = self.state.lock().await;
        let idx = state.per_receive.iter().position(|p| {
            p.state == PendingState::Deferred
                && per_receive_transfer_id(&p.payment_id).to_string() == incoming_payment_id
        })?;
        let resolved = state.per_receive.remove(idx);
        self.persist_pending(&state).await;
        // Wake the worker so it can process the next queued task
        self.notify.notify_one();
        Some(resolved.payment_id)
    }

    /// Remove deferred tasks that have exceeded the timeout and return their `payment_ids`.
    /// Called on `Synced` events to clean up tasks that were never resolved.
    pub async fn clear_expired_tasks(&self) -> Vec<String> {
        let now = now_secs();
        let mut state = self.state.lock().await;
        let mut timed_out = Vec::new();
        state.per_receive.retain(|p| {
            if p.state == PendingState::Deferred
                && p.created_at > 0
                && now.saturating_sub(p.created_at) > DEFERRED_TASK_TIMEOUT_SECS
            {
                timed_out.push(p.payment_id.clone());
                false
            } else {
                true
            }
        });
        if !timed_out.is_empty() {
            self.persist_pending(&state).await;
            // Wake the worker so it can process tasks that were blocked by deferred entries
            self.notify.notify_one();
        }
        timed_out
    }

    /// Persist the per-receive queue for restart recovery.
    async fn persist_pending(&self, state: &ConversionQueueState) {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        if state.per_receive.is_empty() {
            if let Err(e) = cache.delete_pending_conversions().await {
                warn!("Failed to delete pending conversions cache: {e:?}");
            }
        } else if let Err(e) = cache.save_pending_conversions(&state.per_receive).await {
            warn!("Failed to persist pending conversions: {e:?}");
        }
    }
}

impl StableBalance {
    /// Spawns the unified conversion worker that processes all conversion tasks.
    ///
    /// The worker:
    /// 1. Waits for the initial sync to complete
    /// 2. Recovers any pending conversions from a previous session
    /// 3. Queues a cold-start auto-convert
    /// 4. Processes tasks serially (per-receive first, then auto-convert)
    pub(crate) fn spawn_conversion_worker(&self, mut shutdown_receiver: watch::Receiver<()>) {
        let stable_balance = self.clone();
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                // Pre-warm effective values cache
                if let Some(token_id) = stable_balance.get_active_token_identifier().await
                    && let Err(e) = stable_balance.core.get_or_init_effective_values(&token_id).await
                {
                    warn!("Failed to pre-warm effective values: {e:?}");
                }

                // Restore pending conversions before waiting for sync, so the
                // first Synced event can expire any stale deferred tasks.
                stable_balance.recover_pending_conversions().await;

                // Wait for initial sync before processing any tasks
                tokio::select! {
                    _ = shutdown_receiver.changed() => {
                        info!("Conversion worker shutdown before initial sync");
                        return;
                    }
                    () = stable_balance.core.synced_notify.notified() => {
                        debug!("Conversion worker: initial sync completed");
                    }
                }

                // Cold-start: queue auto-convert for any existing excess balance
                stable_balance.core.queue.push_auto_convert().await;

                // Main processing loop
                debug!("Conversion worker: entering main loop");
                loop {
                    // Register notify future BEFORE checking the queue to avoid missed wakeups
                    let notified = stable_balance.core.queue.notify.notified();

                    // Drain all available tasks
                    while let Some(task) = stable_balance.core.queue.next_task().await {
                        debug!("Conversion worker: processing task {task:?}");
                        stable_balance.process_task(&task).await;
                    }

                    // A task under backoff is not returned by `next_task` until
                    // it is due, and nothing notifies when the clock runs out —
                    // so sleep for exactly that long alongside the usual wakeups.
                    let retry_delay = stable_balance.core.queue.retry_delay().await;
                    debug!("Conversion worker: queue drained, waiting for new tasks (retry in {retry_delay:?})");
                    tokio::select! {
                        _ = shutdown_receiver.changed() => {
                            info!("Conversion worker shutdown");
                            return;
                        }
                        () = notified => {
                            debug!("Conversion worker: woken by notify");
                        }
                        () = async {
                            match retry_delay {
                                Some(delay) => tokio::time::sleep(delay).await,
                                None => std::future::pending().await,
                            }
                        } => {
                            debug!("Conversion worker: backoff elapsed");
                        }
                    }
                }
            }
            .instrument(span),
        );
    }

    /// Run one queued task to completion or to its next retry.
    async fn process_task(&self, task: &ConversionTask) {
        match task {
            ConversionTask::PerReceive(payment_id) => {
                match self.process_per_receive(payment_id.clone()).await {
                    PerReceiveResult::Done { converted } => {
                        debug!(
                            "Conversion worker: completed task {task:?} (converted={converted})"
                        );
                        self.core.queue.complete_task(task).await;
                        if converted {
                            // Clear any pending auto-convert — the local balance
                            // is stale until sync completes. The next Synced event
                            // will re-queue auto-convert if there's still excess.
                            self.core.queue.clear_pending_auto_convert().await;
                            self.emit_conversion_completed().await;
                        }
                    }
                    PerReceiveResult::Retry => {
                        // Mark as deferred so next_task skips it until
                        // resolved by a PaymentSucceeded event or timeout
                        debug!("Conversion worker: deferring task {task:?}");
                        self.core.queue.defer_task(payment_id).await;
                    }
                }
            }
            ConversionTask::AutoConvert => match self.auto_convert().await {
                Ok(converted) => {
                    debug!("Conversion worker: auto-convert done (converted={converted})");
                    self.core.queue.complete_task(task).await;
                    if converted {
                        self.emit_conversion_completed().await;
                    }
                }
                Err(e) => {
                    self.schedule_retry(
                        task,
                        StableBalanceConversionKind::AutoConvert,
                        format!("{e:?}"),
                    )
                    .await;
                }
            },
            ConversionTask::Deactivation(token_id) => {
                match self.deactivation_convert(token_id).await {
                    Ok(converted) => {
                        debug!(
                            "Conversion worker: completed task {task:?} (converted={converted})"
                        );
                        self.core.queue.complete_task(task).await;
                        if converted {
                            self.emit_conversion_completed().await;
                        }
                    }
                    Err(e) => {
                        self.schedule_retry(
                            task,
                            StableBalanceConversionKind::Deactivation,
                            format!("{e:?}"),
                        )
                        .await;
                    }
                }
            }
        }
    }

    /// Process a per-receive conversion task.
    ///
    /// On failure, returns `Retry` so the task is deferred until resolved by either
    /// a `PaymentSucceeded` event (another instance completed it) or timeout expiry.
    async fn process_per_receive(&self, payment_id: String) -> PerReceiveResult {
        match self.per_receive_convert(&payment_id).await {
            Ok(converted) => {
                if converted
                    && let Err(e) = self
                        .core
                        .storage
                        .insert_payment_metadata(
                            payment_id.clone(),
                            PaymentMetadata {
                                conversion_status: Some(ConversionStatus::Completed),
                                ..Default::default()
                            },
                        )
                        .await
                {
                    warn!("Failed to persist Completed status for {payment_id}: {e:?}");
                }
                PerReceiveResult::Done { converted }
            }
            Err(e) => {
                if e.is_duplicate_transfer() {
                    info!(
                        "Per-receive conversion for {payment_id}: already handled by another instance"
                    );
                    return PerReceiveResult::Done { converted: false };
                }

                // Defer the task — it will either be resolved by a PaymentSucceeded
                // event for the deterministic transfer_id (another instance converted),
                // or cleaned up by the timeout sweep if it remains unresolved.
                warn!(
                    "Per-receive conversion failed for {payment_id}, deferring until next sync: {e:?}"
                );
                PerReceiveResult::Retry
            }
        }
    }

    /// A failed auto-convert or deactivation: keep it queued under backoff and
    /// tell the integrator. The AMM refunds a failed swap, so balances are
    /// unchanged; what the user sees without this is a toggle that reads "off"
    /// over a token holding that never moved, or a sweep that silently isn't.
    async fn schedule_retry(
        &self,
        task: &ConversionTask,
        conversion: StableBalanceConversionKind,
        error: String,
    ) {
        let (retry_in_secs, failures) = self.core.queue.retry_later(task).await;
        warn!(
            "{conversion:?} conversion failed ({failures} in a row), retrying in \
             {retry_in_secs}s: {error}"
        );
        self.event_emitter
            .emit(&SdkEvent::StableBalanceConversionFailed {
                conversion,
                error,
                retry_in_secs,
            })
            .await;
    }

    /// Recover pending per-receive conversions from a previous session.
    ///
    /// Loads persisted pending conversions and restores them into the queue.
    /// Stale deferred tasks are cleaned up by `clear_expired_tasks()` on the
    /// first `Synced` event. A deactivation that had not yet moved the token
    /// back to bitcoin is re-queued too.
    async fn recover_pending_conversions(&self) {
        let cache = ObjectCacheRepository::new(self.core.storage.clone());
        match cache.fetch_pending_deactivation().await {
            Ok(Some(token_identifier)) => {
                info!("Recovering pending deactivation conversion for {token_identifier}");
                self.core.queue.push_deactivation(token_identifier).await;
            }
            Ok(None) => {}
            Err(e) => warn!("Failed to load pending deactivation for recovery: {e:?}"),
        }
        match cache.fetch_pending_conversions().await {
            Ok(Some(pending)) => {
                if !pending.is_empty() {
                    info!(
                        "Recovering {} pending conversion(s) from previous session",
                        pending.len()
                    );
                    let mut state = self.core.queue.state.lock().await;
                    for entry in pending {
                        if state
                            .per_receive
                            .iter()
                            .any(|p| p.payment_id == entry.payment_id)
                        {
                            continue;
                        }
                        state.per_receive.push(entry);
                    }
                    self.core.queue.persist_pending(&state).await;
                }
            }
            Ok(None) => {}
            Err(e) => {
                warn!("Failed to load pending conversions for recovery: {e:?}");
            }
        }
    }
}

#[cfg(test)]
mod backoff_tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_thirty_seconds_and_caps_at_an_hour() {
        assert_eq!(backoff_secs(1), 30);
        assert_eq!(backoff_secs(2), 60);
        assert_eq!(backoff_secs(3), 120);
        assert_eq!(backoff_secs(7), 1_920);
        assert_eq!(backoff_secs(8), 3_600);
        assert_eq!(backoff_secs(9), 3_600);
        // No overflow however long the outage.
        assert_eq!(backoff_secs(u32::MAX), 3_600);
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod queue_tests {
    use super::*;
    use crate::persist::sqlite::SqliteStorage;
    use std::path::PathBuf;

    fn create_temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("breez-test-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn queue(name: &str) -> (ConversionQueue, Arc<dyn Storage>) {
        let storage: Arc<dyn Storage> =
            Arc::new(SqliteStorage::new(&create_temp_dir(name)).unwrap());
        (ConversionQueue::new(Arc::clone(&storage)), storage)
    }

    #[tokio::test]
    async fn a_failed_auto_convert_stays_queued_but_is_not_due_until_the_backoff_elapses() {
        let (queue, _storage) = queue("auto_convert_backoff");
        queue.push_auto_convert().await;
        assert!(matches!(
            queue.next_task().await,
            Some(ConversionTask::AutoConvert)
        ));
        assert_eq!(queue.retry_delay().await, None);

        let (delay, failures) = queue.retry_later(&ConversionTask::AutoConvert).await;
        assert_eq!((delay, failures), (30, 1));
        // Still queued, so a fresh trigger collapses into it rather than
        // starting a parallel attempt...
        queue.push_auto_convert().await;
        // ...but it is not handed out until it is due.
        assert!(queue.next_task().await.is_none());
        let remaining = queue.retry_delay().await.expect("under backoff");
        assert!(remaining <= Duration::from_secs(30) && remaining > Duration::from_secs(25));

        // Each further failure doubles the wait.
        assert_eq!(
            queue.retry_later(&ConversionTask::AutoConvert).await,
            (60, 2)
        );
        assert_eq!(
            queue.retry_later(&ConversionTask::AutoConvert).await,
            (120, 3)
        );

        // Success clears everything.
        queue.complete_task(&ConversionTask::AutoConvert).await;
        assert_eq!(queue.retry_delay().await, None);
        queue.push_auto_convert().await;
        assert!(queue.next_task().await.is_some());
        assert_eq!(
            queue.retry_later(&ConversionTask::AutoConvert).await,
            (30, 1)
        );
    }

    #[tokio::test]
    async fn a_deactivation_runs_now_and_is_persisted_until_it_succeeds() {
        let (queue, storage) = queue("deactivation_persist");
        let cache = ObjectCacheRepository::new(Arc::clone(&storage));
        let token = "btkn1usdb".to_string();

        // An auto-convert under backoff does not delay the user's deactivation.
        queue.push_auto_convert().await;
        let _ = queue.retry_later(&ConversionTask::AutoConvert).await;
        queue.push_deactivation(token.clone()).await;
        assert!(matches!(
            queue.next_task().await,
            Some(ConversionTask::Deactivation(ref t)) if *t == token
        ));
        assert_eq!(queue.retry_delay().await, None);
        assert_eq!(
            cache.fetch_pending_deactivation().await.unwrap().as_deref(),
            Some("btkn1usdb")
        );

        // A failure keeps it queued, backed off, and still persisted.
        let task = ConversionTask::Deactivation(token.clone());
        assert_eq!(queue.retry_later(&task).await, (30, 1));
        assert!(queue.next_task().await.is_none());
        assert_eq!(
            cache.fetch_pending_deactivation().await.unwrap().as_deref(),
            Some("btkn1usdb")
        );

        // Success forgets it.
        queue.complete_task(&task).await;
        assert_eq!(cache.fetch_pending_deactivation().await.unwrap(), None);
        assert_eq!(queue.retry_delay().await, None);
    }

    #[tokio::test]
    async fn clearing_the_queue_drops_the_persisted_deactivation_and_backoff() {
        // The user re-activates a token while its deactivation is still being
        // retried: the holding is wanted as the token again, so stop trying.
        let (queue, storage) = queue("deactivation_cleared");
        let cache = ObjectCacheRepository::new(Arc::clone(&storage));
        queue.push_deactivation("btkn1usdb".to_string()).await;
        let _ = queue
            .retry_later(&ConversionTask::Deactivation("btkn1usdb".to_string()))
            .await;

        queue.clear_queue().await;
        assert_eq!(cache.fetch_pending_deactivation().await.unwrap(), None);
        assert_eq!(queue.retry_delay().await, None);
        assert!(queue.next_task().await.is_none());
    }

    #[tokio::test]
    async fn a_failure_reported_for_a_replaced_task_does_not_back_off_the_new_one() {
        let (queue, _storage) = queue("replaced_task");
        queue.push_auto_convert().await;
        // The deactivation arrived while the auto-convert was running...
        queue.push_deactivation("btkn1usdb".to_string()).await;
        // ...and the auto-convert then failed.
        assert_eq!(
            queue.retry_later(&ConversionTask::AutoConvert).await,
            (0, 0)
        );
        assert!(queue.next_task().await.is_some(), "deactivation is due now");
        assert_eq!(queue.retry_delay().await, None);
    }
}
