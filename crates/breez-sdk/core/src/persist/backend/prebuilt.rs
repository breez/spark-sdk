//! Backend over caller-supplied store implementations.

use std::sync::Arc;

use macros::async_trait;
use spark_wallet::{SessionStore, TokenOutputStore, TreeStore};

use crate::{Network, SdkError, X402ReplayStore, persist::Storage};

use super::{ResolvedStores, StorageBackend};

/// A [`StorageBackend`] over caller-supplied store implementations.
///
/// `create_stores` returns the stores as-is — `network` and `identity` are
/// ignored, so the caller is responsible for any tenant scoping.
pub struct PrebuiltBackend {
    storage: Arc<dyn Storage>,
    tree_store: Option<Arc<dyn TreeStore>>,
    token_output_store: Option<Arc<dyn TokenOutputStore>>,
    session_store: Option<Arc<dyn SessionStore>>,
    x402_replay_store: Option<Arc<dyn X402ReplayStore>>,
}

impl PrebuiltBackend {
    /// Builds a backend from a caller-supplied [`Storage`] and, optionally, a
    /// tree, token-output and session store. Stores left `None` fall back to
    /// the in-memory defaults.
    #[must_use]
    pub fn new(
        storage: Arc<dyn Storage>,
        tree_store: Option<Arc<dyn TreeStore>>,
        token_output_store: Option<Arc<dyn TokenOutputStore>>,
        session_store: Option<Arc<dyn SessionStore>>,
    ) -> Self {
        Self {
            storage,
            tree_store,
            token_output_store,
            session_store,
            x402_replay_store: None,
        }
    }

    /// Adds the store that records which x402 payments have been used.
    #[must_use]
    pub fn with_x402_replay_store(mut self, store: Arc<dyn X402ReplayStore>) -> Self {
        self.x402_replay_store = Some(store);
        self
    }
}

#[async_trait]
impl StorageBackend for PrebuiltBackend {
    async fn create_stores(
        &self,
        _network: Network,
        _identity: Vec<u8>,
    ) -> Result<Arc<ResolvedStores>, SdkError> {
        Ok(Arc::new(ResolvedStores {
            storage: self.storage.clone(),
            tree_store: self.tree_store.clone(),
            token_output_store: self.token_output_store.clone(),
            session_store: self.session_store.clone(),
            x402_replay_store: self.x402_replay_store.clone(),
        }))
    }
}
