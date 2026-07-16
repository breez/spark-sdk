use spark::operator::OperatorConfig;
use spark::operator::rpc::ConnectionManager;
use spark::session_store::InMemorySessionStore;
use spark::ssp::ServiceProvider;
use spark_wallet::DefaultSigner;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::{Mutex, RwLock, watch};
use tracing::warn;

pub struct State<DB> {
    pub db: DB,
    pub webhook_service: crate::webhooks::WebhookService<DB>,
    pub wallet: Arc<spark_wallet::SparkWallet>,
    pub is_mainnet: bool,
    pub scheme: String,
    pub min_sendable: u64,
    pub max_sendable: u64,
    pub include_spark_address: bool,
    pub domains: Arc<RwLock<crate::domains::DomainMap>>,
    pub nostr_keys: Option<nostr::Keys>,
    pub ca_cert: Option<Vec<u8>>,
    pub crl_url: Option<String>,
    pub crl: HashSet<String>,
    pub connection_manager: Arc<dyn ConnectionManager>,
    pub coordinator: OperatorConfig,
    pub signer: Arc<DefaultSigner>,
    pub session_store: Arc<InMemorySessionStore>,
    pub service_provider: Arc<ServiceProvider>,
    pub spark_config: spark_wallet::SparkWalletConfig,
    pub ssp_http_client: Arc<dyn platform_utils::HttpClient>,
    pub jwt_cache: Option<Arc<crate::partner_jwt::JwtCache>>,
    pub subscribed_keys: Arc<Mutex<HashSet<String>>>,
    pub invoice_paid_trigger: watch::Sender<()>,
    pub webhook_secret: String,
}

impl<DB> State<DB> {
    /// The wallet to create an invoice for `domain` with.
    ///
    /// When partner attribution is active, builds a per-request, background-less
    /// wallet whose provider serves `domain`'s own partner JWT, or the default
    /// when `domain` has no api key of its own. It shares the process signer,
    /// pre-warmed session, HTTP client, and connection pool, starts no background
    /// tasks, and is dropped after the request. Falls back to the state wallet
    /// when attribution is off (non-mainnet) or the per-request build fails.
    pub async fn invoice_wallet(&self, domain: &str) -> Arc<spark_wallet::SparkWallet> {
        let Some(cache) = &self.jwt_cache else {
            return Arc::clone(&self.wallet);
        };
        let domain_jwt_provider = Some(cache.provider_for(domain.to_string())
            as Arc<dyn spark::header_provider::HeaderProvider>);
        let built = spark_wallet::SparkWallet::new(
            self.spark_config.clone(),
            Arc::new(spark_wallet::SparkSignerAdapter::new(self.signer.clone())),
            self.session_store.clone(),
            Arc::new(spark::tree::InMemoryTreeStore::default()),
            Arc::new(spark::token::InMemoryTokenOutputStore::default()),
            Arc::clone(&self.connection_manager),
            Some(Arc::clone(&self.ssp_http_client)),
            None,
            domain_jwt_provider.clone(),
            domain_jwt_provider,
            None,
        )
        .await;
        match built {
            Ok(wallet) => Arc::new(wallet),
            Err(e) => {
                warn!(
                    "failed to build attributed wallet for '{domain}', using the state wallet: {e}"
                );
                Arc::clone(&self.wallet)
            }
        }
    }
}

impl<DB> Clone for State<DB>
where
    DB: Clone,
{
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            webhook_service: self.webhook_service.clone(),
            wallet: Arc::clone(&self.wallet),
            is_mainnet: self.is_mainnet,
            scheme: self.scheme.clone(),
            min_sendable: self.min_sendable,
            max_sendable: self.max_sendable,
            include_spark_address: self.include_spark_address,
            domains: Arc::clone(&self.domains),
            nostr_keys: self.nostr_keys.clone(),
            ca_cert: self.ca_cert.clone(),
            crl_url: self.crl_url.clone(),
            crl: self.crl.clone(),
            connection_manager: self.connection_manager.clone(),
            coordinator: self.coordinator.clone(),
            signer: self.signer.clone(),
            session_store: self.session_store.clone(),
            service_provider: self.service_provider.clone(),
            spark_config: self.spark_config.clone(),
            ssp_http_client: Arc::clone(&self.ssp_http_client),
            jwt_cache: self.jwt_cache.clone(),
            subscribed_keys: Arc::clone(&self.subscribed_keys),
            invoice_paid_trigger: self.invoice_paid_trigger.clone(),
            webhook_secret: self.webhook_secret.clone(),
        }
    }
}
