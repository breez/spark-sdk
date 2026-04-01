use std::collections::HashSet;
use std::sync::Arc;

use platform_utils::tokio;
use tokio::sync::{Mutex, mpsc, watch};

use crate::api::ws::{SwapStatusSubscriber, SwapStatusUpdate};
use crate::error::BoltzError;
use crate::events::{BoltzSwapEvent, EventEmitter};
use crate::models::{BoltzSwap, BoltzSwapStatus};
use crate::recover;
use crate::swap::reverse::{ReverseSwapExecutor, current_unix_timestamp};

/// Maximum number of receipt-poll attempts for a `Claiming` swap (5s * 60 = 5min).
/// If the receipt is still not found after this, the task exits and relies on
/// the WS `transaction.claimed` message. On process restart, `resume_all`
/// re-triggers the poll, so this is self-healing across restarts.
const RECEIPT_POLL_MAX_ATTEMPTS: u32 = 60;
/// Interval between receipt-poll attempts.
const RECEIPT_POLL_INTERVAL_SECS: u64 = 5;

/// Background swap manager.
///
/// Owns a single event loop that:
/// - Receives WebSocket status updates for all tracked swaps.
/// - Progresses each swap through its state machine.
/// - Spawns short-lived tasks for heavy operations (claiming, receipt polling).
pub(crate) struct SwapManager {
    /// Channel for sending swap IDs to track.
    cmd_tx: mpsc::Sender<String>,
    /// Shutdown signal — dropping the sender stops the event loop.
    shutdown_tx: watch::Sender<()>,
    task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Sync-safe handle used by `Drop` to abort the task if `shutdown()` was
    /// never called.
    abort_handle: tokio::task::AbortHandle,
}

impl SwapManager {
    /// Create the manager and spawn its central event loop.
    ///
    /// `ws_rx` is the global receiver for all WebSocket status updates.
    pub fn start(
        executor: Arc<ReverseSwapExecutor>,
        event_emitter: Arc<EventEmitter>,
        ws_subscriber: Arc<SwapStatusSubscriber>,
        ws_rx: mpsc::Receiver<SwapStatusUpdate>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (shutdown_tx, shutdown_rx) = watch::channel(());

        let handle = tokio::spawn(Self::run_loop(
            executor,
            event_emitter,
            ws_subscriber,
            ws_rx,
            cmd_rx,
            shutdown_rx,
        ));

        let abort_handle = handle.abort_handle();

        Self {
            cmd_tx,
            shutdown_tx,
            task_handle: Mutex::new(Some(handle)),
            abort_handle,
        }
    }

    /// Begin tracking a swap. The manager will subscribe to WS updates for it
    /// and progress it through the state machine.
    pub async fn track_swap(&self, swap_id: &str) {
        let _ = self.cmd_tx.send(swap_id.to_string()).await;
    }

    /// Resume all non-terminal swaps from the store.
    pub async fn resume_all(
        &self,
        executor: &ReverseSwapExecutor,
    ) -> Result<Vec<String>, BoltzError> {
        let active = executor.store.list_active_swaps().await?;
        let mut ids = Vec::with_capacity(active.len());
        for swap in &active {
            tracing::info!(swap_id = swap.id, status = ?swap.status, "Resuming swap");
            self.track_swap(&swap.id).await;
            ids.push(swap.id.clone());
        }
        Ok(ids)
    }

    /// Signal the event loop to shut down and wait for it to exit.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
        if let Some(handle) = self.task_handle.lock().await.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for SwapManager {
    fn drop(&mut self) {
        self.abort_handle.abort();
    }
}

impl SwapManager {
    // ─── Central event loop ─────────────────────────────────────────

    async fn run_loop(
        executor: Arc<ReverseSwapExecutor>,
        event_emitter: Arc<EventEmitter>,
        ws_subscriber: Arc<SwapStatusSubscriber>,
        mut ws_rx: mpsc::Receiver<SwapStatusUpdate>,
        mut cmd_rx: mpsc::Receiver<String>,
        mut shutdown_rx: watch::Receiver<()>,
    ) {
        // Swap IDs currently being tracked (for WS dispatch filtering).
        let mut tracked_ids: HashSet<String> = HashSet::new();
        // Track spawned claim/poll tasks so we don't duplicate work.
        let active_claims: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => break,
                update = ws_rx.recv() => {
                    let Some(update) = update else { break };
                    if !tracked_ids.contains(&update.swap_id) {
                        tracing::warn!(boltz_id = update.swap_id, "WS update for untracked swap");
                        continue;
                    }
                    Self::handle_ws_update(
                        &executor,
                        &event_emitter,
                        &ws_subscriber,
                        &active_claims,
                        &mut tracked_ids,
                        &update,
                    ).await;
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(swap_id) => {
                            if let Err(e) = Self::start_tracking(
                                &ws_subscriber,
                                &mut tracked_ids,
                                &swap_id,
                            ).await {
                                tracing::error!(swap_id, error = %e, "Failed to start tracking swap");
                            }
                        }
                        None => break,
                    }
                }
            }
        }

        tracing::info!("SwapManager event loop exiting");
    }

    /// Begin tracking a specific swap: subscribe to WS and wait for the
    /// backend to send the current status. The WS update will drive any
    /// needed action via `handle_ws_update` — we don't act on local state
    /// here because another instance may have progressed the swap.
    async fn start_tracking(
        ws_subscriber: &Arc<SwapStatusSubscriber>,
        tracked_ids: &mut HashSet<String>,
        swap_id: &str,
    ) -> Result<(), BoltzError> {
        tracked_ids.insert(swap_id.to_string());
        ws_subscriber.subscribe(swap_id).await?;
        Ok(())
    }

    /// Process a WS status update for a tracked swap.
    async fn handle_ws_update(
        executor: &Arc<ReverseSwapExecutor>,
        event_emitter: &Arc<EventEmitter>,
        ws_subscriber: &Arc<SwapStatusSubscriber>,
        active_claims: &Arc<Mutex<HashSet<String>>>,
        tracked_ids: &mut HashSet<String>,
        update: &SwapStatusUpdate,
    ) {
        let swap_id = &update.swap_id;
        let swap = match executor.store.get_swap(swap_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(swap_id, "WS update for unknown swap");
                return;
            }
            Err(e) => {
                tracing::error!(swap_id, error = %e, "Failed to load swap for WS update");
                return;
            }
        };

        if swap.status.is_terminal() {
            tracing::debug!(swap_id, status = ?swap.status, "Swap already terminal, cleaning up");
            Self::cleanup_terminal(ws_subscriber, tracked_ids, swap_id).await;
            return;
        }

        tracing::info!(
            swap_id,
            local_status = ?swap.status,
            ws_status = update.status,
            "Processing WS update"
        );

        match update.status.as_str() {
            "swap.created" | "invoice.set" | "invoice.pending" => {}
            "invoice.paid" => {
                Self::update_status(executor, event_emitter, &swap, BoltzSwapStatus::InvoicePaid)
                    .await;
            }
            "transaction.mempool" => {
                if let Some(tx) = &update.transaction {
                    let mut s = swap;
                    s.lockup_tx_id = Some(tx.id.clone());
                    s.updated_at = current_unix_timestamp();
                    if let Err(e) = executor.store.update_swap(&s).await {
                        tracing::error!(swap_id, error = %e, "Failed to persist lockup_tx_id");
                    }
                    event_emitter
                        .emit(&BoltzSwapEvent::SwapUpdated { swap: s })
                        .await;
                }
            }
            "transaction.confirmed" => {
                // If we already submitted a claim, don't overwrite local
                // status — just let the claiming resume logic handle it.
                if matches!(swap.status, BoltzSwapStatus::Claiming) {
                    Self::handle_claiming_resume(executor, event_emitter, active_claims, &swap);
                } else {
                    // tBTC locked on-chain. Update local status, then claim.
                    let mut s = swap.clone();
                    if let Some(tx) = &update.transaction {
                        s.lockup_tx_id = Some(tx.id.clone());
                    }
                    s.status = BoltzSwapStatus::TbtcLocked;
                    s.updated_at = current_unix_timestamp();
                    if let Err(e) = executor.store.update_swap(&s).await {
                        tracing::error!(swap_id, error = %e, "Failed to persist TbtcLocked status");
                    }
                    event_emitter
                        .emit(&BoltzSwapEvent::SwapUpdated { swap: s.clone() })
                        .await;
                    Self::spawn_claim(executor, event_emitter, active_claims, &s, false);
                }
            }
            // `invoice.settled`: reverse swap success (Boltz settled the hold
            //   invoice after detecting our on-chain claim).
            // `transaction.claimed`: submarine/chain swap success (included
            //   for completeness, not expected for reverse swaps).
            //
            // TODO: Before marking Completed, verify the claim TX receipt
            // on-chain (is_success). Currently we trust the Boltz backend
            // status without independent verification. Additionally, parse
            // Transfer event logs from the receipt to record the actual USDT
            // amount delivered (may differ from estimate due to slippage).
            "invoice.settled" | "transaction.claimed" => {
                Self::update_status(executor, event_emitter, &swap, BoltzSwapStatus::Completed)
                    .await;
                Self::cleanup_terminal(ws_subscriber, tracked_ids, swap_id).await;
            }
            "invoice.expired" | "swap.expired" => {
                Self::update_status(executor, event_emitter, &swap, BoltzSwapStatus::Expired).await;
                Self::cleanup_terminal(ws_subscriber, tracked_ids, swap_id).await;
            }
            "invoice.failedToPay"
            | "transaction.lockupFailed"
            | "transaction.refunded"
            | "swap.refunded" => {
                let reason = update
                    .failure_reason
                    .clone()
                    .unwrap_or_else(|| update.status.clone());
                Self::update_status(
                    executor,
                    event_emitter,
                    &swap,
                    BoltzSwapStatus::Failed { reason },
                )
                .await;
                Self::cleanup_terminal(ws_subscriber, tracked_ids, swap_id).await;
            }
            _ => {
                tracing::debug!(
                    swap_id,
                    ws_status = update.status,
                    "Unknown WS status, ignoring"
                );
            }
        }
    }

    /// Spawn a short-lived claim task for a `TbtcLocked` swap.
    fn spawn_claim(
        executor: &Arc<ReverseSwapExecutor>,
        event_emitter: &Arc<EventEmitter>,
        active_claims: &Arc<Mutex<HashSet<String>>>,
        swap: &BoltzSwap,
        skip_drift_check: bool,
    ) {
        let swap_id = swap.id.clone();
        let executor = executor.clone();
        let emitter = event_emitter.clone();
        let claims = active_claims.clone();

        tokio::spawn(async move {
            // Prevent duplicate claim tasks for the same swap.
            {
                let mut set = claims.lock().await;
                if set.contains(&swap_id) {
                    tracing::debug!(swap_id, "Claim already in progress, skipping");
                    return;
                }
                set.insert(swap_id.clone());
            }

            let result = Self::do_claim(&executor, &swap_id, skip_drift_check).await;
            match result {
                Ok(swap) => {
                    emitter.emit(&BoltzSwapEvent::SwapUpdated { swap }).await;
                }
                Err(BoltzError::QuoteDegradedBeyondSlippage {
                    expected_usdt,
                    quoted_usdt,
                }) => {
                    tracing::warn!(
                        swap_id,
                        expected_usdt,
                        quoted_usdt,
                        "Claim-time quote degraded beyond slippage tolerance"
                    );
                    if let Ok(Some(swap)) = executor.store.get_swap(&swap_id).await {
                        emitter
                            .emit(&BoltzSwapEvent::QuoteDegraded {
                                swap,
                                expected_usdt,
                                quoted_usdt,
                            })
                            .await;
                    }
                }
                Err(e) => {
                    tracing::error!(swap_id, error = %e, "Claim failed");
                    // claim_and_swap marks the swap as Failed in the store on
                    // final retry failure. Emit the event so listeners learn.
                    if let Ok(Some(swap)) = executor.store.get_swap(&swap_id).await {
                        emitter.emit(&BoltzSwapEvent::SwapUpdated { swap }).await;
                    }
                }
            }

            claims.lock().await.remove(&swap_id);
        });
    }

    /// Execute the claim flow for a swap.
    async fn do_claim(
        executor: &ReverseSwapExecutor,
        swap_id: &str,
        skip_drift_check: bool,
    ) -> Result<BoltzSwap, BoltzError> {
        let mut swap = executor
            .store
            .get_swap(swap_id)
            .await?
            .ok_or_else(|| BoltzError::Store(format!("Swap not found: {swap_id}")))?;

        executor.claim_and_swap(&mut swap, skip_drift_check).await
    }

    /// Handle resuming a swap stuck in `Claiming` status. Either the tx hash
    /// is known (poll chain for receipt) or unknown (check on-chain if preimage
    /// was revealed).
    fn handle_claiming_resume(
        executor: &Arc<ReverseSwapExecutor>,
        event_emitter: &Arc<EventEmitter>,
        active_claims: &Arc<Mutex<HashSet<String>>>,
        swap: &BoltzSwap,
    ) {
        if let Some(ref tx_hash) = swap.claim_tx_hash {
            // We have a tx hash — poll chain for its receipt.
            Self::spawn_receipt_poll(
                executor,
                event_emitter,
                active_claims,
                swap.id.clone(),
                tx_hash.clone(),
            );
        } else {
            // Crash during Alchemy call: we set Claiming but never got a tx
            // hash back. Check on-chain if the claim went through anyway.
            Self::spawn_on_chain_check(executor, event_emitter, active_claims, swap.clone());
        }
    }

    /// Spawn a task that polls `eth_get_transaction_receipt` for a known tx
    /// hash. If the receipt shows success, mark `Completed`. If reverted,
    /// mark `Failed`.
    fn spawn_receipt_poll(
        executor: &Arc<ReverseSwapExecutor>,
        event_emitter: &Arc<EventEmitter>,
        active_claims: &Arc<Mutex<HashSet<String>>>,
        swap_id: String,
        tx_hash: String,
    ) {
        let executor = executor.clone();
        let emitter = event_emitter.clone();
        let claims = active_claims.clone();

        tokio::spawn(async move {
            {
                let mut set = claims.lock().await;
                if set.contains(&swap_id) {
                    return;
                }
                set.insert(swap_id.clone());
            }

            for attempt in 0..RECEIPT_POLL_MAX_ATTEMPTS {
                match executor
                    .evm_provider
                    .eth_get_transaction_receipt(&tx_hash)
                    .await
                {
                    Ok(Some(receipt)) => {
                        if receipt.is_success() {
                            tracing::info!(swap_id, tx_hash, "Claim receipt confirmed");
                            if let Ok(Some(swap)) = executor.store.get_swap(&swap_id).await {
                                Self::update_status(
                                    &executor,
                                    &emitter,
                                    &swap,
                                    BoltzSwapStatus::Completed,
                                )
                                .await;
                            }
                        } else {
                            tracing::error!(swap_id, tx_hash, "Claim tx reverted");
                            if let Ok(Some(swap)) = executor.store.get_swap(&swap_id).await {
                                Self::update_status(
                                    &executor,
                                    &emitter,
                                    &swap,
                                    BoltzSwapStatus::Failed {
                                        reason: "Claim transaction reverted".to_string(),
                                    },
                                )
                                .await;
                            }
                        }
                        claims.lock().await.remove(&swap_id);
                        return;
                    }
                    Ok(None) => {
                        // Not mined yet.
                        if attempt < RECEIPT_POLL_MAX_ATTEMPTS.saturating_sub(1) {
                            platform_utils::tokio::time::sleep(
                                platform_utils::time::Duration::from_secs(
                                    RECEIPT_POLL_INTERVAL_SECS,
                                ),
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(swap_id, attempt, error = %e, "Receipt poll failed");
                        platform_utils::tokio::time::sleep(
                            platform_utils::time::Duration::from_secs(RECEIPT_POLL_INTERVAL_SECS),
                        )
                        .await;
                    }
                }
            }

            // Timed out — rely on WS `transaction.claimed` to complete.
            // On process restart, `resume_all` re-triggers the poll.
            tracing::warn!(swap_id, tx_hash, "Receipt poll timed out, waiting for WS");
            claims.lock().await.remove(&swap_id);
        });
    }

    /// Spawn a task that checks on-chain whether the preimage was already
    /// revealed. If still locked, retry the claim. If already claimed, wait
    /// for WS `transaction.claimed`.
    fn spawn_on_chain_check(
        executor: &Arc<ReverseSwapExecutor>,
        event_emitter: &Arc<EventEmitter>,
        active_claims: &Arc<Mutex<HashSet<String>>>,
        swap: BoltzSwap,
    ) {
        let executor = executor.clone();
        let emitter = event_emitter.clone();
        let claims = active_claims.clone();
        let swap_id = swap.id.clone();

        tokio::spawn(async move {
            {
                let mut set = claims.lock().await;
                if set.contains(&swap_id) {
                    return;
                }
                set.insert(swap_id.clone());
            }

            match recover::is_swap_still_locked_by_swap(
                &executor.evm_provider,
                &swap,
                &executor.key_manager,
            )
            .await
            {
                Ok(true) => {
                    // Still locked — safe to retry claim.
                    tracing::info!(swap_id, "Swap still locked on-chain, retrying claim");
                    // Reset to TbtcLocked so claim_and_swap can proceed.
                    let mut s = swap;
                    s.status = BoltzSwapStatus::TbtcLocked;
                    s.updated_at = current_unix_timestamp();
                    if let Err(e) = executor.store.update_swap(&s).await {
                        tracing::error!(swap_id, error = %e, "Failed to persist TbtcLocked reset");
                    }
                    match executor.claim_and_swap(&mut s, false).await {
                        Ok(s) => {
                            emitter.emit(&BoltzSwapEvent::SwapUpdated { swap: s }).await;
                        }
                        Err(BoltzError::QuoteDegradedBeyondSlippage {
                            expected_usdt,
                            quoted_usdt,
                        }) => {
                            tracing::warn!(
                                swap_id = s.id,
                                expected_usdt,
                                quoted_usdt,
                                "Claim-time quote degraded beyond slippage on resume"
                            );
                            if let Ok(Some(swap)) = executor.store.get_swap(&s.id).await {
                                emitter
                                    .emit(&BoltzSwapEvent::QuoteDegraded {
                                        swap,
                                        expected_usdt,
                                        quoted_usdt,
                                    })
                                    .await;
                            }
                        }
                        Err(e) => {
                            tracing::error!(swap_id = s.id, error = %e, "Retry claim failed");
                            // claim_and_swap marks Failed in store. Emit so
                            // listeners learn about the failure.
                            if let Ok(Some(failed)) = executor.store.get_swap(&s.id).await {
                                emitter
                                    .emit(&BoltzSwapEvent::SwapUpdated { swap: failed })
                                    .await;
                            }
                        }
                    }
                }
                Ok(false) => {
                    // Already claimed — just wait for WS `transaction.claimed`.
                    tracing::info!(
                        swap_id,
                        "Swap already claimed on-chain, waiting for WS confirmation"
                    );
                }
                Err(e) => {
                    tracing::error!(swap_id, error = %e, "On-chain check failed");
                }
            }

            claims.lock().await.remove(&swap_id);
        });
    }

    /// Update a swap's status, persist, and emit an event.
    async fn update_status(
        executor: &ReverseSwapExecutor,
        emitter: &EventEmitter,
        swap: &BoltzSwap,
        new_status: BoltzSwapStatus,
    ) {
        let mut s = swap.clone();
        s.status = new_status;
        s.updated_at = current_unix_timestamp();
        if let Err(e) = executor.store.update_swap(&s).await {
            tracing::error!(swap_id = s.id, error = %e, "Failed to update swap status");
        }
        emitter.emit(&BoltzSwapEvent::SwapUpdated { swap: s }).await;
    }

    /// Unsubscribe from WS and remove from tracking set after a swap
    /// reaches a terminal state.
    async fn cleanup_terminal(
        ws_subscriber: &SwapStatusSubscriber,
        tracked_ids: &mut HashSet<String>,
        swap_id: &str,
    ) {
        ws_subscriber.unsubscribe(swap_id).await;
        tracked_ids.remove(swap_id);
    }
}
