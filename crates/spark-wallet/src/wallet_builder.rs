use std::sync::Arc;

use spark::{
    operator::rpc::{ConnectionManager, DefaultConnectionManager},
    services::TransferObserver,
    session_manager::{InMemorySessionManager, SessionManager},
    signer::Signer,
    token::{InMemoryTokenOutputStore, TokenOutputStore},
    tree::{InMemoryTreeStore, TreeStore},
};
use tokio::sync::watch;

use crate::{SparkWallet, SparkWalletConfig, SparkWalletError};

pub struct WalletBuilder {
    config: SparkWalletConfig,
    signer: Arc<dyn Signer>,
    cancellation_token: Option<watch::Receiver<()>>,
    session_manager: Option<Arc<dyn SessionManager>>,
    tree_store: Option<Arc<dyn TreeStore>>,
    token_output_store: Option<Arc<dyn TokenOutputStore>>,
    connection_manager: Option<Arc<dyn ConnectionManager>>,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
    with_background_processing: bool,
}

impl WalletBuilder {
    pub fn new(config: SparkWalletConfig, signer: Arc<dyn Signer>) -> Self {
        WalletBuilder {
            config,
            signer,
            cancellation_token: None,
            session_manager: None,
            tree_store: None,
            token_output_store: None,
            connection_manager: None,
            transfer_observer: None,
            with_background_processing: true,
        }
    }

    /// Sets an external cancellation token for stopping background tasks.
    /// If not set, an internal token will be created that stops tasks when the wallet is dropped.
    #[must_use]
    pub fn with_cancellation_token(mut self, cancellation_token: watch::Receiver<()>) -> Self {
        self.cancellation_token = Some(cancellation_token);
        self
    }

    #[must_use]
    pub fn with_session_manager(mut self, session_manager: Arc<dyn SessionManager>) -> Self {
        self.session_manager = Some(session_manager);
        self
    }

    #[must_use]
    pub fn with_tree_store(mut self, tree_store: Arc<dyn TreeStore>) -> Self {
        self.tree_store = Some(tree_store);
        self
    }

    #[must_use]
    pub fn with_token_output_store(
        mut self,
        token_output_store: Arc<dyn TokenOutputStore>,
    ) -> Self {
        self.token_output_store = Some(token_output_store);
        self
    }

    #[must_use]
    pub fn with_connection_manager(
        mut self,
        connection_manager: Arc<dyn ConnectionManager>,
    ) -> Self {
        self.connection_manager = Some(connection_manager);
        self
    }

    #[must_use]
    pub fn with_transfer_observer(mut self, transfer_observer: Arc<dyn TransferObserver>) -> Self {
        self.transfer_observer = Some(transfer_observer);
        self
    }

    #[must_use]
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
            self.token_output_store
                .unwrap_or(Arc::new(InMemoryTokenOutputStore::default())),
            self.connection_manager
                .unwrap_or(Arc::new(DefaultConnectionManager::new())),
            self.transfer_observer,
            self.with_background_processing,
            self.cancellation_token,
        )
        .await
    }
}
