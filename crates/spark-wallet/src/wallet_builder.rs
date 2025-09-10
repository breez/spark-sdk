use std::sync::Arc;

use spark::{
    operator::{InMemorySessionManager, SessionManager},
    signer::Signer,
    tree::{InMemoryTreeStore, TreeStore},
};

use crate::{SparkWallet, SparkWalletConfig, SparkWalletError};

#[derive(Clone)]
pub struct WalletBuilder<S> {
    config: SparkWalletConfig,
    signer: S,
    session_manager: Option<Arc<dyn SessionManager>>,
    tree_store: Option<Arc<dyn TreeStore>>,
}

impl<S: Signer> WalletBuilder<S> {
    pub fn new(config: SparkWalletConfig, signer: S) -> Self {
        WalletBuilder {
            config,
            signer,
            session_manager: None,
            tree_store: None,
        }
    }

    pub fn with_session_manager(mut self, session_manager: Arc<dyn SessionManager>) -> Self {
        self.session_manager = Some(session_manager);
        self
    }

    pub fn with_tree_store(mut self, tree_store: Arc<dyn TreeStore>) -> Self {
        self.tree_store = Some(tree_store);
        self
    }

    pub async fn build(self) -> Result<SparkWallet<S>, SparkWalletError> {
        SparkWallet::new(
            self.config,
            Arc::new(self.signer),
            self.session_manager
                .unwrap_or(Arc::new(InMemorySessionManager::default())),
            self.tree_store
                .unwrap_or(Arc::new(InMemoryTreeStore::default())),
        )
        .await
    }
}
