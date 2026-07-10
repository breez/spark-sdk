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

    /// Like [`headers`](Self::headers), but forces any cached authentication to
    /// refresh before returning. Request layers call this after the server
    /// rejects a request with an auth error (HTTP 401 / gRPC `Unauthenticated`)
    /// so a stale-but-unexpired cached token is re-minted instead of failing
    /// until its TTL. The default delegates to `headers`; auth providers
    /// override it to bypass their session cache and re-authenticate.
    async fn headers_refresh(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        self.headers().await
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
        merge(try_join_all(self.providers.iter().map(|p| p.headers())).await?)
    }

    async fn headers_refresh(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        merge(try_join_all(self.providers.iter().map(|p| p.headers_refresh())).await?)
    }
}

/// Merges header maps left to right; later providers win on key collisions.
fn merge(
    results: Vec<HashMap<String, String>>,
) -> Result<HashMap<String, String>, HeaderProviderError> {
    let mut merged = HashMap::new();
    for headers in results {
        merged.extend(headers);
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use macros::async_test_all;

    use super::*;

    /// Counts `headers` vs `headers_refresh` calls and tags its emitted header
    /// with `name`, so tests can assert which method ran and how the merge
    /// resolves collisions.
    #[derive(Default)]
    struct CountingProvider {
        name: String,
        headers_calls: AtomicUsize,
        refresh_calls: AtomicUsize,
    }

    impl CountingProvider {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                ..Default::default()
            }
        }
    }

    #[macros::async_trait]
    impl HeaderProvider for CountingProvider {
        async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
            self.headers_calls.fetch_add(1, Ordering::SeqCst);
            Ok(HashMap::from([("x-who".to_string(), self.name.clone())]))
        }
    }

    #[async_test_all]
    async fn default_headers_refresh_delegates_to_headers() {
        let provider = CountingProvider::new("a");
        provider.headers_refresh().await.unwrap();
        assert_eq!(provider.headers_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.refresh_calls.load(Ordering::SeqCst), 0);
    }

    #[async_test_all]
    async fn combined_headers_refresh_fans_out_to_children() {
        let a = Arc::new(CountingProvider::new("a"));
        let b = Arc::new(CountingProvider::new("b"));
        let combined = CombinedHeaderProvider::new(vec![a.clone(), b.clone()]);

        combined.headers_refresh().await.unwrap();

        // Neither child overrides headers_refresh, so each falls back to headers.
        assert_eq!(a.headers_calls.load(Ordering::SeqCst), 1);
        assert_eq!(b.headers_calls.load(Ordering::SeqCst), 1);
    }
}
