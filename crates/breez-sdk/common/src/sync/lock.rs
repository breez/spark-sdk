use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio_with_wasm::alias as tokio;
use tracing::{debug, warn};

/// Parameters for acquiring or releasing a distributed lock.
pub struct SetLockParams {
    pub lock_name: String,
    pub acquire: bool,
    pub exclusive: bool,
}

/// Trait for distributed lock operations via the sync service.
///
/// Used to coordinate actions across multiple SDK instances sharing the same identity.
/// All operations are best-effort — callers handle errors gracefully.
#[macros::async_trait]
pub trait SyncLockClient: Send + Sync {
    async fn set_lock(&self, params: SetLockParams) -> anyhow::Result<()>;
    async fn is_locked(&self, lock_name: &str) -> anyhow::Result<bool>;
}

/// Tracks the number of in-flight lock holders sharing a distributed lock.
pub struct LockCounter {
    count: AtomicU64,
}

impl LockCounter {
    pub fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
        }
    }

    pub fn increment(&self) -> u64 {
        self.count.fetch_add(1, Ordering::Release).saturating_add(1)
    }

    pub fn decrement(&self) -> u64 {
        self.count.fetch_sub(1, Ordering::Release).saturating_sub(1)
    }

    pub fn get(&self) -> u64 {
        self.count.load(Ordering::Acquire)
    }
}

impl Default for LockCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard for a distributed lock.
///
/// Increments a shared counter on creation and decrements on drop. When the
/// last guard is dropped and no holders remain, the distributed lock is
/// released (if configured).
pub struct SyncLockGuard {
    lock_name: String,
    counter: Arc<LockCounter>,
    sync_lock_client: Option<Arc<dyn SyncLockClient>>,
}

impl SyncLockGuard {
    /// Creates a new guard, incrementing the counter.
    ///
    /// For non-exclusive acquires, the distributed lock is acquired
    /// fire-and-forget (best effort). For exclusive acquires, use
    /// [`Self::new_exclusive`] which returns an error if the lock is
    /// already held by another instance.
    pub fn new(
        lock_name: String,
        counter: Arc<LockCounter>,
        sync_lock_client: Option<Arc<dyn SyncLockClient>>,
    ) -> Self {
        let count = counter.increment();
        debug!("Lock guard acquired for '{}' (holders: {count})", lock_name);

        // Best-effort acquire (fire-and-forget)
        if let Some(client) = &sync_lock_client {
            let client = Arc::clone(client);
            let name = lock_name.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .set_lock(SetLockParams {
                        lock_name: name,
                        acquire: true,
                        exclusive: false,
                    })
                    .await
                {
                    warn!("Failed to acquire distributed lock: {e:?}");
                }
            });
        }

        Self {
            lock_name,
            counter,
            sync_lock_client,
        }
    }

    /// Creates a new exclusive guard.
    ///
    /// Uses its own internal counter since there is only ever one exclusive
    /// holder. Returns `Err` if another instance already holds the lock.
    pub async fn new_exclusive(
        lock_name: String,
        sync_lock_client: Option<Arc<dyn SyncLockClient>>,
    ) -> anyhow::Result<Self> {
        if let Some(client) = &sync_lock_client {
            client
                .set_lock(SetLockParams {
                    lock_name: lock_name.clone(),
                    acquire: true,
                    exclusive: true,
                })
                .await?;
        }

        let counter = Arc::new(LockCounter::new());
        counter.increment();
        debug!("Exclusive lock guard acquired for '{lock_name}'");

        Ok(Self {
            lock_name,
            counter,
            sync_lock_client,
        })
    }
}

impl Drop for SyncLockGuard {
    fn drop(&mut self) {
        let remaining = self.counter.decrement();
        debug!(
            "Lock guard released for '{}' (holders: {remaining})",
            self.lock_name
        );

        // Best-effort release of the distributed lock when no holders remain
        if remaining == 0
            && let Some(sync_lock_client) = self.sync_lock_client.take()
        {
            let lock_name = self.lock_name.clone();
            tokio::spawn(async move {
                if let Err(e) = sync_lock_client
                    .set_lock(SetLockParams {
                        lock_name,
                        acquire: false,
                        exclusive: false,
                    })
                    .await
                {
                    warn!("Failed to release distributed lock: {e:?}");
                }
            });
        }
    }
}
