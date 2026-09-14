pub mod onchain;

use std::sync::Arc;

use crate::tree::deposit::TreeDepositService;
use bitcoin::Network;
use bitcoin::secp256k1::PublicKey;
use spark::bitcoin::BitcoinService;
use spark::operator::rpc::{ConnectionManager, DefaultConnectionManager};
use spark::operator::{OperatorConfig, OperatorPool, OperatorPoolConfig};
use spark::services::{DepositService, TimelockManager, TransferService};
use spark::session_store::InMemorySessionStore;
use spark::signer::{DefaultSigner, Signer, SparkSigner, SparkSignerAdapter};
use spark::ssp::{RetryConfig, ServiceProvider, ServiceProviderConfig};
use spark::tree::{SynchronousTreeService, TreeService, TreeStore};
use spark_postgres::{PostgresStorageConfig, PostgresTreeStore};
use sqlx::PgPool;
use thiserror::Error;
use tracing::info;

use crate::chain::ChainRepository;

pub use onchain::{OnchainWallet, OnchainWalletError};

#[derive(Error, Debug)]
pub enum WalletError {
    #[error("spark service error: {0}")]
    Spark(String),

    #[error("signer error: {0}")]
    Signer(String),

    #[error("postgres error: {0}")]
    Postgres(#[from] spark_postgres::PostgresError),

    #[error("onchain wallet error: {0}")]
    Onchain(#[from] OnchainWalletError),

    #[error("operator error: {0}")]
    Operator(String),

    #[error("unsupported network: {0}")]
    UnsupportedNetwork(String),
}

pub struct SparkServices {
    pub signer: Arc<dyn Signer>,
    pub spark_signer: Arc<dyn SparkSigner>,
    pub operator_pool: Arc<OperatorPool>,
    pub tree_store: Arc<dyn TreeStore>,
    pub transfer_service: Arc<TransferService>,
    pub deposit_service: Arc<DepositService>,
    pub tree_deposit_service: TreeDepositService,
    pub tree_service: Arc<dyn TreeService>,
    pub identity_public_key: PublicKey,
    pub network: spark::Network,
}

pub struct SspWallet<R: ChainRepository> {
    /// `Arc` because the coins held by builds in progress are tracked per instance.
    pub onchain: Arc<OnchainWallet<R>>,
    pub spark: SparkServices,
}

impl<R: ChainRepository + Send + Sync + 'static> SspWallet<R> {
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    pub async fn new(
        seed: &[u8],
        network: Network,
        leaf_store: PostgresStorageConfig,
        chain_repository: Arc<R>,
        pgpool: Arc<PgPool>,
        operators: Option<Vec<OperatorSetting>>,
        max_nodes_per_request: usize,
    ) -> Result<Self, WalletError> {
        let onchain = Arc::new(OnchainWallet::new(seed, network, chain_repository, pgpool)?);

        let spark_network =
            spark::Network::try_from(network).map_err(WalletError::UnsupportedNetwork)?;
        let signer: Arc<dyn Signer> = Arc::new(
            DefaultSigner::new(seed, spark_network)
                .map_err(|e| WalletError::Signer(e.to_string()))?,
        );

        let spark_signer: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer.clone()));

        let identity_public_key = spark::signer::derive_identity_public_key(signer.as_ref())
            .await
            .map_err(|e| WalletError::Signer(e.to_string()))?;

        let session_store = Arc::new(InMemorySessionStore::default());
        let connection_manager: Arc<dyn ConnectionManager> =
            Arc::new(DefaultConnectionManager::new());

        let operator_pool_config = operator_pool_config(spark_network, operators)?;
        let operator_pool = Arc::new(
            OperatorPool::connect(
                &operator_pool_config,
                connection_manager,
                session_store.clone(),
                spark_signer.clone(),
                None,
            )
            .await
            .map_err(|e| WalletError::Operator(e.to_string()))?,
        );

        let transfer_service = Arc::new(TransferService::new(
            spark_signer.clone(),
            spark_network,
            2, // split_secret_threshold
            operator_pool.clone(),
            None,
        ));

        // DepositService requires a service provider, but the SSP never calls
        // another SSP, so this one has no URL.
        let service_provider = Arc::new(
            ServiceProvider::new(
                ServiceProviderConfig {
                    base_url: String::new(),
                    schema_endpoint: None,
                    identity_public_key,
                    user_agent: None,
                    retry_config: RetryConfig::default(),
                },
                spark_signer.clone(),
                session_store,
                None,
            )
            .map_err(|e| WalletError::Spark(e.to_string()))?,
        );

        let bitcoin_service = BitcoinService::new(spark_network);
        let deposit_service = Arc::new(DepositService::new(
            bitcoin_service,
            identity_public_key,
            spark_network,
            operator_pool.clone(),
            service_provider,
            spark_signer.clone(),
        ));

        let tree_store: Arc<dyn TreeStore> = Arc::new(
            PostgresTreeStore::from_config(leaf_store, &identity_public_key.serialize()).await?,
        );

        let tree_service: Arc<dyn TreeService> = Arc::new(SynchronousTreeService::new(
            identity_public_key,
            spark_network,
            operator_pool.clone(),
            Arc::clone(&tree_store),
            Arc::new(TimelockManager::new(
                spark_signer.clone(),
                spark_network,
                operator_pool.clone(),
            )),
            spark_signer.clone(),
            // The SSP is the service provider, so it has none to swap with.
            None,
            None,
        ));

        let tree_deposit_service = TreeDepositService::new(
            identity_public_key,
            spark_network,
            operator_pool.clone(),
            signer.clone(),
        )
        .with_max_nodes_per_request(max_nodes_per_request);

        info!("SSP wallet initialized (onchain + spark services)");
        Ok(Self {
            onchain,
            spark: SparkServices {
                signer,
                spark_signer,
                operator_pool,
                tree_store,
                transfer_service,
                deposit_service,
                tree_deposit_service,
                tree_service,
                identity_public_key,
                network: spark_network,
            },
        })
    }
}

/// Deserialized from the daemon's config, so its field names are config keys.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OperatorSetting {
    /// Position in the pool. The operator at index 0 is the coordinator.
    pub id: usize,
    /// FROST identifier, hex.
    pub identifier: String,
    pub address: String,
    /// Hex.
    pub identity_public_key: String,
    /// PEM CA certificate to trust instead of the bundled Mozilla root
    /// certificates.
    #[serde(default)]
    pub ca_cert_pem: Option<String>,
}

fn operator_pool_config(
    network: spark::Network,
    operators: Option<Vec<OperatorSetting>>,
) -> Result<OperatorPoolConfig, WalletError> {
    let Some(operators) = operators.filter(|o| !o.is_empty()) else {
        return default_operator_pool_config(network);
    };
    let operators = operators
        .into_iter()
        .map(|operator| {
            use std::str::FromStr;
            Ok(OperatorConfig {
                id: operator.id,
                identifier: frost_secp256k1_tr::Identifier::deserialize(
                    &hex::decode(&operator.identifier).map_err(|e| {
                        WalletError::Spark(format!("bad operator identifier hex: {e}"))
                    })?,
                )
                .map_err(|e| WalletError::Spark(format!("bad operator identifier: {e}")))?,
                address: operator.address,
                ca_cert: operator.ca_cert_pem.map(String::into_bytes),
                identity_public_key: PublicKey::from_str(&operator.identity_public_key)
                    .map_err(|e| WalletError::Spark(format!("bad operator pubkey: {e}")))?,
                user_agent: None,
            })
        })
        .collect::<Result<Vec<_>, WalletError>>()?;
    OperatorPoolConfig::new(0, operators)
        .map_err(|e| WalletError::Operator(format!("invalid operator pool config: {e}")))
}

fn default_operator_pool_config(
    _network: spark::Network,
) -> Result<OperatorPoolConfig, WalletError> {
    fn make_operator(
        id: usize,
        identifier_hex: &str,
        address: &str,
        identity_pubkey_hex: &str,
    ) -> Result<OperatorConfig, WalletError> {
        use std::str::FromStr;
        Ok(OperatorConfig {
            id,
            identifier: frost_secp256k1_tr::Identifier::deserialize(
                &hex::decode(identifier_hex)
                    .map_err(|e| WalletError::Spark(format!("bad identifier hex: {e}")))?,
            )
            .map_err(|e| WalletError::Spark(format!("bad identifier: {e}")))?,
            address: address.to_string(),
            ca_cert: None,
            identity_public_key: PublicKey::from_str(identity_pubkey_hex)
                .map_err(|e| WalletError::Spark(format!("bad pubkey: {e}")))?,
            user_agent: None,
        })
    }

    let operators = vec![
        make_operator(
            0,
            "0000000000000000000000000000000000000000000000000000000000000001",
            "https://0.spark.lightspark.com",
            "03dfbdff4b6332c220f8fa2ba8ed496c698ceada563fa01b67d9983bfc5c95e763",
        )?,
        make_operator(
            1,
            "0000000000000000000000000000000000000000000000000000000000000002",
            "https://spark-operator.breez.technology",
            "03e625e9768651c9be268e287245cc33f96a68ce9141b0b4769205db027ee8ed77",
        )?,
        make_operator(
            2,
            "0000000000000000000000000000000000000000000000000000000000000003",
            "https://2.spark.flashnet.xyz",
            "022eda13465a59205413086130a65dc0ed1b8f8e51937043161f8be0c369b1a410",
        )?,
    ];

    OperatorPoolConfig::new(0, operators)
        .map_err(|e| WalletError::Operator(format!("invalid operator pool config: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct OperatorsConfig {
        operators: Vec<OperatorSetting>,
    }

    #[test]
    fn operators_are_read_from_config_and_replace_the_deployed_set() {
        let toml = r#"
[[operators]]
id = 0
identifier = "0000000000000000000000000000000000000000000000000000000000000001"
address = "https://127.0.0.1:8535"
identity_public_key = "03dfbdff4b6332c220f8fa2ba8ed496c698ceada563fa01b67d9983bfc5c95e763"

[[operators]]
id = 1
identifier = "0000000000000000000000000000000000000000000000000000000000000002"
address = "https://127.0.0.1:8536"
identity_public_key = "03e625e9768651c9be268e287245cc33f96a68ce9141b0b4769205db027ee8ed77"
"#;
        let parsed: OperatorsConfig = toml::from_str(toml).expect("operators parse from TOML");
        let config = operator_pool_config(spark::Network::Regtest, Some(parsed.operators))
            .expect("configured operators build a pool");
        let addresses: Vec<String> = config
            .get_all_operators()
            .map(|operator| operator.address.clone())
            .collect();
        assert_eq!(
            addresses,
            vec![
                "https://127.0.0.1:8535".to_string(),
                "https://127.0.0.1:8536".to_string()
            ],
            "the configured operators are used, not the deployed ones"
        );
    }

    #[test]
    fn no_configured_operators_falls_back_to_the_deployed_set() {
        let configured = operator_pool_config(spark::Network::Mainnet, None)
            .expect("the deployed operators build a pool");
        let empty = operator_pool_config(spark::Network::Mainnet, Some(Vec::new()))
            .expect("an empty list is the same as none");
        assert_eq!(configured.get_all_operators().count(), 3);
        assert_eq!(empty.get_all_operators().count(), 3);
    }
}
