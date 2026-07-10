use std::collections::HashMap;
use std::sync::Arc;

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

/// Composes multiple [`HeaderProvider`]s by resolving their `headers()` in
/// order and merging the results. On key collisions, later providers win.
///
/// Resolution is sequential, not parallel, so a later provider benefits from an
/// earlier one's latency: pairing an auth provider (whose first call performs a
/// challenge/response handshake) before a best-effort provider gives the latter
/// the handshake window to hydrate before its header is snapshotted.
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
        let mut merged = HashMap::new();
        for provider in &self.providers {
            merged.extend(provider.headers().await?);
        }
        Ok(merged)
    }

    async fn headers_refresh(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        let mut merged = HashMap::new();
        for provider in &self.providers {
            merged.extend(provider.headers_refresh().await?);
        }
        Ok(merged)
    }
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
