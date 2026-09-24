//! Unified conversion queue and worker for stable balance.
//!
//! Serializes all conversion tasks (per-receive, auto-convert, and deactivation)
//! through a single queue to eliminate race conditions between the paths.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use platform_utils::tokio;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify, watch};
use tokio::time::sleep;
use tracing::{Instrument, debug, info, warn};

use crate::events::{SdkEvent, StableBalanceConversionKind};
use crate::models::ConversionStatus;
use crate::persist::{ObjectCacheRepository, PaymentMetadata, Storage};
use crate::token_conversion::ConversionError;
use crate::utils::payments::insert_payment_metadata_and_emit;
use crate::utils::time::now_secs;

use super::{StableBalance, per_receive_transfer_id};

/// A conversion task to be processed by the worker.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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

/// First wait after a failed conversion, doubling on each further failure.
const CONVERSION_BACKOFF_BASE_SECS: u64 = 30;

/// Ceiling on the retry delay, and the age past which persisted state is
/// discarded. Caps a permanently failing pair at roughly 30 attempts a day.
const CONVERSION_BACKOFF_CAP_SECS: u64 = 3600;

/// A swap that ran and then failed delivered the conversion, so it settles as
/// converted rather than as a failure.
fn settle_swap_that_ran(result: Result<bool, ConversionError>) -> Result<bool, ConversionError> {
    match result {
        Err(ConversionError::FailedAfterSwap(e)) => {
            warn!("Conversion ran, then failed: {e}");
            Ok(true)
        }
        result => result,
    }
}

/// How long to wait after `consecutive_failures` failures.
fn backoff_secs(consecutive_failures: u32) -> u64 {
    let doublings = consecutive_failures.saturating_sub(1).min(20);
    CONVERSION_BACKOFF_BASE_SECS
        .saturating_mul(1u64 << doublings)
        .min(CONVERSION_BACKOFF_CAP_SECS)
}

/// How long the worker waits before retrying after a failed conversion.
///
/// One counter covers per-receive, auto-convert and deactivation, since they
/// usually convert against the one active token and a failure in one predicts
/// a failure in the others. A deactivation still owed for a previously active
/// token shares it too, so its failures delay the active token's conversions.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct ConversionBackoff {
    consecutive_failures: u32,
    /// Unix timestamp of the most recent failure. Zero when closed.
    last_failure_at: u64,
}

impl ConversionBackoff {
    /// Remaining wait, or `None` when a task may run now.
    fn retry_delay(&self) -> Option<Duration> {
        if self.consecutive_failures == 0 {
            return None;
        }
        let wait = backoff_secs(self.consecutive_failures);
        let remaining = self
            .last_failure_at
            .saturating_add(wait)
            .saturating_sub(now_secs());
        // Clamped because `last_failure_at` comes from the wall clock: a clock
        // that jumped backwards, or a device that booted without one, would
        // otherwise yield a wait of decades that nothing here can clear.
        match remaining.min(wait) {
            0 => None,
            remaining => Some(Duration::from_secs(remaining)),
        }
    }

    fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.last_failure_at = now_secs();
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    /// Whether this state is unusable: recorded more than a cap ago, so the
    /// next failure would start at the ceiling, or stamped in the future by a
    /// clock that has since been corrected.
    fn is_stale(&self) -> bool {
        if self.consecutive_failures == 0 {
            return false;
        }
        let now = now_secs();
        now.saturating_sub(self.last_failure_at) > CONVERSION_BACKOFF_CAP_SECS
            || self.last_failure_at > now
    }
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

/// The queue as persisted, so a restart restores both halves together and
/// they cannot disagree about what is still pending.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct PendingQueue {
    pub(super) per_receive: Vec<PendingConversion>,
    /// Tokens awaiting conversion back to bitcoin. A list because two can be
    /// pending at once, and dropping either strands that balance.
    pub(super) deactivations: Vec<String>,
}

impl<'de> Deserialize<'de> for PendingQueue {
    /// Restores either the current queue shape or the previous per-receive
    /// only pending conversions.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Queue {
                #[serde(default)]
                per_receive: Vec<PendingConversion>,
                #[serde(default)]
                deactivations: Vec<String>,
            },
            PerReceiveOnly(Vec<PendingConversion>),
        }
        Ok(match Stored::deserialize(deserializer)? {
            Stored::Queue {
                per_receive,
                deactivations,
            } => PendingQueue {
                per_receive,
                deactivations,
            },
            Stored::PerReceiveOnly(per_receive) => PendingQueue {
                per_receive,
                deactivations: Vec::new(),
            },
        })
    }
}

/// Internal state of the conversion queue.
struct ConversionQueueState {
    /// Ordered list of pending per-receive conversions.
    per_receive: Vec<PendingConversion>,
    /// Tokens awaiting conversion back to bitcoin, oldest first.
    deactivations: Vec<String>,
    /// Whether a batch sweep of excess bitcoin is due.
    auto_convert_pending: bool,
    /// Delay applied to every task after a failed conversion.
    backoff: ConversionBackoff,
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
                deactivations: Vec::new(),
                auto_convert_pending: false,
                backoff: ConversionBackoff::default(),
            }),
            notify: Arc::new(Notify::new()),
            storage,
        }
    }

    /// A queue holding what the previous session left pending, and its retry
    /// delay. Built before the queue is shared, so no token change or arriving
    /// payment can overwrite the saved state before it is read.
    pub async fn load(storage: Arc<dyn Storage>) -> Self {
        let queue = Self::new(Arc::clone(&storage));
        let cache = ObjectCacheRepository::new(storage);
        match cache.fetch_pending_conversions().await {
            Ok(Some(saved)) => {
                if !saved.per_receive.is_empty() || !saved.deactivations.is_empty() {
                    info!(
                        "Recovering {} pending conversion(s) and {:?} from previous session",
                        saved.per_receive.len(),
                        saved.deactivations
                    );
                }
                queue.restore_pending(saved).await;
            }
            Ok(None) => {}
            Err(e) => warn!("Failed to load pending conversions for recovery: {e:?}"),
        }
        match cache.fetch_conversion_backoff().await {
            Ok(Some(backoff)) => queue.restore_backoff(backoff).await,
            Ok(None) => {}
            Err(e) => warn!("Failed to load conversion backoff for recovery: {e:?}"),
        }
        queue
    }

    /// How long until the next task may run, or `None` if one may run now.
    pub async fn retry_delay(&self) -> Option<Duration> {
        self.state.lock().await.backoff.retry_delay()
    }

    /// Record a failed conversion. Returns how long until the next attempt is
    /// due, and how many failures have now run consecutively.
    pub async fn record_failure(&self) -> (u64, u32) {
        let mut state = self.state.lock().await;
        state.backoff.record_failure();
        let failures = state.backoff.consecutive_failures;
        self.persist_backoff(&state).await;
        (backoff_secs(failures), failures)
    }

    /// Reset the failure count to zero, so the next task is due immediately.
    pub async fn clear_backoff(&self) {
        let mut state = self.state.lock().await;
        state.backoff.reset();
        self.persist_backoff(&state).await;
        self.notify.notify_one();
    }

    /// Install the delay saved by a previous session, dropping it if stale.
    async fn restore_backoff(&self, backoff: ConversionBackoff) {
        if backoff.is_stale() {
            debug!("Discarding stale conversion backoff: {backoff:?}");
            let cache = ObjectCacheRepository::new(self.storage.clone());
            if let Err(e) = cache.delete_conversion_backoff().await {
                warn!("Failed to delete stale conversion backoff: {e:?}");
            }
            return;
        }
        self.state.lock().await.backoff = backoff;
    }

    async fn persist_backoff(&self, state: &ConversionQueueState) {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let result = if state.backoff == ConversionBackoff::default() {
            cache.delete_conversion_backoff().await
        } else {
            cache.save_conversion_backoff(&state.backoff).await
        };
        if let Err(e) = result {
            warn!("Failed to persist conversion backoff: {e:?}");
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
        if !state.auto_convert_pending {
            state.auto_convert_pending = true;
            self.notify.notify_one();
        }
    }

    /// Queue a deactivation conversion task. Overrides any pending auto-convert.
    pub async fn push_deactivation(&self, token_identifier: String) {
        let mut state = self.state.lock().await;
        if state.deactivations.contains(&token_identifier) {
            return;
        }
        debug!("Queuing deactivation conversion for token {token_identifier}");
        state.deactivations.push(token_identifier);
        self.persist_pending(&state).await;
        self.notify.notify_one();
    }

    /// Clear a pending auto-convert task if one exists.
    /// Used after a successful per-receive conversion to prevent auto-convert
    /// from running with a stale balance before the next sync completes.
    pub async fn clear_pending_auto_convert(&self) {
        let mut state = self.state.lock().await;
        state.auto_convert_pending = false;
    }

    /// Install the queue saved by a previous session.
    ///
    /// Entries keep the creation time they were saved with, so one that
    /// predates a restored failure is gated like any other stale entry.
    async fn restore_pending(&self, queue: PendingQueue) {
        let mut state = self.state.lock().await;
        state.per_receive = queue.per_receive;
        state.deactivations = queue.deactivations;
    }

    /// Returns `true` if there are any per-receive tasks in the queue.
    /// Used by auto-convert to yield to per-receive tasks that arrived while it was preparing.
    /// Matches `next_task()` semantics: auto-convert should not run while any per-receive
    /// tasks exist (ready or deferred), since deferred tasks may still need those sats.
    pub async fn has_per_receive(&self) -> bool {
        let state = self.state.lock().await;
        !state.per_receive.is_empty()
    }

    /// Drops the work queued for the token being left: every per-receive task,
    /// the batch sweep, and a deactivation owed for the token being activated.
    /// A deactivation owed for any other token is kept. Returns the payment ids
    /// of the cleared per-receive tasks.
    pub async fn clear_for_token_change(&self, new_token: Option<&str>) -> Vec<String> {
        let mut state = self.state.lock().await;
        let cleared: Vec<String> = state.per_receive.drain(..).map(|p| p.payment_id).collect();
        state.auto_convert_pending = false;
        // A deactivation for any other token still has to run: dropping it
        // strands that balance with nothing left to convert it back.
        if let Some(new_token) = new_token {
            state.deactivations.retain(|pending| pending != new_token);
        }
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
    /// Skips deferred per-receive tasks, and yields nothing while the delay
    /// from a previous failure runs.
    #[cfg(all(test, feature = "sqlite"))]
    pub async fn next_task(&self) -> Option<ConversionTask> {
        self.next_task_or_delay().await.ok()?
    }

    /// The next task, or `Err(remaining)` when one is queued but the retry
    /// delay has not elapsed. Both come from a single read of the clock, so a
    /// caller cannot see "delayed" here and "not delayed" a moment later and
    /// park itself with no timer armed.
    pub async fn next_task_or_delay(&self) -> Result<Option<ConversionTask>, Duration> {
        let state = self.state.lock().await;
        // Every task waits out the delay, a newly received payment included.
        if let Some(remaining) = state.backoff.retry_delay() {
            return Err(remaining);
        }
        let ready = state
            .per_receive
            .iter()
            .find(|p| p.state != PendingState::Deferred);
        if let Some(pending) = ready {
            return Ok(Some(ConversionTask::PerReceive(pending.payment_id.clone())));
        }
        // Only run auto-convert/deactivation when no per-receive tasks exist (including
        // deferred). Deferred tasks may still be resolved by a PaymentSucceeded event
        // and need those sats.
        if !state.per_receive.is_empty() {
            return Ok(None);
        }
        // A deactivation is a user action, so it outranks the batch sweep.
        if let Some(token) = state.deactivations.first() {
            return Ok(Some(ConversionTask::Deactivation(token.clone())));
        }
        Ok(state
            .auto_convert_pending
            .then_some(ConversionTask::AutoConvert))
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
            ConversionTask::AutoConvert => state.auto_convert_pending = false,
            ConversionTask::Deactivation(token) => {
                state.deactivations.retain(|pending| pending != token);
                self.persist_pending(&state).await;
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
        if state.per_receive.is_empty() && state.deactivations.is_empty() {
            if let Err(e) = cache.delete_pending_conversions().await {
                warn!("Failed to delete pending conversions cache: {e:?}");
            }
            return;
        }
        let queue = PendingQueue {
            per_receive: state.per_receive.clone(),
            deactivations: state.deactivations.clone(),
        };
        if let Err(e) = cache.save_pending_conversions(&queue).await {
            warn!("Failed to persist pending conversions: {e:?}");
        }
    }
}

impl StableBalance {
    /// Spawns the unified conversion worker that processes all conversion tasks.
    ///
    /// The worker:
    /// 1. Waits for the initial sync to complete
    /// 2. Queues a cold-start auto-convert
    /// 3. Processes tasks serially (per-receive first, then auto-convert)
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

                    // Drain all available tasks. The delay comes back from the
                    // same call that withheld the task, so the sleep below
                    // cannot miss a deadline that passed between two reads.
                    let retry_delay = loop {
                        match stable_balance.core.queue.next_task_or_delay().await {
                            Ok(Some(task)) => stable_balance.process_task(&task).await,
                            Ok(None) => break None,
                            Err(remaining) => break Some(remaining),
                        }
                    };

                    debug!(
                        "Conversion worker: queue drained, waiting for new tasks (retry in {retry_delay:?})"
                    );
                    tokio::select! {
                        _ = shutdown_receiver.changed() => {
                            info!("Conversion worker shutdown");
                            return;
                        }
                        () = notified => {
                            debug!("Conversion worker: woken by notify");
                        }
                        () = sleep(retry_delay.unwrap_or_default()), if retry_delay.is_some() => {
                            debug!("Conversion worker: woken by backoff expiry");
                        }
                    }
                }
            }
            .instrument(span),
        );
    }

    /// Run one dequeued task and settle it on the queue.
    async fn process_task(&self, task: &ConversionTask) {
        debug!("Conversion worker: processing task {task:?}");
        let converted = match task {
            ConversionTask::PerReceive(payment_id) => {
                match self.process_per_receive(payment_id.clone()).await {
                    PerReceiveResult::Done { converted } => {
                        self.core.queue.complete_task(task).await;
                        if converted {
                            // Clear any pending auto-convert: the local balance
                            // is stale until sync completes. The next Synced event
                            // will re-queue auto-convert if there's still excess.
                            self.core.queue.clear_pending_auto_convert().await;
                        }
                        converted
                    }
                    PerReceiveResult::Retry => {
                        // Mark as deferred so next_task skips it until
                        // resolved by a PaymentSucceeded event or timeout
                        debug!("Conversion worker: deferring task {task:?}");
                        self.core.queue.defer_task(payment_id).await;
                        return;
                    }
                }
            }
            ConversionTask::AutoConvert => {
                // Settled either way. `auto_convert` reads a local balance
                // snapshot, so retrying it without a sync in between can send
                // sats that are already at the pool. `Synced` re-queues it, and
                // the retry delay decides when it may run.
                let converted = self
                    .try_conversion(
                        StableBalanceConversionKind::AutoConvert,
                        self.auto_convert(),
                    )
                    .await
                    .unwrap_or(false);
                self.core.queue.complete_task(task).await;
                converted
            }
            ConversionTask::Deactivation(token_id) => {
                let Ok(converted) = self
                    .try_conversion(
                        StableBalanceConversionKind::Deactivation,
                        self.deactivation_convert(token_id),
                    )
                    .await
                else {
                    return;
                };
                self.core.queue.complete_task(task).await;
                // Cleared on any settled outcome, not just a conversion. There
                // is nothing left to convert when the token balance is zero or
                // below the pool minimum, and keeping it in the saved queue
                // would run it again on every start.
                converted
            }
        };

        debug!("Conversion worker: completed task {task:?} (converted={converted})");
        if converted {
            self.emit_conversion_completed().await;
        }
    }

    /// Await a conversion, recording its outcome against the retry delay.
    /// `Ok(false)` means it settled with nothing to convert.
    async fn try_conversion(
        &self,
        kind: StableBalanceConversionKind,
        conversion: impl Future<Output = Result<bool, ConversionError>>,
    ) -> Result<bool, ()> {
        match settle_swap_that_ran(conversion.await) {
            Ok(converted) => {
                self.record_conversion_success(converted).await;
                Ok(converted)
            }
            Err(e) => {
                self.record_conversion_failure(kind, &e).await;
                Err(())
            }
        }
    }

    /// Process a per-receive conversion task.
    ///
    /// On failure, returns `Retry` so the task is deferred until resolved by either
    /// a `PaymentSucceeded` event (another instance completed it) or timeout expiry.
    async fn process_per_receive(&self, payment_id: String) -> PerReceiveResult {
        match settle_swap_that_ran(self.per_receive_convert(&payment_id).await) {
            Ok(converted) => {
                if converted
                    && let Err(e) = insert_payment_metadata_and_emit(
                        &self.core.storage,
                        &self.event_emitter,
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
                self.record_conversion_success(converted).await;
                PerReceiveResult::Done { converted }
            }
            Err(e) => {
                if e.is_duplicate_transfer() {
                    info!(
                        "Per-receive conversion for {payment_id}: already handled by another instance"
                    );
                    return PerReceiveResult::Done { converted: false };
                }

                self.record_conversion_failure(StableBalanceConversionKind::PerReceive, &e)
                    .await;

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

    /// Delay the next attempt and emit `StableBalanceConversionFailed`, so a
    /// conversion that keeps failing is visible rather than only slow.
    async fn record_conversion_failure(
        &self,
        conversion: StableBalanceConversionKind,
        error: &ConversionError,
    ) {
        let (delay_secs, failures) = self.core.queue.record_failure().await;
        // A per-receive is deferred, not retried: its sats go to the batch
        // sweep, which reports its own failures.
        let retry_in_secs =
            (conversion != StableBalanceConversionKind::PerReceive).then_some(delay_secs);
        warn!(
            "{conversion:?} conversion failed ({failures} in a row), conversions delayed {delay_secs}s: {error:?}"
        );
        self.event_emitter
            .emit(&SdkEvent::StableBalanceConversionFailed {
                conversion,
                error: error.to_string(),
                retry_in_secs,
            })
            .await;
    }

    /// Reset the delay once a conversion has actually run. A skipped
    /// conversion leaves it alone: declining to convert says nothing about
    /// whether the pool works.
    async fn record_conversion_success(&self, converted: bool) {
        if converted {
            self.core.queue.clear_backoff().await;
        }
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::persist::sqlite::SqliteStorage;

    fn temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("breez-test-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn queue(name: &str) -> (ConversionQueue, Arc<dyn Storage>) {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir(name)).unwrap());
        (ConversionQueue::new(Arc::clone(&storage)), storage)
    }

    #[test]
    fn a_swap_that_ran_settles_as_converted() {
        let ran = settle_swap_that_ran(Err(ConversionError::FailedAfterSwap("x".to_string())));
        assert!(matches!(ran, Ok(true)));

        let failed = settle_swap_that_ran(Err(ConversionError::ConversionFailed("x".to_string())));
        assert!(matches!(failed, Err(ConversionError::ConversionFailed(_))));

        assert!(matches!(settle_swap_that_ran(Ok(false)), Ok(false)));
    }

    #[test]
    fn backoff_doubles_from_thirty_seconds_and_caps_at_an_hour() {
        assert_eq!(backoff_secs(1), 30);
        assert_eq!(backoff_secs(2), 60);
        assert_eq!(backoff_secs(3), 120);
        assert_eq!(backoff_secs(7), 1_920);
        assert_eq!(backoff_secs(8), 3_600);
        assert_eq!(backoff_secs(9), 3_600);
        // The doubling must not overflow on a long-running failure streak.
        assert_eq!(backoff_secs(u32::MAX), 3_600);
    }

    /// The schedule allows seven attempts in an hour, and the queue withholds a
    /// task straight after its failure.
    #[tokio::test]
    async fn the_schedule_allows_seven_attempts_an_hour() {
        let (queue, _storage) = queue("backoff_bounds_attempts");

        let mut attempts = 0;
        let mut clock = 0u64;
        let mut backoff = ConversionBackoff::default();
        // Walk an hour of wall time, re-queueing the task as often as a sync
        // would and counting the attempts the queue actually hands out.
        while clock <= 3_600 {
            let due = backoff.consecutive_failures == 0
                || clock
                    >= backoff
                        .last_failure_at
                        .saturating_add(backoff_secs(backoff.consecutive_failures));
            if due {
                attempts += 1;
                backoff.consecutive_failures = backoff.consecutive_failures.saturating_add(1);
                backoff.last_failure_at = clock;
            }
            clock += 10;
        }
        assert_eq!(
            attempts, 7,
            "roughly 470 an hour before the backoff, 7 after"
        );

        // And the queue agrees that a fresh failure is not immediately due.
        queue.push_auto_convert().await;
        assert!(queue.next_task().await.is_some());
        queue.record_failure().await;
        assert!(
            queue.next_task().await.is_none(),
            "a failed task must not be handed straight back"
        );
    }

    #[tokio::test]
    async fn a_failed_task_stays_queued_and_reports_its_remaining_wait() {
        let (queue, _storage) = queue("backoff_reports_delay");

        queue.push_auto_convert().await;
        assert!(queue.retry_delay().await.is_none());

        let (retry_in_secs, failures) = queue.record_failure().await;
        assert_eq!((retry_in_secs, failures), (30, 1));

        // Still queued, just not due: a second push must not start a parallel attempt.
        queue.push_auto_convert().await;
        assert!(queue.next_task().await.is_none());
        let remaining = queue.retry_delay().await.expect("under backoff");
        assert!(remaining <= Duration::from_secs(30) && remaining > Duration::from_secs(25));

        // Each further failure doubles the wait.
        assert_eq!(queue.record_failure().await, (60, 2));
        assert_eq!(queue.record_failure().await, (120, 3));
    }

    /// Per-receive is gated by the same delay, not only auto-convert.
    #[tokio::test]
    async fn a_failed_conversion_also_gates_per_receive() {
        let (queue, _storage) = queue("backoff_gates_per_receive");

        queue.push_per_receive("payment-1".to_string()).await;
        assert!(queue.next_task().await.is_some());

        queue.record_failure().await;
        assert!(
            queue.next_task().await.is_none(),
            "per-receive must be gated too: the pool is failing, not the payment"
        );
    }

    #[tokio::test]
    async fn a_successful_conversion_closes_the_breaker() {
        let (queue, _storage) = queue("backoff_cleared_on_success");

        queue.push_auto_convert().await;
        queue.record_failure().await;
        assert!(queue.next_task().await.is_none());

        queue.clear_backoff().await;
        assert!(queue.retry_delay().await.is_none());
        assert!(queue.next_task().await.is_some());
    }

    #[tokio::test]
    async fn the_breaker_survives_a_restart() {
        let dir = temp_dir("backoff_persisted");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&dir).unwrap());

        let first = ConversionQueue::new(Arc::clone(&storage));
        first.record_failure().await;
        first.record_failure().await;

        // A new queue over the same storage, as a restarted client would build.
        let second = ConversionQueue::load(Arc::clone(&storage)).await;

        second.push_auto_convert().await;
        assert!(
            second.next_task().await.is_none(),
            "restarting must not reset the backoff and resume the loop"
        );
        assert_eq!(
            second.record_failure().await,
            (120, 3),
            "count carried over"
        );
    }

    #[tokio::test]
    async fn a_backoff_older_than_the_cap_is_discarded() {
        let (queue, storage) = queue("backoff_decays");

        let stale = ConversionBackoff {
            consecutive_failures: 8,
            last_failure_at: now_secs().saturating_sub(CONVERSION_BACKOFF_CAP_SECS + 1),
        };
        queue.restore_backoff(stale).await;

        // A user returning after a break starts clean rather than inheriting
        // an hour-long wait on their first failure.
        assert!(queue.retry_delay().await.is_none());
        assert_eq!(queue.record_failure().await, (30, 1));
        assert!(
            ObjectCacheRepository::new(storage)
                .fetch_conversion_backoff()
                .await
                .unwrap()
                .is_some_and(|b| b.consecutive_failures == 1)
        );
    }

    /// A user's own payment proving the provider works must not be made to
    /// wait out a delay the background sweeps built up.
    #[tokio::test]
    async fn a_conversion_outside_the_worker_clears_the_delay() {
        let (queue, _storage) = queue("backoff_cleared_by_user_conversion");

        queue.push_auto_convert().await;
        for _ in 0..8 {
            queue.record_failure().await;
        }
        let cap = Duration::from_secs(CONVERSION_BACKOFF_CAP_SECS);
        let remaining = queue.retry_delay().await.expect("at the cap");
        assert!(remaining <= cap && remaining > cap.saturating_sub(Duration::from_secs(10)));

        queue.clear_backoff().await;
        assert!(queue.retry_delay().await.is_none());
        assert!(
            queue.next_task().await.is_some(),
            "the sweep resumes at once"
        );
    }

    /// A payment received during the delay waits for it. A refund the SDK has
    /// not recognised arrives like any payment, so this is what stops it being
    /// retried at its own pace instead of the schedule's.
    #[tokio::test]
    async fn a_payment_received_during_the_delay_waits_for_it() {
        let (queue, _storage) = queue("arrival_waits");

        queue
            .restore_backoff(ConversionBackoff {
                consecutive_failures: 2,
                last_failure_at: now_secs().saturating_sub(5),
            })
            .await;
        queue.push_per_receive("payment-1".to_string()).await;

        assert!(queue.next_task().await.is_none());
        assert!(queue.retry_delay().await.is_some());
    }

    /// A backoff stamped by a clock that has since moved backwards must not
    /// survive: `retry_delay` would otherwise report decades and nothing in the
    /// worker could clear it.
    #[tokio::test]
    async fn a_backoff_from_the_future_is_discarded() {
        let (queue, _storage) = queue("backoff_future_timestamp");

        queue
            .restore_backoff(ConversionBackoff {
                consecutive_failures: 3,
                last_failure_at: now_secs().saturating_add(86_400),
            })
            .await;

        assert!(queue.retry_delay().await.is_none());
        assert_eq!(queue.record_failure().await, (30, 1));
    }

    /// The wait can never exceed the schedule, whatever the stored timestamp.
    #[tokio::test]
    async fn the_reported_delay_never_exceeds_the_schedule() {
        let (queue, _storage) = queue("backoff_delay_clamped");

        // Past is_stale, but still ahead of the clock.
        queue
            .restore_backoff(ConversionBackoff {
                consecutive_failures: 1,
                last_failure_at: now_secs().saturating_add(5),
            })
            .await;

        let remaining = queue.retry_delay().await;
        assert!(
            remaining.is_none_or(|d| d <= Duration::from_secs(backoff_secs(1))),
            "got {remaining:?}"
        );
    }

    /// The task and the delay must come from one read of the clock: a caller
    /// that is told "delayed" and then reads "not delayed" parks with no timer.
    #[tokio::test]
    async fn a_withheld_task_reports_the_delay_that_withheld_it() {
        let (queue, _storage) = queue("backoff_single_clock_read");

        queue.push_auto_convert().await;
        queue.record_failure().await;

        match queue.next_task_or_delay().await {
            Err(remaining) => assert!(remaining > Duration::ZERO),
            other => panic!("expected a delay, got {other:?}"),
        }
    }

    /// An entry that predates the failure waits, whether it was queued in this
    /// session or restored from the last one.
    #[tokio::test]
    async fn an_entry_older_than_the_failure_waits() {
        let (queue, _storage) = queue("stale_entry_waits");

        queue.push_per_receive("payment-1".to_string()).await;
        queue.record_failure().await;

        assert!(
            queue.next_task().await.is_none(),
            "the failure is newer than the entry"
        );
    }

    /// A failed task must stay queued, or the `retry_in_secs` the event
    /// carries names an attempt that never comes.
    #[tokio::test]
    async fn a_failed_task_is_retried_once_the_delay_elapses() {
        let (queue, _storage) = queue("failed_task_stays_queued");

        queue.push_deactivation("token-1".to_string()).await;
        let task = queue.next_task().await.expect("the deactivation");
        assert!(matches!(task, ConversionTask::Deactivation(_)));

        // The worker leaves a failed task in place, unlike a settled one.
        queue.record_failure().await;
        assert!(queue.next_task().await.is_none(), "delayed");

        queue.clear_backoff().await;
        assert!(
            matches!(
                queue.next_task().await,
                Some(ConversionTask::Deactivation(id)) if id == "token-1"
            ),
            "the same task is still there to retry"
        );
    }

    /// A settled auto-convert is not handed back without a re-queue.
    ///
    /// Only the queue half. Whether `process_task` settles a *failed*
    /// auto-convert runs through `StableBalance`, which holds a concrete
    /// `SparkWallet` and is not constructible here.
    #[tokio::test]
    async fn a_settled_auto_convert_is_not_handed_back() {
        let (queue, _storage) = queue("auto_convert_settles_on_failure");

        queue.push_auto_convert().await;
        let task = queue.next_task().await.expect("the auto-convert");
        assert!(matches!(task, ConversionTask::AutoConvert));

        queue.complete_task(&task).await;
        queue.record_failure().await;
        queue.clear_backoff().await;

        assert!(
            queue.next_task().await.is_none(),
            "only a Synced re-queue may bring it back"
        );
    }

    /// Switching to another token must not drop a deactivation pending for the
    /// one being left: nothing else would convert that balance back.
    #[tokio::test]
    async fn a_token_change_keeps_a_deactivation_for_another_token() {
        let (queue, _storage) = queue("token_change_keeps_deactivation");

        queue.push_deactivation("token-a".to_string()).await;
        queue.push_per_receive("payment-1".to_string()).await;

        let cleared = queue.clear_for_token_change(Some("token-b")).await;
        assert_eq!(cleared, vec!["payment-1".to_string()]);
        assert!(matches!(
            queue.next_task().await,
            Some(ConversionTask::Deactivation(id)) if id == "token-a"
        ));
    }

    /// Turning the same token back on cancels its pending deactivation.
    #[tokio::test]
    async fn a_token_change_drops_a_deactivation_for_the_token_being_activated() {
        let (queue, _storage) = queue("token_change_drops_own_deactivation");

        queue.push_deactivation("token-a".to_string()).await;
        queue.clear_for_token_change(Some("token-a")).await;

        assert!(queue.next_task().await.is_none());
    }

    /// The queue is persisted whole, so a restart resumes exactly what was
    /// pending, with no second marker to disagree with.
    #[tokio::test]
    async fn a_queued_deactivation_survives_a_restart() {
        let dir = temp_dir("deactivation_persisted");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&dir).unwrap());

        let first = ConversionQueue::new(Arc::clone(&storage));
        first.push_deactivation("token-a".to_string()).await;

        let second = ConversionQueue::load(Arc::clone(&storage)).await;

        assert!(matches!(
            second.next_task().await,
            Some(ConversionTask::Deactivation(id)) if id == "token-a"
        ));
    }

    /// The key kept its name when its shape changed, so a queue written by the
    /// previous release has to still load. Dropping it would lose the user's
    /// pending conversions on the first start after an upgrade.
    #[test]
    fn a_queue_saved_before_the_pending_task_was_persisted_still_loads() {
        let stored = r#"[{"payment_id":"p1","state":"Ready","created_at":100}]"#;
        let queue: PendingQueue = serde_json::from_str(stored).expect("old shape");
        assert_eq!(queue.per_receive.len(), 1);
        assert_eq!(queue.per_receive[0].payment_id, "p1");
        assert!(queue.deactivations.is_empty());

        // And the shape written now round-trips.
        let current = PendingQueue {
            per_receive: queue.per_receive.clone(),
            deactivations: vec!["token-a".to_string()],
        };
        let reloaded: PendingQueue =
            serde_json::from_str(&serde_json::to_string(&current).unwrap()).unwrap();
        assert_eq!(reloaded.per_receive.len(), 1);
        assert_eq!(reloaded.deactivations, vec!["token-a".to_string()]);
    }

    /// Two tokens can be awaiting deactivation at once. Neither may be
    /// dropped: nothing else would convert that balance back.
    #[tokio::test]
    async fn two_tokens_can_await_deactivation() {
        let (queue, _storage) = queue("two_deactivations");

        queue.push_deactivation("token-a".to_string()).await;
        queue.push_deactivation("token-b".to_string()).await;

        let first = queue.next_task().await.expect("the first pending");
        assert!(matches!(&first, ConversionTask::Deactivation(id) if id == "token-a"));
        queue.complete_task(&first).await;

        assert!(matches!(
            queue.next_task().await,
            Some(ConversionTask::Deactivation(id)) if id == "token-b"
        ));
    }

    /// A deactivation is a user's request, so it outranks the batch sweep.
    #[tokio::test]
    async fn a_deactivation_outranks_the_batch_sweep() {
        let (queue, _storage) = queue("deactivation_outranks_sweep");

        queue.push_auto_convert().await;
        queue.push_deactivation("token-a".to_string()).await;

        assert!(matches!(
            queue.next_task().await,
            Some(ConversionTask::Deactivation(id)) if id == "token-a"
        ));
    }

    /// A token change made as soon as the SDK is up must not erase a
    /// deactivation the previous session still owed for another token.
    #[tokio::test]
    async fn a_token_change_at_startup_keeps_a_saved_deactivation() {
        let dir = temp_dir("startup_token_change");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&dir).unwrap());
        ConversionQueue::new(Arc::clone(&storage))
            .push_deactivation("token-a".to_string())
            .await;

        let restarted = ConversionQueue::load(Arc::clone(&storage)).await;
        restarted.clear_for_token_change(Some("token-b")).await;

        assert!(matches!(
            restarted.next_task().await,
            Some(ConversionTask::Deactivation(id)) if id == "token-a"
        ));
        assert_eq!(
            ObjectCacheRepository::new(storage)
                .fetch_pending_conversions()
                .await
                .unwrap()
                .expect("persisted")
                .deactivations,
            vec!["token-a".to_string()],
            "and is still saved for the next restart"
        );
    }

    #[tokio::test]
    async fn clearing_the_backoff_wakes_the_worker() {
        let (queue, _storage) = queue("backoff_wakes_worker");
        queue.record_failure().await;

        let notified = queue.notify.notified();
        queue.clear_backoff().await;
        // Already-armed notify resolves immediately; a missed wake would hang.
        tokio::time::timeout(Duration::from_secs(1), notified)
            .await
            .expect("clearing the backoff must wake the worker");
    }
}
