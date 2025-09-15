use std::sync::Arc;

use spark::{
    operator::rpc::{ConnectionManager, DefaultConnectionManager},
    session_manager::{InMemorySessionManager, SessionManager},
    signer::Signer,
    tree::{InMemoryTreeStore, TreeStore},
};

use crate::{SparkWallet, SparkWalletConfig, SparkWalletError};

#[derive(Clone)]
pub struct WalletBuilder {
    config: SparkWalletConfig,
    signer: Arc<dyn Signer>,
    session_manager: Option<Arc<dyn SessionManager>>,
    tree_store: Option<Arc<dyn TreeStore>>,
    connection_manager: Option<Arc<dyn ConnectionManager>>,
    with_background_processing: bool,
}

impl WalletBuilder {
    pub fn new(config: SparkWalletConfig, signer: Arc<dyn Signer>) -> Self {
        WalletBuilder {
            config,
            signer,
            session_manager: None,
            tree_store: None,
            connection_manager: None,
            with_background_processing: true,
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

    pub fn with_connection_manager(
        mut self,
        connection_manager: Arc<dyn ConnectionManager>,
    ) -> Self {
        self.connection_manager = Some(connection_manager);
        self
    }

    pub fn with_background_processing(mut self, with_background_processing: bool) -> Self {
        self.with_background_processing = with_background_processing;
        self
    }

    pub async fn build(self) -> Result<SparkWallet, SparkWalletError> {
        SparkWallet::new(
            self.config,
            self.signer,
            self.session_manager
                .unwrap_or(Arc::new(InMemorySessionManager::default())),
            self.tree_store
                .unwrap_or(Arc::new(InMemoryTreeStore::default())),
            self.connection_manager
                .unwrap_or(Arc::new(DefaultConnectionManager::new())),
            self.with_background_processing,
        )
        .await
    }
}
