use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::future::try_join_all;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum HeaderProviderError {
    #[error("{0}")]
    Generic(String),
}

#[macros::async_trait]
pub trait HeaderProvider: Send + Sync {
    async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError>;

    /// Force a fresh authentication on the next request, discarding any cached
    /// session. Default no-op for providers without a session.
    async fn reauthenticate(&self) -> Result<(), HeaderProviderError> {
        Ok(())
    }
}

/// Composes multiple [`HeaderProvider`]s by fanning out their `headers()`
/// calls in parallel and merging the results. On key collisions, later
/// providers win.
pub struct CombinedHeaderProvider {
    providers: Vec<Arc<dyn HeaderProvider>>,
}

impl CombinedHeaderProvider {
    pub fn new(providers: Vec<Arc<dyn HeaderProvider>>) -> Self {
        Self { providers }
    }
}

#[macros::async_trait]
impl HeaderProvider for CombinedHeaderProvider {
    async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        let results = try_join_all(self.providers.iter().map(|p| p.headers())).await?;
        let mut merged = HashMap::new();
        for headers in results {
            merged.extend(headers);
        }
        Ok(merged)
    }

    async fn reauthenticate(&self) -> Result<(), HeaderProviderError> {
        try_join_all(self.providers.iter().map(|p| p.reauthenticate())).await?;
        Ok(())
    }
}

/// Coalesces concurrent re-authentications so a single session-invalidation
/// wave triggers one `authenticate()`, not one per in-flight call.
#[derive(Default)]
pub(crate) struct ReauthGuard {
    lock: tokio::sync::Mutex<()>,
    generation: AtomicU64,
}

impl ReauthGuard {
    /// Runs `reauth` unless another caller already completed one since this call
    /// began (the generation advanced while waiting for the lock), in which case
    /// the fresh session that caller stored is reused and `reauth` is skipped.
    pub(crate) async fn run<F, Fut>(&self, reauth: F) -> Result<(), HeaderProviderError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), HeaderProviderError>>,
    {
        let generation = self.generation.load(Ordering::Acquire);
        let _guard = self.lock.lock().await;
        if self.generation.load(Ordering::Acquire) != generation {
            return Ok(());
        }
        reauth().await?;
        self.generation.fetch_add(1, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct CountingProvider {
        reauth_calls: AtomicUsize,
    }

    #[macros::async_trait]
    impl HeaderProvider for CountingProvider {
        async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
            Ok(HashMap::new())
        }

        async fn reauthenticate(&self) -> Result<(), HeaderProviderError> {
            self.reauth_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[macros::async_test_all]
    async fn combined_reauthenticate_fans_out_to_every_provider() {
        let a = Arc::new(CountingProvider::default());
        let b = Arc::new(CountingProvider::default());
        let combined = CombinedHeaderProvider::new(vec![a.clone(), b.clone()]);

        combined.reauthenticate().await.unwrap();

        assert_eq!(a.reauth_calls.load(Ordering::SeqCst), 1);
        assert_eq!(b.reauth_calls.load(Ordering::SeqCst), 1);
    }

    #[macros::async_test_all]
    async fn reauth_guard_collapses_concurrent_wave_to_one_run() {
        let guard = ReauthGuard::default();
        let runs = AtomicUsize::new(0);
        let release = tokio::sync::Notify::new();

        // The holder enters `reauth` and parks holding the lock until released.
        let holder = guard.run(|| async {
            runs.fetch_add(1, Ordering::SeqCst);
            release.notified().await;
            Ok(())
        });
        // Contenders capture the holder's generation and queue on the lock. Once
        // the holder is released and bumps the generation, each sees it changed
        // and skips. `join` polls left-to-right on the single-threaded test
        // runtime, so the holder parks on the lock before the contenders run.
        let contend = || {
            guard.run(|| async {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        };
        let contenders = async {
            release.notify_one();
            let (r1, r2, r3) = futures::future::join3(contend(), contend(), contend()).await;
            r1.unwrap();
            r2.unwrap();
            r3.unwrap();
        };

        let (holder_res, _) = futures::future::join(holder, contenders).await;
        holder_res.unwrap();
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "one invalidation wave must collapse to a single authenticate"
        );
    }
}
