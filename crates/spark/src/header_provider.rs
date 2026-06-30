use std::collections::HashMap;
use std::sync::Arc;

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
}
