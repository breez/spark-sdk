//! Sync request coordinator with coalescing.
//!
//! Coalesces multiple sync requests of the same type: if requests arrive while
//! a sync is running, they share a single NEW sync that starts after the current
//! one completes. Different sync types are processed in order.

use platform_utils::tokio;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast, oneshot};
use tracing::debug;

use super::{SyncRequest, SyncType};
use crate::error::SdkError;

struct Waiter {
    sync_type: SyncType,
    force: bool,
    /// None for fire-and-forget requests
    sender: Option<oneshot::Sender<Result<(), SdkError>>>,
}

#[derive(Clone)]
pub(crate) struct SyncCoordinator {
    sender: broadcast::Sender<SyncRequest>,
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    sync_running: bool,
    waiters: Vec<Waiter>,
}

impl SyncCoordinator {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(10);
        Self {
            sender,
            inner: Arc::new(Mutex::new(Inner {
                sync_running: false,
                waiters: Vec::new(),
            })),
        }
    }

    /// Get a receiver to listen for sync requests (for the sync loop).
    pub fn subscribe(&self) -> broadcast::Receiver<SyncRequest> {
        self.sender.subscribe()
    }

    /// Trigger a sync and wait for completion.
    ///
    /// Guarantees the result is from a sync that started AFTER this call.
    /// Multiple concurrent callers with the same `sync_type` share the same sync.
    pub async fn trigger_sync_and_wait(
        &self,
        sync_type: SyncType,
        force: bool,
    ) -> Result<(), SdkError> {
        let (tx, rx) = oneshot::channel();

        let should_run = self.add_waiter(sync_type, force, Some(tx)).await;

        if should_run {
            self.run_sync_loop().await;
        }

        rx.await
            .map_err(|_| SdkError::Generic("Sync completion channel closed".to_string()))?
    }

    /// Trigger a sync without waiting (fire-and-forget).
    ///
    /// Uses the same coalescing mechanism but doesn't block the caller.
    pub async fn trigger_sync_no_wait(&self, sync_type: SyncType, force: bool) {
        let should_run = self.add_waiter(sync_type, force, None).await;

        if should_run {
            let coordinator = self.clone();
            tokio::spawn(async move {
                coordinator.run_sync_loop().await;
            });
        }
    }

    /// Add a waiter and return whether this caller should run the sync loop.
    async fn add_waiter(
        &self,
        sync_type: SyncType,
        force: bool,
        sender: Option<oneshot::Sender<Result<(), SdkError>>>,
    ) -> bool {
        let mut inner = self.inner.lock().await;
        inner.waiters.push(Waiter {
            sync_type,
            force,
            sender,
        });
        if inner.sync_running {
            false
        } else {
            inner.sync_running = true;
            true
        }
    }

    /// Runs syncs in a loop until no more waiters remain.
    /// Processes waiters of the same `sync_type` together, in order.
    async fn run_sync_loop(&self) {
        loop {
            // Take waiters matching the first waiter's sync_type
            let (sync_type, force, batch_senders) = {
                let mut inner = self.inner.lock().await;
                if inner.waiters.is_empty() {
                    inner.sync_running = false;
                    return;
                }

                // Use first waiter's sync_type as the batch type
                let batch_type = inner.waiters[0].sync_type.clone();
                let mut batch_force = false;
                let mut batch_senders = Vec::new();
                let mut remaining = Vec::new();

                for waiter in inner.waiters.drain(..) {
                    if waiter.sync_type == batch_type {
                        batch_force |= waiter.force;
                        if let Some(sender) = waiter.sender {
                            batch_senders.push(sender);
                        }
                    } else {
                        remaining.push(waiter);
                    }
                }

                inner.waiters = remaining;
                (batch_type, batch_force, batch_senders)
            };

            debug!(
                "Running sync type {:?} for {} waiters",
                sync_type,
                batch_senders.len()
            );

            // Run the sync
            let result = self.run_single_sync(sync_type, force).await;

            // Notify all waiters from this batch
            for sender in batch_senders {
                let _ = sender.send(result.clone());
            }
        }
    }

    /// Run a single sync operation.
    async fn run_single_sync(&self, sync_type: SyncType, force: bool) -> Result<(), SdkError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let request = SyncRequest {
            sync_type,
            reply: Arc::new(tokio::sync::Mutex::new(Some(reply_tx))),
            force,
        };

        self.sender
            .send(request)
            .map_err(|e| SdkError::Generic(format!("Failed to trigger sync: {e}")))?;

        reply_rx
            .await
            .map_err(|_| SdkError::Generic("Sync reply channel closed".to_string()))?
    }
}
