use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use platform_utils::tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Serializes `pay` calls for the same payment hash within one SDK instance, so
/// a second call finds the first one's payment instead of sending another.
#[derive(Clone, Default)]
pub(crate) struct X402PayLocks {
    locks: Arc<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
}

impl X402PayLocks {
    /// Waits until no other `pay` holds any of `payment_hashes`, then holds
    /// them all until the returned guards are dropped.
    pub(crate) async fn lock(&self, mut payment_hashes: Vec<String>) -> Vec<X402PayLock> {
        // A fixed order, so two calls sharing hashes can't wait on each other.
        payment_hashes.sort();
        payment_hashes.dedup();
        let mut held = Vec::with_capacity(payment_hashes.len());
        for payment_hash in payment_hashes {
            let lock = self
                .entries()
                .entry(payment_hash.clone())
                .or_default()
                .clone();
            held.push(X402PayLock {
                locks: self.clone(),
                payment_hash,
                guard: Some(lock.lock_owned().await),
            });
        }
        held
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<AsyncMutex<()>>>> {
        self.locks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) struct X402PayLock {
    locks: X402PayLocks,
    payment_hash: String,
    guard: Option<OwnedMutexGuard<()>>,
}

impl Drop for X402PayLock {
    fn drop(&mut self) {
        self.guard.take();
        let mut entries = self.locks.entries();
        // Only the map's own reference is left once no one holds or awaits it.
        if entries
            .get(&self.payment_hash)
            .is_some_and(|lock| Arc::strong_count(lock) == 1)
        {
            entries.remove(&self.payment_hash);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[macros::async_test_all]
    async fn a_second_call_waits_and_the_lock_is_forgotten_after() {
        let locks = X402PayLocks::default();
        let first = locks.lock(vec!["a".to_string()]).await;
        let waiting = {
            let locks = locks.clone();
            platform_utils::spawn(async move { locks.lock(vec!["a".to_string()]).await.len() })
        };
        platform_utils::tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(first);
        assert_eq!(waiting.await.unwrap(), 1);
        assert!(locks.entries().is_empty());
    }
}
