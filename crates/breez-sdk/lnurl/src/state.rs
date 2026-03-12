use spark::operator::OperatorConfig;
use spark::operator::rpc::ConnectionManager;
use spark::session_manager::InMemorySessionManager;
use spark::ssp::ServiceProvider;
use spark_wallet::DefaultSigner;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::{Mutex, watch};

pub struct State<DB> {
    pub db: DB,
    pub wallet: Arc<spark_wallet::SparkWallet>,
    pub scheme: String,
    pub min_sendable: u64,
    pub max_sendable: u64,
    pub include_spark_address: bool,
    pub domains: HashSet<String>,
    pub nostr_keys: Option<nostr::Keys>,
    pub ca_cert: Option<Vec<u8>>,
    pub connection_manager: Arc<dyn ConnectionManager>,
    pub coordinator: OperatorConfig,
    pub signer: Arc<DefaultSigner>,
    pub session_manager: Arc<InMemorySessionManager>,
    pub service_provider: Arc<ServiceProvider>,
    pub subscribed_keys: Arc<Mutex<HashSet<String>>>,
    pub invoice_paid_trigger: watch::Sender<()>,
}

impl<DB> Clone for State<DB>
where
    DB: Clone,
{
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            wallet: Arc::clone(&self.wallet),
            scheme: self.scheme.clone(),
            min_sendable: self.min_sendable,
            max_sendable: self.max_sendable,
            include_spark_address: self.include_spark_address,
            domains: self.domains.clone(),
            nostr_keys: self.nostr_keys.clone(),
            ca_cert: self.ca_cert.clone(),
            connection_manager: self.connection_manager.clone(),
            coordinator: self.coordinator.clone(),
            signer: self.signer.clone(),
            session_manager: self.session_manager.clone(),
            service_provider: self.service_provider.clone(),
            subscribed_keys: Arc::clone(&self.subscribed_keys),
            invoice_paid_trigger: self.invoice_paid_trigger.clone(),
        }
    }
}
