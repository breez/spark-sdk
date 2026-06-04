#![cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    allow(clippy::arc_with_non_send_sync)
)]
use std::sync::Arc;

use breez_sdk_common::buy::moonpay::MoonpayProvider;

use spark_wallet::{InMemorySessionStore, SparkWalletConfig};
use spark_wallet::{Signer, SparkWallet};
use tokio::sync::watch;
use tracing::{debug, info};

use flashnet::{FlashnetConfig, IntegratorConfig, OrchestraConfig};

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
    persist::backend::StorageBackend,
    realtime_sync::{RealTimeSyncParams, init_and_start_real_time_sync},
    sdk::{BreezSdk, BreezSdkParams, SyncCoordinator, runtime_from_config},
    sdk_context::{SdkContext, SdkContextConfig, new_shared_sdk_context},
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

    storage: Option<Arc<dyn StorageBackend>>,
    chain_service: Option<Arc<dyn BitcoinChainService>>,
    rest_chain_service_config: Option<RestChainServiceConfig>,
    fiat_service: Option<Arc<dyn FiatService>>,
    lnurl_client: Option<Arc<dyn platform_utils::HttpClient>>,
    lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    payment_observer: Option<Arc<dyn PaymentObserver>>,
    context: Option<Arc<SdkContext>>,
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
            storage: None,
            chain_service: None,
            rest_chain_service_config: None,
            fiat_service: None,
            lnurl_client: None,
            lnurl_server_client: None,
            payment_observer: None,
            context: None,
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
            storage: None,
            chain_service: None,
            rest_chain_service_config: None,
            fiat_service: None,
            lnurl_client: None,
            lnurl_server_client: None,
            payment_observer: None,
            context: None,
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

    #[cfg(feature = "sqlite")]
    #[must_use]
    /// Sets the root storage directory to initialize the default storage with.
    /// This initializes both storage and real-time sync storage with the
    /// default implementations.
    /// Arguments:
    /// - `storage_dir`: The data directory for storage.
    pub fn with_default_storage(self, storage_dir: String) -> Self {
        self.with_storage_backend(crate::default_storage(storage_dir))
    }

    #[must_use]
    /// Sets the storage backend to be used by the SDK.
    ///
    /// Build the [`StorageBackend`](crate::StorageBackend) with
    /// [`default_storage`](crate::default_storage),
    /// [`postgres_storage`](crate::postgres_storage),
    /// [`mysql_storage`](crate::mysql_storage) or
    /// [`custom_storage`](crate::custom_storage).
    /// Arguments:
    /// - `storage`: The storage backend to be used.
    pub fn with_storage_backend(mut self, storage: Arc<dyn StorageBackend>) -> Self {
        self.storage = Some(storage);
        self
    }

    #[must_use]
    /// **Deprecated.** Use
    /// [`with_storage_backend`](Self::with_storage_backend) with
    /// [`custom_storage`](crate::custom_storage).
    /// Arguments:
    /// - `storage`: The storage implementation to be used.
    #[deprecated(note = "use `with_storage_backend(custom_storage(storage))`")]
    pub fn with_storage(self, storage: Arc<dyn crate::Storage>) -> Self {
        self.with_storage_backend(crate::custom_storage(storage))
    }

    /// **Deprecated.** Use
    /// [`with_storage_backend`](Self::with_storage_backend) with
    /// [`postgres_storage`](crate::postgres_storage).
    #[cfg(feature = "postgres")]
    #[deprecated(note = "use `with_storage_backend(postgres_storage(config)?)`")]
    pub fn with_postgres_backend(
        self,
        config: crate::persist::postgres::PostgresStorageConfig,
    ) -> Result<Self, SdkError> {
        Ok(self.with_storage_backend(crate::postgres_storage(config)?))
    }

    /// **Deprecated.** Use
    /// [`with_storage_backend`](Self::with_storage_backend) with
    /// [`mysql_storage`](crate::mysql_storage).
    #[cfg(feature = "mysql")]
    #[deprecated(note = "use `with_storage_backend(mysql_storage(config)?)`")]
    pub fn with_mysql_backend(
        self,
        config: crate::persist::mysql::MysqlStorageConfig,
    ) -> Result<Self, SdkError> {
        Ok(self.with_storage_backend(crate::mysql_storage(config)?))
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
            if self.config.cross_chain_config.is_some() {
                return Err(SdkError::InvalidInput(
                    "cross_chain_config must be None when background_tasks_enabled is false"
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

        // Resolve the single storage backend. It comes either from the
        // builder's `with_storage` or from a shared `SdkContext` — exactly one
        // must supply it. All per-database wiring lives behind
        // `StorageBackend::create_stores`.
        let storage_backend: Arc<dyn StorageBackend> =
            match (self.storage, context.storage_backend.clone()) {
                (Some(storage), None) => storage,
                (None, Some(backend)) => backend,
                (Some(_), Some(_)) => {
                    return Err(SdkError::Generic(
                        "storage is configured on both the SdkBuilder and the shared SdkContext"
                            .to_string(),
                    ));
                }
                (None, None) => {
                    return Err(SdkError::Generic("No storage configured".to_string()));
                }
            };
        let identity_public_key = spark_signer
            .get_identity_public_key()
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let resolved = storage_backend
            .create_stores(
                self.config.network,
                identity_public_key.serialize().to_vec(),
            )
            .await?;
        let storage = resolved.storage.clone();
        let tree_store = resolved.tree_store.clone();
        let token_output_store = resolved.token_output_store.clone();
        let session_store = resolved.session_store.clone();

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

        let inner_session_store =
            session_store.unwrap_or_else(|| Arc::new(InMemorySessionStore::default()));
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

        // Create sync coordinator early so downstream services (stable
        // balance, lightning sender, …) can trigger syncs after their
        // respective flows.
        let sync_coordinator = SyncCoordinator::new();

        // Shared lightning-send helper used by `send_bolt11_invoice` and
        // by cross-chain providers that pay LN invoices (currently: Boltz
        // reverse swap).
        let lightning_sender = Arc::new(crate::sdk::LightningSender::new(
            Arc::clone(&spark_wallet),
            Arc::clone(&storage),
            sync_coordinator.clone(),
            Arc::clone(&event_emitter),
            shutdown_sender.clone(),
        ));

        let cross_chain_providers = build_cross_chain_providers(
            &self.config,
            &spark_wallet,
            &storage,
            &lightning_sender,
            shutdown_sender.subscribe(),
        )
        .await;

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
        let token_converter: Arc<dyn TokenConverter> = Arc::new(FlashnetTokenConverter::new(
            flashnet_config,
            Arc::clone(&storage),
            Arc::clone(&spark_wallet),
            Arc::clone(&event_emitter),
            self.config.network,
            context.http_client.clone(),
        ));

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
            cross_chain_providers,
            lightning_sender,
        })
        .await?;
        debug!("Initialized and started breez sdk.");

        Ok(sdk)
    }
}

/// Builds the cross-chain provider map. Each provider owns its own HTTP
/// client, route cache, and background monitor task. Returns an empty map
/// when `config.cross_chain_config` is unset.
async fn build_cross_chain_providers(
    config: &Config,
    spark_wallet: &Arc<SparkWallet>,
    storage: &Arc<dyn crate::persist::Storage>,
    lightning_sender: &Arc<crate::sdk::LightningSender>,
    shutdown_receiver: watch::Receiver<()>,
) -> crate::cross_chain::CrossChainProviders {
    let mut providers = crate::cross_chain::CrossChainProviders::new();
    if config.cross_chain_config.is_none() {
        return providers;
    }

    let maybe_orchestra_config = OrchestraConfig::default_for_network(config.network.into());
    if let Some(orchestra_config) = maybe_orchestra_config {
        providers.insert(
            crate::cross_chain::CrossChainProvider::Orchestra,
            Arc::new(crate::cross_chain::OrchestraService::new(
                orchestra_config,
                Arc::clone(spark_wallet),
                Arc::clone(storage),
                shutdown_receiver,
            )),
        );
    }

    match build_boltz_service(
        config.network,
        Arc::clone(spark_wallet),
        Arc::clone(storage),
        Arc::clone(lightning_sender),
    )
    .await
    {
        Ok(Some(service)) => {
            providers.insert(crate::cross_chain::CrossChainProvider::Boltz, service);
        }
        Ok(None) => {
            info!(
                "Boltz provider skipped: no default configuration for network {:?}",
                config.network
            );
        }
        Err(e) => {
            tracing::error!("Failed to initialize Boltz provider: {e:?}");
        }
    }

    providers
}

#[cfg(test)]
#[cfg(feature = "sqlite")]
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

    #[tokio::test]
    async fn server_mode_rejects_cross_chain_config() {
        use crate::{CrossChainConfig, SdkError, default_server_config};

        let mut config = default_server_config(Network::Regtest);
        config.cross_chain_config = Some(CrossChainConfig::default());

        let seed = test_seed();
        let result = SdkBuilder::new(config, seed).build().await;
        match result {
            Err(SdkError::InvalidInput(message)) => {
                assert!(message.contains("cross_chain_config"));
            }
            Err(err) => panic!("expected InvalidInput error, got {err:?}"),
            Ok(_) => panic!("expected server mode with cross_chain_config to fail"),
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

/// Loads or generates the device-local Boltz instance handle (random 32-byte
/// seed + instance id). In v1 this is kept local only — cross-device recovery
/// of swaps lands with the v2 submarine-swap feature.
///
/// Cross-device consequence in v1: a user who restores from mnemonic on a
/// second device cannot claim destination-chain payouts for reverse swaps
/// initiated on the first device. Funds are not at risk — Boltz's
/// hold-invoice timeout refunds the lightning leg — but the second device
/// is blind to the in-flight swap until it terminates on Boltz's side.
/// v2 is expected to retroactively publish the existing local seed on
/// first boot so new devices can bootstrap from rtsync.
async fn load_or_create_boltz_instance(
    storage: &Arc<dyn crate::Storage>,
) -> Result<BoltzInstanceHandle, SdkError> {
    use bitcoin::secp256k1::rand::{RngCore, thread_rng};

    const BOLTZ_INSTANCE_KEY: &str = "boltz_instance_current";

    if let Some(raw) = storage
        .get_cached_item(BOLTZ_INSTANCE_KEY.to_string())
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to read Boltz instance: {e}")))?
    {
        let handle: BoltzInstanceHandle = serde_json::from_str(&raw)
            .map_err(|e| SdkError::Generic(format!("Corrupted Boltz instance handle: {e}")))?;
        return Ok(handle);
    }

    let mut seed = [0u8; 32];
    thread_rng().fill_bytes(&mut seed);
    let handle = BoltzInstanceHandle {
        instance_id: uuid::Uuid::new_v4().to_string(),
        seed_hex: hex::encode(seed),
    };
    let serialized = serde_json::to_string(&handle)
        .map_err(|e| SdkError::Generic(format!("Failed to serialize Boltz instance: {e}")))?;
    storage
        .set_cached_item(BOLTZ_INSTANCE_KEY.to_string(), serialized)
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to persist Boltz instance: {e}")))?;
    Ok(handle)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BoltzInstanceHandle {
    instance_id: String,
    seed_hex: String,
}

/// Initializes the Boltz reverse-swap cross-chain provider: loads or creates
/// the local instance seed, constructs the inner `BoltzClient`, registers the
/// event listener, resumes any active swaps, and returns an SDK-side wrapper
/// ready to be inserted into the provider registry.
async fn build_boltz_service(
    network: Network,
    spark_wallet: Arc<spark_wallet::SparkWallet>,
    storage: Arc<dyn crate::Storage>,
    lightning_sender: Arc<crate::sdk::LightningSender>,
) -> Result<Option<Arc<dyn crate::cross_chain::CrossChainService>>, SdkError> {
    let Some(client_config) = crate::cross_chain::BoltzService::default_client_config(network)
    else {
        return Ok(None);
    };

    let handle = load_or_create_boltz_instance(&storage).await?;
    let seed = hex::decode(&handle.seed_hex)
        .map_err(|e| SdkError::Generic(format!("Invalid Boltz instance seed hex: {e}")))?;

    let adapter = Arc::new(
        crate::cross_chain::boltz_storage_adapter::BoltzStorageAdapter::new(
            Arc::clone(&storage),
            handle.instance_id.clone(),
        ),
    );

    let client = boltz_client::BoltzService::new(client_config, &seed, adapter)
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to construct Boltz client: {e}")))?;

    let listener = Box::new(
        crate::cross_chain::boltz_event_listener::BoltzSdkEventListener::new(Arc::clone(&storage)),
    );
    client.add_event_listener(listener).await;

    if let Err(e) = client.resume_swaps().await {
        tracing::warn!("Boltz resume_swaps failed on startup: {e:?}");
    }

    Ok(Some(Arc::new(crate::cross_chain::BoltzService::new(
        Arc::new(client),
        spark_wallet,
        storage,
        lightning_sender,
    ))))
}
