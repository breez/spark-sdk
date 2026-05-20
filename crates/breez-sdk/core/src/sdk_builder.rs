#![cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    allow(clippy::arc_with_non_send_sync)
)]
use std::sync::Arc;

use breez_sdk_common::buy::moonpay::MoonpayProvider;

#[cfg(not(target_family = "wasm"))]
use spark_wallet::Signer;
use spark_wallet::{InMemorySessionStore, SparkWalletConfig, TokenOutputStore, TreeStore};
use tokio::sync::watch;
use tracing::{debug, info};

use flashnet::{FlashnetConfig, IntegratorConfig};

use crate::{
    Credentials, EventEmitter, FiatService, FiatServiceWrapper, KeySetType, Network, Seed,
    chain::{
        BitcoinChainService,
        rest_client::{BasicAuth, ChainApiType, RestClientChainService},
    },
    error::SdkError,
    lnurl::{DefaultLnurlServerClient, LnurlServerClient},
    models::Config,
    payment_observer::{PaymentObserver, SparkTransferObserver},
    persist::Storage,
    realtime_sync::{RealTimeSyncParams, init_and_start_real_time_sync},
    sdk::{BreezSdk, BreezSdkParams, SyncCoordinator, runtime_from_config},
    sdk_context::{SdkContext, SdkContextConfig, new_shared_sdk_context},
    session_store::{SessionStore, SessionStoreAdapter},
    signer::{
        breez::BreezSignerImpl, lnurl_auth::LnurlAuthSignerAdapter, rtsync::RTSyncSigner,
        spark::SparkSigner,
    },
    stable_balance::StableBalance,
    token_conversion::TokenConversionMiddleware,
    token_conversion::{
        DEFAULT_INTEGRATOR_FEE_BPS, DEFAULT_INTEGRATOR_PUBKEY, FlashnetTokenConverter,
        TokenConverter,
    },
};

/// Configuration captured by [`SdkBuilder::with_rest_chain_service`].
///
/// Stored on the builder and resolved during `build()` so the resulting
/// `RestClientChainService` reuses the shared HTTP client from the
/// [`SdkContext`](crate::SdkContext).
#[derive(Clone)]
struct RestChainServiceConfig {
    url: String,
    api_type: ChainApiType,
    credentials: Option<Credentials>,
}

/// Source for the signer - either a seed or an external signer implementation
#[derive(Clone)]
enum SignerSource {
    Seed {
        seed: Seed,
        key_set_type: KeySetType,
        use_address_index: bool,
        account_number: Option<u32>,
    },
    External(Arc<dyn crate::signer::ExternalSigner>),
}

/// Builder for creating `BreezSdk` instances with customizable components.
#[derive(Clone)]
pub struct SdkBuilder {
    config: Config,
    signer_source: SignerSource,

    storage_dir: Option<String>,
    storage: Option<Arc<dyn Storage>>,
    #[cfg(feature = "postgres")]
    postgres_pool: Option<Arc<crate::persist::postgres::PostgresConnectionPool>>,
    #[cfg(feature = "mysql")]
    mysql_pool: Option<Arc<crate::persist::mysql::MysqlConnectionPool>>,
    chain_service: Option<Arc<dyn BitcoinChainService>>,
    rest_chain_service_config: Option<RestChainServiceConfig>,
    fiat_service: Option<Arc<dyn FiatService>>,
    lnurl_client: Option<Arc<dyn platform_utils::HttpClient>>,
    lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    payment_observer: Option<Arc<dyn PaymentObserver>>,
    tree_store: Option<Arc<dyn TreeStore>>,
    token_output_store: Option<Arc<dyn TokenOutputStore>>,
    context: Option<Arc<SdkContext>>,
    session_store: Option<Arc<dyn SessionStore>>,
}

impl SdkBuilder {
    /// Creates a new `SdkBuilder` with the provided configuration and seed.
    ///
    /// For external signer support, use `new_with_signer` instead.
    ///
    /// # Arguments
    /// - `config`: The configuration to be used.
    /// - `seed`: The seed for wallet generation.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(config: Config, seed: Seed) -> Self {
        SdkBuilder {
            config,
            signer_source: SignerSource::Seed {
                seed,
                key_set_type: KeySetType::Default,
                use_address_index: false,
                account_number: None,
            },
            storage_dir: None,
            storage: None,
            #[cfg(feature = "postgres")]
            postgres_pool: None,
            #[cfg(feature = "mysql")]
            mysql_pool: None,
            chain_service: None,
            rest_chain_service_config: None,
            fiat_service: None,
            lnurl_client: None,
            lnurl_server_client: None,
            payment_observer: None,
            tree_store: None,
            token_output_store: None,
            context: None,
            session_store: None,
        }
    }

    /// Creates a new `SdkBuilder` with the provided configuration and external signer.
    ///
    /// # Arguments
    /// - `config`: The configuration to be used.
    /// - `signer`: An external signer implementation.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new_with_signer(config: Config, signer: Arc<dyn crate::signer::ExternalSigner>) -> Self {
        SdkBuilder {
            config,
            signer_source: SignerSource::External(signer),
            storage_dir: None,
            storage: None,
            #[cfg(feature = "postgres")]
            postgres_pool: None,
            #[cfg(feature = "mysql")]
            mysql_pool: None,
            chain_service: None,
            rest_chain_service_config: None,
            fiat_service: None,
            lnurl_client: None,
            lnurl_server_client: None,
            payment_observer: None,
            tree_store: None,
            token_output_store: None,
            context: None,
            session_store: None,
        }
    }

    /// Sets the key set type to be used by the SDK.
    ///
    /// Note: This only applies when using a seed-based signer. It has no effect
    /// when using an external signer (created with `new_with_signer`).
    ///
    /// # Arguments
    /// - `config`: Key set configuration containing the key set type, address index flag, and optional account number.
    #[must_use]
    pub fn with_key_set(mut self, config: crate::models::KeySetConfig) -> Self {
        if let SignerSource::Seed {
            key_set_type: ref mut kst,
            use_address_index: ref mut uai,
            account_number: ref mut an,
            ..
        } = self.signer_source
        {
            *kst = config.key_set_type;
            *uai = config.use_address_index;
            *an = config.account_number;
        }
        self
    }

    #[must_use]
    /// Sets the root storage directory to initialize the default storage with.
    /// This initializes both storage and real-time sync storage with the
    /// default implementations.
    /// Arguments:
    /// - `storage_dir`: The data directory for storage.
    pub fn with_default_storage(mut self, storage_dir: String) -> Self {
        self.storage_dir = Some(storage_dir);
        self
    }

    #[must_use]
    /// Sets the storage implementation to be used by the SDK.
    /// Arguments:
    /// - `storage`: The storage implementation to be used.
    pub fn with_storage(mut self, storage: Arc<dyn Storage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Threads a shared [`SdkContext`] into this builder.
    ///
    /// Construct the context once via [`new_shared_sdk_context`] and pass the
    /// same `Arc` to every `SdkBuilder` whose SDKs should share its underlying
    /// resources (operator gRPC channels, SSP HTTP client, database pool).
    ///
    /// If not set, `build()` constructs a context internally from the SDK's
    /// own network and api key — fine for a single-SDK process with no DB
    /// backend.
    #[must_use]
    pub fn with_shared_context(mut self, context: Arc<SdkContext>) -> Self {
        self.context = Some(context);
        self
    }

    /// Sets a shared `PostgreSQL` connection pool as the backend for all
    /// stores (storage, tree store, and token store).
    ///
    /// Construct the pool once via
    /// [`create_postgres_connection_pool`](crate::create_postgres_connection_pool) and pass the
    /// same `Arc` to multiple `SdkBuilder` instances to share connections
    /// across SDKs. Per-tenant scoping is derived from each SDK's seed.
    ///
    /// If you've also threaded an [`SdkContext`] (via [`with_shared_context`](Self::with_shared_context))
    /// that already carries a Postgres pool, `build()` will error — pick one
    /// source. Most integrators use either this method *or* a context, not both.
    #[must_use]
    #[cfg(feature = "postgres")]
    pub fn with_postgres_connection_pool(
        mut self,
        pool: Arc<crate::persist::postgres::PostgresConnectionPool>,
    ) -> Self {
        self.postgres_pool = Some(pool);
        self
    }

    /// Sets a shared `MySQL` connection pool as the backend for all stores
    /// (storage, tree store, and token store).
    ///
    /// Construct the pool once via
    /// [`create_mysql_connection_pool`](crate::create_mysql_connection_pool) and pass the same
    /// `Arc` to multiple `SdkBuilder` instances to share connections across
    /// SDKs. Per-tenant scoping is derived from each SDK's seed.
    ///
    /// If you've also threaded an [`SdkContext`] (via [`with_shared_context`](Self::with_shared_context))
    /// that already carries a `MySQL` pool, `build()` will error — pick one
    /// source. Most integrators use either this method *or* a context, not both.
    #[must_use]
    #[cfg(feature = "mysql")]
    pub fn with_mysql_connection_pool(
        mut self,
        pool: Arc<crate::persist::mysql::MysqlConnectionPool>,
    ) -> Self {
        self.mysql_pool = Some(pool);
        self
    }

    /// Sets `PostgreSQL` as the backend by building a fresh pool from a
    /// config. The store instances will be created during `build()`.
    ///
    /// Arguments:
    /// - `config`: The `PostgreSQL` storage configuration.
    #[cfg(feature = "postgres")]
    #[deprecated(
        note = "Call `create_postgres_connection_pool(&config)` and `with_postgres_connection_pool(pool)` instead."
    )]
    #[allow(clippy::needless_pass_by_value)]
    pub fn with_postgres_backend(
        self,
        config: crate::persist::postgres::PostgresStorageConfig,
    ) -> Result<Self, SdkError> {
        let pool = crate::persist::postgres::create_postgres_connection_pool(&config)?;
        Ok(self.with_postgres_connection_pool(pool))
    }

    /// Sets `MySQL` as the backend by building a fresh pool from a config.
    /// The store instances will be created during `build()`.
    ///
    /// Arguments:
    /// - `config`: The `MySQL` storage configuration.
    #[cfg(feature = "mysql")]
    #[deprecated(
        note = "Call `create_mysql_connection_pool(&config)` and `with_mysql_connection_pool(pool)` instead."
    )]
    #[allow(clippy::needless_pass_by_value)]
    pub fn with_mysql_backend(
        self,
        config: crate::persist::mysql::MysqlStorageConfig,
    ) -> Result<Self, SdkError> {
        let pool = crate::persist::mysql::create_mysql_connection_pool(&config)?;
        Ok(self.with_mysql_connection_pool(pool))
    }

    /// WASM-only seam for the JS-side DB-backed session store.
    ///
    /// Cfg-gated to the WASM target so it literally doesn't exist on native
    /// builds and can't be misused.
    #[cfg(target_family = "wasm")]
    #[must_use]
    pub fn with_session_store(mut self, session_store: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(session_store);
        self
    }

    /// Sets the chain service to be used by the SDK.
    /// Arguments:
    /// - `chain_service`: The chain service to be used.
    #[must_use]
    pub fn with_chain_service(mut self, chain_service: Arc<dyn BitcoinChainService>) -> Self {
        self.chain_service = Some(chain_service);
        self.rest_chain_service_config = None;
        self
    }

    /// Configures a REST chain service to be used by the SDK.
    ///
    /// The service is constructed during [`build()`](Self::build) so it can
    /// reuse the shared HTTP client carried by the [`SdkContext`](crate::SdkContext).
    ///
    /// Arguments:
    /// - `url`: The base URL of the REST API.
    /// - `api_type`: The API type to be used.
    /// - `credentials`: Optional credentials for basic authentication.
    #[must_use]
    pub fn with_rest_chain_service(
        mut self,
        url: String,
        api_type: ChainApiType,
        credentials: Option<Credentials>,
    ) -> Self {
        self.chain_service = None;
        self.rest_chain_service_config = Some(RestChainServiceConfig {
            url,
            api_type,
            credentials,
        });
        self
    }

    /// Sets the fiat service to be used by the SDK.
    /// Arguments:
    /// - `fiat_service`: The fiat service to be used.
    #[must_use]
    pub fn with_fiat_service(mut self, fiat_service: Arc<dyn FiatService>) -> Self {
        self.fiat_service = Some(fiat_service);
        self
    }

    #[must_use]
    pub fn with_lnurl_client(mut self, lnurl_client: Arc<dyn crate::RestClient>) -> Self {
        self.lnurl_client = Some(Arc::new(crate::common::rest::RestClientWrapper::new(
            lnurl_client,
        )));
        self
    }

    #[must_use]
    #[allow(unused)]
    pub fn with_lnurl_server_client(
        mut self,
        lnurl_serverclient: Arc<dyn LnurlServerClient>,
    ) -> Self {
        self.lnurl_server_client = Some(lnurl_serverclient);
        self
    }

    /// Sets the payment observer to be used by the SDK.
    /// This observer will receive callbacks before outgoing payments for Lightning, Spark and onchain Bitcoin.
    /// Arguments:
    /// - `payment_observer`: The payment observer to be used.
    #[must_use]
    #[allow(unused)]
    pub fn with_payment_observer(mut self, payment_observer: Arc<dyn PaymentObserver>) -> Self {
        self.payment_observer = Some(payment_observer);
        self
    }

    /// Sets a custom tree store implementation.
    ///
    /// # Arguments
    /// - `tree_store`: The tree store implementation to use.
    #[must_use]
    pub fn with_tree_store(mut self, tree_store: Arc<dyn TreeStore>) -> Self {
        self.tree_store = Some(tree_store);
        self
    }

    /// Sets a custom token output store implementation.
    ///
    /// # Arguments
    /// - `token_output_store`: The token output store implementation to use.
    #[must_use]
    pub fn with_token_output_store(
        mut self,
        token_output_store: Arc<dyn TokenOutputStore>,
    ) -> Self {
        self.token_output_store = Some(token_output_store);
        self
    }

    /// Builds a [`SparkWalletConfig`](spark_wallet::SparkWalletConfig) from a
    /// [`SparkConfig`](crate::models::SparkConfig).
    fn build_spark_wallet_config(
        network: spark_wallet::Network,
        env_config: &crate::models::SparkConfig,
    ) -> Result<spark_wallet::SparkWalletConfig, SdkError> {
        let coordinator_index = env_config
            .signing_operators
            .iter()
            .position(|op| op.identifier == env_config.coordinator_identifier)
            .ok_or_else(|| {
                SdkError::InvalidInput(
                    "coordinator_identifier does not match any signing operator".to_string(),
                )
            })?;

        let operators: Vec<_> = env_config
            .signing_operators
            .iter()
            .map(|op| {
                SparkWalletConfig::create_operator_config(
                    op.id as usize,
                    &op.identifier,
                    &op.address,
                    None,
                    &op.identity_public_key,
                )
                .map_err(|e| SdkError::InvalidInput(e.to_string()))
            })
            .collect::<Result<_, _>>()?;

        let operator_pool = spark_wallet::OperatorPoolConfig::new(coordinator_index, operators)
            .map_err(|e| SdkError::InvalidInput(e.to_string()))?;

        let service_provider_config = SparkWalletConfig::create_service_provider_config(
            &env_config.ssp_config.base_url,
            &env_config.ssp_config.identity_public_key,
            env_config.ssp_config.schema_endpoint.clone(),
        )
        .map_err(|e| SdkError::InvalidInput(e.to_string()))?;

        let mut config = SparkWalletConfig::default_config(network);
        config.operator_pool = operator_pool;
        config.split_secret_threshold = env_config.threshold;
        config.service_provider_config = service_provider_config;
        config.tokens_config.expected_withdraw_bond_sats = env_config.expected_withdraw_bond_sats;
        config
            .tokens_config
            .expected_withdraw_relative_block_locktime =
            env_config.expected_withdraw_relative_block_locktime;

        Ok(config)
    }

    /// Builds the `BreezSdk` instance with the configured components.
    #[allow(clippy::too_many_lines)]
    pub async fn build(self) -> Result<BreezSdk, SdkError> {
        // Validate configuration
        self.config.validate()?;
        let runtime = runtime_from_config(&self.config);
        if !runtime.starts_background_services() {
            if self.config.stable_balance_config.is_some() {
                return Err(SdkError::InvalidInput(
                    "stable_balance_config is not supported when background_tasks_enabled is false"
                        .to_string(),
                ));
            }
            if self.config.real_time_sync_server_url.is_some() {
                return Err(SdkError::InvalidInput(
                    "real_time_sync_server_url must be None when background_tasks_enabled is false"
                        .to_string(),
                ));
            }
            if self.config.leaf_optimization_config.auto_enabled {
                return Err(SdkError::InvalidInput(
                    "leaf_optimization_config.auto_enabled must be false when background_tasks_enabled is false"
                        .to_string(),
                ));
            }
            if self.config.token_optimization_config.auto_enabled {
                return Err(SdkError::InvalidInput(
                    "token_optimization_config.auto_enabled must be false when background_tasks_enabled is false"
                        .to_string(),
                ));
            }
        }

        // Create the base signer based on the signer source
        let signer: Arc<dyn crate::signer::BreezSigner> = match self.signer_source {
            SignerSource::Seed {
                seed,
                key_set_type,
                use_address_index,
                account_number,
            } => Arc::new(
                BreezSignerImpl::new(
                    &self.config,
                    &seed,
                    key_set_type.into(),
                    use_address_index,
                    account_number,
                )
                .map_err(|e| SdkError::Generic(e.to_string()))?,
            ),
            SignerSource::External(external_signer) => {
                use crate::signer::ExternalSignerAdapter;
                Arc::new(ExternalSignerAdapter::new(external_signer))
            }
        };

        // Create the specialized signers
        let spark_signer = Arc::new(SparkSigner::new(signer.clone()));
        let rtsync_signer = Arc::new(
            RTSyncSigner::new(signer.clone(), self.config.network)
                .map_err(|e| SdkError::Generic(e.to_string()))?,
        );
        let lnurl_auth_signer = Arc::new(LnurlAuthSignerAdapter::new(signer.clone()));

        // Resolve the shared resources: use the caller-supplied context if
        // present, otherwise spin up a default one. Either way, downstream
        // wiring reads from `context` for connection managers.
        let context = match self.context {
            Some(ctx) => ctx,
            None => {
                new_shared_sdk_context(SdkContextConfig {
                    api_key: self.config.api_key.clone(),
                    ..SdkContextConfig::new(self.config.network)
                })
                .await?
            }
        };

        // Ensure the context's parameters are the same as the config parameters.
        if context.network != self.config.network || context.api_key != self.config.api_key {
            return Err(SdkError::Generic(
                "SdkContext network/api_key do not match SdkConfig".to_string(),
            ));
        }

        // Resolve the DB pools from at most one source: the legacy
        // `with_postgres_connection_pool` / `with_mysql_connection_pool`
        // setters take precedence-by-exclusion over a pool carried by the
        // context. Passing both is a configuration error.
        #[cfg(feature = "postgres")]
        let postgres_pool: Option<Arc<crate::persist::postgres::PostgresConnectionPool>> = match (
            self.postgres_pool,
            context.postgres_pool.clone(),
        ) {
            (Some(_), Some(_)) => {
                return Err(SdkError::Generic(
                        "multiple postgres pools configured: passed both via with_postgres_connection_pool and SdkContext"
                            .to_string(),
                    ));
            }
            (Some(p), None) | (None, Some(p)) => Some(p),
            (None, None) => None,
        };
        #[cfg(feature = "mysql")]
        let mysql_pool: Option<Arc<crate::persist::mysql::MysqlConnectionPool>> = match (
            self.mysql_pool,
            context.mysql_pool.clone(),
        ) {
            (Some(_), Some(_)) => {
                return Err(SdkError::Generic(
                        "multiple mysql pools configured: passed both via with_mysql_connection_pool and SdkContext"
                            .to_string(),
                    ));
            }
            (Some(p), None) | (None, Some(p)) => Some(p),
            (None, None) => None,
        };

        let chain_service: Arc<dyn BitcoinChainService> = if let Some(service) = self.chain_service
        {
            service
        } else if let Some(cfg) = self.rest_chain_service_config {
            Arc::new(RestClientChainService::new(
                cfg.url,
                self.config.network,
                5,
                context.http_client.clone(),
                cfg.credentials
                    .map(|c| BasicAuth::new(c.username, c.password)),
                cfg.api_type,
            ))
        } else {
            let inner_client: Arc<dyn platform_utils::HttpClient> = context.http_client.clone();
            match self.config.network {
                Network::Mainnet => Arc::new(RestClientChainService::new(
                    "https://blockstream.info/api".to_string(),
                    self.config.network,
                    5,
                    inner_client,
                    None,
                    ChainApiType::Esplora,
                )),
                Network::Regtest => Arc::new(RestClientChainService::new(
                    "https://regtest-mempool.us-west-2.sparkinfra.net/api".to_string(),
                    self.config.network,
                    5,
                    inner_client,
                    match (
                        std::env::var("CHAIN_SERVICE_USERNAME"),
                        std::env::var("CHAIN_SERVICE_PASSWORD"),
                    ) {
                        (Ok(username), Ok(password)) => Some(BasicAuth::new(username, password)),
                        _ => Some(BasicAuth::new(
                            "spark-sdk".to_string(),
                            "mCMk1JqlBNtetUNy".to_string(),
                        )),
                    },
                    ChainApiType::MempoolSpace,
                )),
            }
        };

        // Validate storage configuration
        #[cfg(feature = "postgres")]
        let has_postgres = postgres_pool.is_some();
        #[cfg(not(feature = "postgres"))]
        let has_postgres = false;

        #[cfg(feature = "mysql")]
        let has_mysql = mysql_pool.is_some();
        #[cfg(not(feature = "mysql"))]
        let has_mysql = false;

        let storage_count = [
            self.storage.is_some(),
            self.storage_dir.is_some(),
            has_postgres,
            has_mysql,
        ]
        .into_iter()
        .filter(|&v| v)
        .count();
        match storage_count {
            0 => return Err(SdkError::Generic("No storage configured".to_string())),
            2.. => {
                return Err(SdkError::Generic(
                    "Multiple storage configurations provided".to_string(),
                ));
            }
            _ => {}
        }

        // Bundle the resolved Postgres pool with the tenant identity used to
        // scope every read/write so storage, tree store, and token store
        // share the same scope. The pool itself may back many SDK instances.
        #[cfg(feature = "postgres")]
        let postgres_backend = if let Some(ref pool) = postgres_pool {
            let identity = spark_signer
                .get_identity_public_key()
                .await
                .map_err(|e| SdkError::Generic(e.to_string()))?
                .serialize();
            Some((pool.inner.clone(), identity, pool.run_migration))
        } else {
            None
        };

        // Same for MySQL.
        #[cfg(feature = "mysql")]
        let mysql_backend = if let Some(ref pool) = mysql_pool {
            let identity = spark_signer
                .get_identity_public_key()
                .await
                .map_err(|e| SdkError::Generic(e.to_string()))?
                .serialize();
            Some((
                pool.inner.clone(),
                identity,
                pool.run_migration,
                pool.foreign_key_mode,
            ))
        } else {
            None
        };

        // Initialize storage
        let storage: Arc<dyn Storage> = if let Some(storage) = self.storage {
            storage
        } else if let Some(storage_dir) = self.storage_dir {
            #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
            {
                let identity_pub_key = spark_signer
                    .get_identity_public_key()
                    .await
                    .map_err(|e| SdkError::Generic(e.to_string()))?;
                default_storage(&storage_dir, self.config.network, &identity_pub_key)?
            }
            #[cfg(all(target_family = "wasm", target_os = "unknown"))]
            {
                let _ = storage_dir;
                return Err(SdkError::Generic(
                    "with_default_storage is not supported on WASM".to_string(),
                ));
            }
        } else {
            #[allow(unused_mut)]
            let mut s: Option<Arc<dyn Storage>> = None;

            #[cfg(all(
                feature = "postgres",
                not(all(target_family = "wasm", target_os = "unknown"))
            ))]
            if s.is_none()
                && let Some((ref pool, ref identity, run_migration)) = postgres_backend
            {
                s = Some(Arc::new(
                    crate::persist::postgres::PostgresStorage::new_with_pool(
                        pool.clone(),
                        identity,
                        run_migration,
                    )
                    .await
                    .map_err(|e| SdkError::Generic(e.to_string()))?,
                ));
            }

            #[cfg(all(
                feature = "mysql",
                not(all(target_family = "wasm", target_os = "unknown"))
            ))]
            if s.is_none()
                && let Some((ref pool, ref identity, run_migration, _)) = mysql_backend
            {
                s = Some(Arc::new(
                    crate::persist::mysql::MysqlStorage::new_with_pool(
                        pool.clone(),
                        identity,
                        run_migration,
                    )
                    .await
                    .map_err(|e| SdkError::Generic(e.to_string()))?,
                ));
            }

            s.ok_or_else(|| SdkError::Generic("No storage configured".to_string()))?
        };

        let user_agent = crate::default_user_agent();
        info!("Building sdk with user agent: {}", user_agent);

        let breez_server = Arc::clone(&context.breez_server);

        let fiat_service: Arc<dyn breez_sdk_common::fiat::FiatService> = match self.fiat_service {
            Some(service) => Arc::new(FiatServiceWrapper::new(service)),
            None => breez_server.clone(),
        };

        let lnurl_client: Arc<dyn platform_utils::HttpClient> = match self.lnurl_client {
            Some(client) => client,
            None => context.http_client.clone(),
        };
        let mut spark_wallet_config = if let Some(env_config) = &self.config.spark_config {
            Self::build_spark_wallet_config(self.config.network.into(), env_config)?
        } else {
            spark_wallet::SparkWalletConfig::default_config(self.config.network.into())
        };
        spark_wallet_config.operator_pool = spark_wallet_config
            .operator_pool
            .with_user_agent(Some(user_agent.clone()));
        spark_wallet_config.service_provider_config.user_agent = Some(user_agent.clone());
        let background_services_enabled = runtime.starts_background_services();
        spark_wallet_config.leaf_auto_optimize_enabled =
            background_services_enabled && self.config.leaf_optimization_config.auto_enabled;
        spark_wallet_config.leaf_optimization_options.multiplicity =
            self.config.leaf_optimization_config.multiplicity;

        let token_opt = &self.config.token_optimization_config;
        let token_options = &mut spark_wallet_config.token_outputs_optimization_options;
        token_options.target_output_count = token_opt.target_output_count;
        token_options.min_outputs_threshold = token_opt.min_outputs_threshold;
        // Only override when disabled; enabled keeps the network default interval.
        if !token_opt.auto_enabled || !background_services_enabled {
            token_options.auto_optimize_interval = None;
        }
        spark_wallet_config.max_concurrent_claims = self.config.max_concurrent_claims;

        let shutdown_sender = watch::channel::<()>(()).0;

        // Create tree store if configured
        #[allow(unused_mut)]
        let mut tree_store: Option<Arc<dyn TreeStore>> = self.tree_store;

        #[cfg(feature = "postgres")]
        if tree_store.is_none()
            && let Some((ref pool, ref identity, run_migration)) = postgres_backend
        {
            tree_store = Some(
                crate::persist::postgres::create_postgres_tree_store(
                    pool.clone(),
                    identity,
                    run_migration,
                )
                .await?,
            );
        }

        #[cfg(feature = "mysql")]
        if tree_store.is_none()
            && let Some((ref pool, ref identity, run_migration, foreign_key_mode)) = mysql_backend
        {
            tree_store = Some(
                crate::persist::mysql::create_mysql_tree_store(
                    pool.clone(),
                    identity,
                    run_migration,
                    foreign_key_mode,
                )
                .await?,
            );
        }

        // Create token output store if configured
        #[allow(unused_mut)]
        let mut token_output_store: Option<Arc<dyn TokenOutputStore>> = self.token_output_store;

        #[cfg(feature = "postgres")]
        if token_output_store.is_none()
            && let Some((ref pool, ref identity, run_migration)) = postgres_backend
        {
            token_output_store = Some(
                crate::persist::postgres::create_postgres_token_store(
                    pool.clone(),
                    identity,
                    run_migration,
                )
                .await?,
            );
        }

        #[cfg(feature = "mysql")]
        if token_output_store.is_none()
            && let Some((ref pool, ref identity, run_migration, foreign_key_mode)) = mysql_backend
        {
            token_output_store = Some(
                crate::persist::mysql::create_mysql_token_store(
                    pool.clone(),
                    identity,
                    run_migration,
                    foreign_key_mode,
                )
                .await?,
            );
        }

        #[allow(unused_mut)]
        let mut inner_session_store: Option<Arc<dyn spark_wallet::SessionStore>> = self
            .session_store
            .map(|sm| Arc::new(SessionStoreAdapter(sm)) as Arc<dyn spark_wallet::SessionStore>);

        #[cfg(feature = "postgres")]
        if inner_session_store.is_none()
            && let Some((ref pool, ref identity, run_migration)) = postgres_backend
        {
            inner_session_store = Some(
                crate::persist::postgres::create_postgres_session_store(
                    pool.clone(),
                    identity,
                    run_migration,
                )
                .await?,
            );
        }

        #[cfg(feature = "mysql")]
        if inner_session_store.is_none()
            && let Some((ref pool, ref identity, run_migration, _)) = mysql_backend
        {
            inner_session_store = Some(
                crate::persist::mysql::create_mysql_session_store(
                    pool.clone(),
                    identity,
                    run_migration,
                )
                .await?,
            );
        }

        let inner_session_store =
            inner_session_store.unwrap_or_else(|| Arc::new(InMemorySessionStore::default()));
        let inner_session_store: Arc<dyn spark_wallet::SessionStore> = Arc::new(
            crate::session_store::EncryptingSessionStore::new(
                inner_session_store,
                signer.clone(),
                self.config.network,
            )
            .map_err(|e| {
                SdkError::Generic(format!("failed to set up session token encryption: {e}"))
            })?,
        );
        let inner_session_store: Arc<dyn spark_wallet::SessionStore> = Arc::new(
            crate::session_store::CachingSessionStore::new(inner_session_store),
        );
        let mut wallet_builder =
            spark_wallet::WalletBuilder::new(spark_wallet_config, spark_signer)
                .with_cancellation_token(shutdown_sender.subscribe())
                .with_session_store(inner_session_store)
                .with_background_processing(background_services_enabled);
        if let Some(provider) = &context.jwt_header_provider {
            wallet_builder = wallet_builder.with_so_extra_header_provider(
                Arc::clone(provider) as Arc<dyn spark_wallet::HeaderProvider>
            );
        }
        if let Some(observer) = self.payment_observer {
            let observer: Arc<dyn spark_wallet::TransferObserver> =
                Arc::new(SparkTransferObserver::new(observer));
            wallet_builder = wallet_builder.with_transfer_observer(observer);
        }
        if let Some(tree_store) = tree_store {
            wallet_builder = wallet_builder.with_tree_store(tree_store);
        }
        if let Some(token_output_store) = token_output_store {
            wallet_builder = wallet_builder.with_token_output_store(token_output_store);
        }
        wallet_builder = wallet_builder.with_ssp_http_client(context.http_client.clone());
        wallet_builder = wallet_builder.with_connection_manager(context.connection_manager.clone());
        let spark_wallet = Arc::new(wallet_builder.build().await?);

        let lnurl_server_client: Option<Arc<dyn LnurlServerClient>> = match self.lnurl_server_client
        {
            Some(client) => Some(client),
            None => match &self.config.lnurl_domain {
                Some(domain) => Some(Arc::new(DefaultLnurlServerClient::new(
                    context.http_client.clone(),
                    domain.clone(),
                    self.config.api_key.clone(),
                    Arc::clone(&spark_wallet),
                ))),
                None => None,
            },
        };

        let real_time_sync_active =
            background_services_enabled && self.config.real_time_sync_server_url.is_some();
        let event_emitter = Arc::new(EventEmitter::new(real_time_sync_active));

        let storage = match &self.config.real_time_sync_server_url {
            Some(server_url) if background_services_enabled => {
                init_and_start_real_time_sync(RealTimeSyncParams {
                    server_url: server_url.clone(),
                    api_key: self.config.api_key.clone(),
                    user_agent,
                    signer: rtsync_signer,
                    storage: Arc::clone(&storage),
                    shutdown_receiver: shutdown_sender.subscribe(),
                    event_emitter: Arc::clone(&event_emitter),
                    lnurl_server_client: lnurl_server_client.clone(),
                })
                .await?
            }
            _ => storage,
        };

        // Create the MoonPay provider for buying Bitcoin
        let buy_bitcoin_provider = Arc::new(MoonpayProvider::new(breez_server.clone()));

        // Create the FlashnetTokenConverter. Client runtime starts its refunder.
        let flashnet_config = FlashnetConfig::default_config(
            self.config.network.into(),
            DEFAULT_INTEGRATOR_PUBKEY
                .parse()
                .ok()
                .map(|pubkey| IntegratorConfig {
                    pubkey,
                    fee_bps: DEFAULT_INTEGRATOR_FEE_BPS,
                }),
        );
        let flashnet_converter = Arc::new(FlashnetTokenConverter::new(
            flashnet_config,
            Arc::clone(&storage),
            Arc::clone(&spark_wallet),
            self.config.network,
            context.http_client.clone(),
        ));
        let token_converter: Arc<dyn TokenConverter> = flashnet_converter;

        // Create sync coordinator for the client runtime's sync loop
        let sync_coordinator = SyncCoordinator::new();
        // Create StableBalance if configured. Client runtime starts its worker.
        // It registers itself as event middleware (must be before TokenConversionMiddleware
        // so it can see conversion child payment events for deferred task resolution)
        let stable_balance = if let Some(config) = &self.config.stable_balance_config {
            let stable_balance = Arc::new(
                StableBalance::new(
                    config.clone(),
                    Arc::clone(&token_converter),
                    Arc::clone(&spark_wallet),
                    Arc::clone(&storage),
                    Arc::clone(&event_emitter),
                )
                .await,
            );
            Some(stable_balance)
        } else {
            None
        };

        // Register TokenConversionMiddleware to suppress conversion child events
        // before they reach external listeners (after StableBalance middleware)
        event_emitter
            .add_middleware(Box::new(TokenConversionMiddleware))
            .await;

        // Create the SDK instance
        let sdk = BreezSdk::init_and_start(BreezSdkParams {
            config: self.config,
            storage,
            chain_service,
            fiat_service,
            lnurl_client,
            lnurl_server_client,
            lnurl_auth_signer,
            shutdown_sender,
            runtime,
            spark_wallet,
            event_emitter,
            buy_bitcoin_provider,
            token_converter,
            stable_balance,
            sync_coordinator,
        })
        .await?;
        debug!("Initialized and started breez sdk.");

        Ok(sdk)
    }
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
fn default_storage(
    data_dir: &str,
    network: Network,
    identity_pub_key: &spark_wallet::PublicKey,
) -> Result<Arc<dyn Storage>, SdkError> {
    let db_path = crate::default_storage_path(data_dir, &network, identity_pub_key)?;
    let storage = Arc::new(crate::SqliteStorage::new(&db_path)?);
    Ok(storage)
}

#[cfg(test)]
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod tests {
    use super::SdkBuilder;
    use crate::{Network, default_config};

    #[test]
    fn default_config_spark_config_builds_valid_wallet_config() {
        for network in [Network::Mainnet, Network::Regtest] {
            let config = default_config(network);
            let spark_config = config
                .spark_config
                .as_ref()
                .expect("default_config must populate spark_config");
            SdkBuilder::build_spark_wallet_config(network.into(), spark_config).unwrap_or_else(
                |e| {
                    panic!(
                        "default_config({network:?}).spark_config failed to build SparkWalletConfig: {e}"
                    )
                },
            );
        }
    }

    #[tokio::test]
    async fn server_mode_rejects_stable_balance_config() {
        use crate::{SdkError, StableBalanceConfig, StableBalanceToken, default_server_config};

        let mut config = default_server_config(Network::Regtest);
        config.stable_balance_config = Some(StableBalanceConfig {
            tokens: vec![StableBalanceToken {
                label: "USDB".to_string(),
                token_identifier: "btkn1test".to_string(),
            }],
            default_active_label: None,
            threshold_sats: None,
            max_slippage_bps: None,
        });

        let seed = test_seed();
        let result = SdkBuilder::new(config, seed).build().await;
        match result {
            Err(SdkError::InvalidInput(message)) => {
                assert!(message.contains("stable_balance_config"));
            }
            Err(err) => panic!("expected InvalidInput error, got {err:?}"),
            Ok(_) => panic!("expected server mode with Stable Balance config to fail"),
        }
    }

    #[tokio::test]
    async fn server_mode_rejects_real_time_sync_server_url() {
        use crate::{SdkError, default_server_config};

        let mut config = default_server_config(Network::Regtest);
        config.real_time_sync_server_url = Some("https://example.com".to_string());

        let seed = test_seed();
        let result = SdkBuilder::new(config, seed).build().await;
        match result {
            Err(SdkError::InvalidInput(message)) => {
                assert!(message.contains("real_time_sync_server_url"));
            }
            Err(err) => panic!("expected InvalidInput error, got {err:?}"),
            Ok(_) => panic!("expected server mode with real_time_sync_server_url to fail"),
        }
    }

    #[tokio::test]
    async fn server_mode_rejects_leaf_optimization_auto_enabled() {
        use crate::{SdkError, default_server_config};

        let mut config = default_server_config(Network::Regtest);
        config.leaf_optimization_config.auto_enabled = true;

        let seed = test_seed();
        let result = SdkBuilder::new(config, seed).build().await;
        match result {
            Err(SdkError::InvalidInput(message)) => {
                assert!(message.contains("leaf_optimization_config.auto_enabled"));
            }
            Err(err) => panic!("expected InvalidInput error, got {err:?}"),
            Ok(_) => panic!("expected server mode with optimization auto_enabled to fail"),
        }
    }

    #[tokio::test]
    async fn server_mode_rejects_token_optimization_auto_enabled() {
        use crate::{SdkError, default_server_config};

        let mut config = default_server_config(Network::Regtest);
        config.token_optimization_config.auto_enabled = true;

        let seed = test_seed();
        let result = SdkBuilder::new(config, seed).build().await;
        match result {
            Err(SdkError::InvalidInput(message)) => {
                assert!(message.contains("token_optimization_config.auto_enabled"));
            }
            Err(err) => panic!("expected InvalidInput error, got {err:?}"),
            Ok(_) => panic!("expected server mode with optimization auto_enabled to fail"),
        }
    }

    /// Mainnet SDK with a caller-supplied Regtest context errors at `build()`
    /// — the context has no JWT provider so the partner JWT would be silently
    /// disabled.
    #[tokio::test]
    async fn build_errors_on_network_mismatch() {
        use crate::{SdkContextConfig, new_shared_sdk_context};
        let mut config = default_config(Network::Mainnet);
        config.api_key = Some("partner-key".to_string());
        let ctx = new_shared_sdk_context(SdkContextConfig {
            api_key: Some("partner-key".to_string()),
            ..SdkContextConfig::new(Network::Regtest)
        })
        .await
        .expect("regtest context");
        let err = SdkBuilder::new(config, test_seed())
            .with_shared_context(ctx)
            .with_default_storage("/tmp/breez-sdk-test-network-mismatch".to_string())
            .build()
            .await
            .err()
            .expect("expected network-mismatch error");
        assert!(
            err.to_string().contains("network/api_key do not match"),
            "unexpected error: {err}"
        );
    }

    /// Mainnet SDK with a Mainnet context whose `api_key` differs from
    /// `Config`'s errors at `build()` — the JWT provider would sign with a
    /// different key than the integrator intended.
    #[tokio::test]
    #[allow(clippy::manual_assert)]
    async fn build_errors_on_api_key_mismatch() {
        use crate::{SdkContextConfig, new_shared_sdk_context};
        let mut config = default_config(Network::Mainnet);
        config.api_key = Some("intended-key".to_string());
        let ctx = new_shared_sdk_context(SdkContextConfig {
            api_key: Some("wrong-key".to_string()),
            ..SdkContextConfig::new(Network::Mainnet)
        })
        .await
        .expect("mainnet context");
        let err = SdkBuilder::new(config, test_seed())
            .with_shared_context(ctx)
            .with_default_storage("/tmp/breez-sdk-test-key-mismatch".to_string())
            .build()
            .await
            .err()
            .expect("expected api_key-mismatch error");
        assert!(
            err.to_string().contains("network/api_key do not match"),
            "unexpected error: {err}"
        );
    }

    fn test_seed() -> crate::Seed {
        crate::Seed::Mnemonic {
            mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon \
                       abandon abandon about"
                .to_string(),
            passphrase: None,
        }
    }
}
