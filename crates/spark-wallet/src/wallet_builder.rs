use std::sync::Arc;

use platform_utils::HttpClient;
use spark::{
    header_provider::HeaderProvider,
    operator::rpc::{ConnectionManager, DefaultConnectionManager},
    services::TransferObserver,
    session_store::{InMemorySessionStore, SessionStore},
    signer::SparkSigner,
    token::{InMemoryTokenOutputStore, TokenOutputStore},
    tree::{InMemoryTreeStore, TreeStore},
};
use tokio::sync::watch;

use crate::{SparkWallet, SparkWalletConfig, SparkWalletError};

pub struct WalletBuilder {
    config: SparkWalletConfig,
    spark_signer: Arc<dyn SparkSigner>,
    cancellation_token: Option<watch::Receiver<()>>,
    session_store: Option<Arc<dyn SessionStore>>,
    tree_store: Option<Arc<dyn TreeStore>>,
    token_output_store: Option<Arc<dyn TokenOutputStore>>,
    connection_manager: Option<Arc<dyn ConnectionManager>>,
    ssp_http_client: Option<Arc<dyn HttpClient>>,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
    ssp_extra_header_provider: Option<Arc<dyn HeaderProvider>>,
    so_extra_header_provider: Option<Arc<dyn HeaderProvider>>,
}

impl WalletBuilder {
    pub fn new(config: SparkWalletConfig, spark_signer: Arc<dyn SparkSigner>) -> Self {
        WalletBuilder {
            config,
            spark_signer,
            cancellation_token: None,
            session_store: None,
            tree_store: None,
            token_output_store: None,
            connection_manager: None,
            ssp_http_client: None,
            transfer_observer: None,
            ssp_extra_header_provider: None,
            so_extra_header_provider: None,
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
    pub fn with_session_store(mut self, session_store: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(session_store);
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

    /// Sets a shared HTTP client to use for SSP GraphQL traffic. When the same
    /// client is passed to multiple wallets in one process, they share the
    /// underlying `reqwest::Client` (and its h2 connection pool).
    #[must_use]
    pub fn with_ssp_http_client(mut self, ssp_http_client: Arc<dyn HttpClient>) -> Self {
        self.ssp_http_client = Some(ssp_http_client);
        self
    }

    #[must_use]
    pub fn with_transfer_observer(mut self, transfer_observer: Arc<dyn TransferObserver>) -> Self {
        self.transfer_observer = Some(transfer_observer);
        self
    }

    /// Adds an extra header provider whose headers are attached to every
    /// outgoing SSP request alongside the built-in auth headers.
    #[must_use]
    pub fn with_ssp_extra_header_provider(mut self, provider: Arc<dyn HeaderProvider>) -> Self {
        self.ssp_extra_header_provider = Some(provider);
        self
    }

    /// Adds an extra header provider whose headers are attached to every
    /// outgoing Spark Operator (gRPC) request alongside the built-in auth
    /// headers.
    #[must_use]
    pub fn with_so_extra_header_provider(mut self, provider: Arc<dyn HeaderProvider>) -> Self {
        self.so_extra_header_provider = Some(provider);
        self
    }

    pub async fn build(self) -> Result<SparkWallet, SparkWalletError> {
        SparkWallet::new(
            self.config,
            self.spark_signer,
            self.session_store
                .unwrap_or(Arc::new(InMemorySessionStore::default())),
            self.tree_store
                .unwrap_or(Arc::new(InMemoryTreeStore::default())),
            self.token_output_store
                .unwrap_or(Arc::new(InMemoryTokenOutputStore::default())),
            self.connection_manager
                .unwrap_or(Arc::new(DefaultConnectionManager::new())),
            self.ssp_http_client,
            self.transfer_observer,
            self.ssp_extra_header_provider,
            self.so_extra_header_provider,
            self.cancellation_token,
        )
        .await
    }
}
