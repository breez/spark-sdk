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
}
