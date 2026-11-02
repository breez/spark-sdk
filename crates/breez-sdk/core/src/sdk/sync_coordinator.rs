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
    /// Cancellation-safe: dropping the future stops only the wait, never the loop.
    pub async fn trigger_sync_and_wait(
        &self,
        sync_type: SyncType,
        force: bool,
    ) -> Result<(), SdkError> {
        let (tx, rx) = oneshot::channel();
        self.trigger(sync_type, force, Some(tx)).await;

        rx.await
            .map_err(|_| SdkError::Generic("Sync completion channel closed".to_string()))?
    }

    /// Trigger a sync without waiting (fire-and-forget).
    ///
    /// Uses the same coalescing mechanism but doesn't block the caller.
    pub async fn trigger_sync_no_wait(&self, sync_type: SyncType, force: bool) {
        self.trigger(sync_type, force, None).await;
    }

    /// Queue a waiter, and start the sync loop if none is running.
    ///
    /// The loop runs on its own task, never on the caller's: a loop dropped
    /// mid-sync would leave `sync_running` set, and every later sync would wait
    /// forever. No await point separates claiming the loop from spawning it.
    async fn trigger(
        &self,
        sync_type: SyncType,
        force: bool,
        sender: Option<oneshot::Sender<Result<(), SdkError>>>,
    ) {
        if self.add_waiter(sync_type, force, sender).await {
            let coordinator = self.clone();
            platform_utils::spawn(async move {
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

#[cfg(test)]
mod tests {
    use macros::async_test_all;
    use platform_utils::time::Duration;

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[async_test_all]
    async fn a_caller_dropped_mid_sync_does_not_block_later_syncs() {
        let coordinator = SyncCoordinator::new();
        let (release, held) = oneshot::channel::<()>();
        // Stands in for the runtime loop: holds the first sync until released,
        // and replies to the rest at once.
        let mut requests = coordinator.subscribe();
        platform_utils::spawn(async move {
            let mut held = Some(held);
            while let Ok(request) = requests.recv().await {
                if let Some(held) = held.take() {
                    let _ = held.await;
                }
                request.reply(None).await;
            }
        });

        let dropped = tokio::time::timeout(
            Duration::from_millis(50),
            coordinator.trigger_sync_and_wait(SyncType::Full, true),
        )
        .await;
        assert!(dropped.is_err(), "the held sync outlasts the timeout");
        release.send(()).unwrap();

        tokio::time::timeout(
            Duration::from_secs(2),
            coordinator.trigger_sync_and_wait(SyncType::Full, true),
        )
        .await
        .expect("a sync after the dropped caller completes")
        .unwrap();
    }
}
