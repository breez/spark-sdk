//! Unified conversion queue and worker for stable balance.
//!
//! Serializes all conversion tasks (per-receive, auto-convert, and deactivation)
//! through a single queue to eliminate race conditions between the paths.

use std::sync::Arc;

use platform_utils::{
    time::{SystemTime, UNIX_EPOCH},
    tokio,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify, watch};
use tracing::{Instrument, debug, info, warn};

use crate::models::ConversionStatus;
use crate::persist::{ObjectCacheRepository, PaymentMetadata, Storage};

use super::{StableBalance, per_receive_transfer_id};

pub(super) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

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

/// How often the worker wakes to re-check the queue (e.g. for debounce polling).
const WORKER_POLL_INTERVAL_SECS: u64 = 20;

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
}

/// A priority queue that serializes conversion tasks.
///
/// Per-receive tasks always execute before auto-convert. Items remain in the
/// queue while being processed (dequeue after completion) so that dedup and
/// collapse continue to work during processing.
pub(crate) struct ConversionQueue {
    state: Mutex<ConversionQueueState>,
    notify: Notify,
    storage: Arc<dyn Storage>,
}

impl ConversionQueue {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            state: Mutex::new(ConversionQueueState {
                per_receive: Vec::new(),
                pending_task: None,
            }),
            notify: Notify::new(),
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

    /// Queue a deactivation conversion task. Overrides any pending auto-convert.
    pub async fn push_deactivation(&self, token_identifier: String) {
        let mut state = self.state.lock().await;
        debug!("Queuing deactivation conversion for token {token_identifier}");
        state.pending_task = Some(ConversionTask::Deactivation(token_identifier));
        self.notify.notify_one();
    }

    /// Clear all pending tasks from the queue.
    /// Returns the payment IDs of any cleared per-receive tasks (for status updates).
    pub async fn clear_queue(&self) -> Vec<String> {
        let mut state = self.state.lock().await;
        let cleared: Vec<String> = state.per_receive.drain(..).map(|p| p.payment_id).collect();
        state.pending_task = None;
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
            state.pending_task.clone()
        } else {
            None
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
            ConversionTask::AutoConvert | ConversionTask::Deactivation(_) => {
                state.pending_task = None;
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
    pub(super) fn spawn_conversion_worker(&self, mut shutdown_receiver: watch::Receiver<()>) {
        let stable_balance = self.clone();
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                // Pre-warm effective values cache
                if let Some(token_id) = stable_balance.get_active_token_identifier().await
                    && let Err(e) = stable_balance.get_or_init_effective_values(&token_id).await
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
                    () = stable_balance.synced_notify.notified() => {
                        debug!("Conversion worker: initial sync completed");
                    }
                }

                // Cold-start: queue auto-convert for any existing excess balance
                stable_balance.queue.push_auto_convert().await;

                // Main processing loop
                debug!("Conversion worker: entering main loop");
                loop {
                    // Register notify future BEFORE checking the queue to avoid missed wakeups
                    let notified = stable_balance.queue.notify.notified();

                    // Drain all available tasks
                    while let Some(task) = stable_balance.queue.next_task().await {
                        debug!("Conversion worker: processing task {task:?}");
                        match &task {
                            ConversionTask::PerReceive(payment_id) => {
                                match stable_balance
                                    .process_per_receive(payment_id.clone())
                                    .await
                                {
                                    PerReceiveResult::Done { converted } => {
                                        debug!("Conversion worker: completed task {task:?} (converted={converted})");
                                        stable_balance.queue.complete_task(&task).await;
                                        if converted {
                                            stable_balance.trigger_sync().await;
                                        }
                                    }
                                    PerReceiveResult::Retry => {
                                        // Mark as deferred so next_task skips it until
                                        // resolved by a PaymentSucceeded event or timeout
                                        debug!("Conversion worker: deferring task {task:?}");
                                        stable_balance.queue.defer_task(payment_id).await;
                                    }
                                }
                            }
                            ConversionTask::AutoConvert => {
                                use super::conversions::AutoConvertResult;
                                match stable_balance.debounced_auto_convert().await {
                                    Ok(AutoConvertResult::Done { converted }) => {
                                        debug!("Conversion worker: auto-convert done (converted={converted})");
                                        stable_balance.queue.complete_task(&task).await;
                                        if converted {
                                            stable_balance.trigger_sync().await;
                                        }
                                    }
                                    Ok(AutoConvertResult::Debounced) => {
                                        debug!("Conversion worker: auto-convert debounce deferred");
                                        break;
                                    }
                                    Err(e) => {
                                        warn!("Auto-conversion failed: {e:?}");
                                        stable_balance.queue.complete_task(&task).await;
                                    }
                                }
                            }
                            ConversionTask::Deactivation(token_id) => {
                                let converted =
                                    match stable_balance.deactivation_convert(token_id).await {
                                        Ok(converted) => converted,
                                        Err(e) => {
                                            warn!("Deactivation conversion failed: {e:?}");
                                            false
                                        }
                                    };
                                debug!("Conversion worker: completed task {task:?} (converted={converted})");
                                stable_balance.queue.complete_task(&task).await;
                                if converted {
                                    stable_balance.trigger_sync().await;
                                }
                            }
                        }
                    }

                    debug!("Conversion worker: queue drained, waiting for new tasks");
                    tokio::select! {
                        _ = shutdown_receiver.changed() => {
                            info!("Conversion worker shutdown");
                            return;
                        }
                        () = notified => {
                            debug!("Conversion worker: woken by notify");
                        }
                        () = tokio::time::sleep(std::time::Duration::from_secs(WORKER_POLL_INTERVAL_SECS)) => {
                            debug!("Conversion worker: periodic wake");
                        }
                    }
                }
            }
            .instrument(span),
        );
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

    /// Recover pending per-receive conversions from a previous session.
    ///
    /// Loads persisted pending conversions and restores them into the queue.
    /// Stale deferred tasks are cleaned up by `clear_expired_tasks()` on the
    /// first `Synced` event.
    async fn recover_pending_conversions(&self) {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        match cache.fetch_pending_conversions().await {
            Ok(Some(pending)) => {
                if !pending.is_empty() {
                    info!(
                        "Recovering {} pending conversion(s) from previous session",
                        pending.len()
                    );
                    let mut state = self.queue.state.lock().await;
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
                    self.queue.persist_pending(&state).await;
                }
            }
            Ok(None) => {}
            Err(e) => {
                warn!("Failed to load pending conversions for recovery: {e:?}");
            }
        }
    }
}
