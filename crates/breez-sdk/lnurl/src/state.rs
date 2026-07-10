use spark::operator::OperatorConfig;
use spark::operator::rpc::ConnectionManager;
use spark::session_store::InMemorySessionStore;
use spark::ssp::ServiceProvider;
use spark_wallet::DefaultSigner;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::{Mutex, RwLock, watch};

pub struct State<DB> {
    pub db: DB,
    pub webhook_service: crate::webhooks::WebhookService<DB>,
    pub wallet: Arc<spark_wallet::SparkWallet>,
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
    pub jwt_cache: Option<Arc<crate::partner_jwt::JwtCache>>,
    pub subscribed_keys: Arc<Mutex<HashSet<String>>>,
    pub invoice_paid_trigger: watch::Sender<()>,
    pub webhook_secret: String,
}

impl<DB> State<DB> {
    /// Builds a per-request, background-less wallet for creating one invoice. It
    /// shares the process signer, the pre-warmed session, and the connection
    /// pool, and carries `domain`'s partner-JWT provider (when attribution is
    /// enabled) so the invoice is attributed to that domain's partner. No
    /// background tasks are started; the wallet is dropped after the request.
    pub async fn build_invoice_wallet(
        &self,
        domain: &str,
    ) -> Result<Arc<spark_wallet::SparkWallet>, spark_wallet::SparkWalletError> {
        let attribution = self.jwt_cache.as_ref().map(|c| {
            c.provider_for(domain.to_string()) as Arc<dyn spark::header_provider::HeaderProvider>
        });
        let wallet = spark_wallet::SparkWallet::new(
            self.spark_config.clone(),
            Arc::new(spark_wallet::SparkSignerAdapter::new(self.signer.clone())),
            self.session_store.clone(),
            Arc::new(spark::tree::InMemoryTreeStore::default()),
            Arc::new(spark::token::InMemoryTokenOutputStore::default()),
            Arc::clone(&self.connection_manager),
            None,
            None,
            attribution.clone(),
            attribution,
            None,
        )
        .await?;
        Ok(Arc::new(wallet))
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
            jwt_cache: self.jwt_cache.clone(),
            subscribed_keys: Arc::clone(&self.subscribed_keys),
            invoice_paid_trigger: self.invoice_paid_trigger.clone(),
            webhook_secret: self.webhook_secret.clone(),
        }
    }
}
