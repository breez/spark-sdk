#![cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    allow(clippy::arc_with_non_send_sync)
)]
use std::sync::Arc;

use breez_sdk_common::{
    breez_server::{BreezServer, PRODUCTION_BREEZSERVER_URL},
    buy::moonpay::MoonpayProvider,
};
use platform_utils::DefaultHttpClient;

#[cfg(not(target_family = "wasm"))]
use spark_wallet::Signer;
use spark_wallet::{SparkWalletConfig, TokenOutputStore, TreeStore};
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
    sdk::{BreezSdk, BreezSdkParams, SyncCoordinator},
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
    postgres_backend_config: Option<crate::persist::postgres::PostgresStorageConfig>,
    chain_service: Option<Arc<dyn BitcoinChainService>>,
    fiat_service: Option<Arc<dyn FiatService>>,
    lnurl_client: Option<Arc<dyn platform_utils::HttpClient>>,
    lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    payment_observer: Option<Arc<dyn PaymentObserver>>,
    tree_store: Option<Arc<dyn TreeStore>>,
    token_output_store: Option<Arc<dyn TokenOutputStore>>,
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
            postgres_backend_config: None,
            chain_service: None,
            fiat_service: None,
            lnurl_client: None,
            lnurl_server_client: None,
            payment_observer: None,
            tree_store: None,
            token_output_store: None,
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
            postgres_backend_config: None,
            chain_service: None,
            fiat_service: None,
            lnurl_client: None,
            lnurl_server_client: None,
            payment_observer: None,
            tree_store: None,
            token_output_store: None,
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

    /// Sets `PostgreSQL` as the backend for all stores (storage, tree store, and token store).
    /// The store instances will be created during `build()`.
    /// Arguments:
    /// - `config`: The `PostgreSQL` storage configuration.
    #[must_use]
    #[cfg(feature = "postgres")]
    pub fn with_postgres_backend(
        mut self,
        config: crate::persist::postgres::PostgresStorageConfig,
    ) -> Self {
        self.postgres_backend_config = Some(config);
        self
    }

    /// Sets the chain service to be used by the SDK.
    /// Arguments:
    /// - `chain_service`: The chain service to be used.
    #[must_use]
    pub fn with_chain_service(mut self, chain_service: Arc<dyn BitcoinChainService>) -> Self {
        self.chain_service = Some(chain_service);
        self
    }

    /// Sets the REST chain service to be used by the SDK.
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
        self.chain_service = Some(Arc::new(RestClientChainService::new(
            url,
            self.config.network,
            5,
            Box::new(DefaultHttpClient::default()),
            credentials.map(|c| BasicAuth::new(c.username, c.password)),
            api_type,
        )));
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

        let chain_service = if let Some(service) = self.chain_service {
            service
        } else {
            let inner_client = DefaultHttpClient::default();
            match self.config.network {
                Network::Mainnet => Arc::new(RestClientChainService::new(
                    "https://blockstream.info/api".to_string(),
                    self.config.network,
                    5,
                    Box::new(inner_client),
                    None,
                    ChainApiType::Esplora,
                )),
                Network::Regtest => Arc::new(RestClientChainService::new(
                    "https://regtest-mempool.us-west-2.sparkinfra.net/api".to_string(),
                    self.config.network,
                    5,
                    Box::new(inner_client),
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
        let has_postgres = self.postgres_backend_config.is_some();
        #[cfg(not(feature = "postgres"))]
        let has_postgres = false;

        let storage_count = [
            self.storage.is_some(),
            self.storage_dir.is_some(),
            has_postgres,
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

        // Create a shared PostgreSQL pool if postgres backend is configured.
        // This single pool is reused for storage, tree store, and token store.
        #[cfg(feature = "postgres")]
        let postgres_pool = if let Some(ref postgres_config) = self.postgres_backend_config {
            Some(
                crate::persist::postgres::create_pool(postgres_config)
                    .map_err(|e| SdkError::Generic(e.to_string()))?,
            )
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
            #[cfg(all(
                feature = "postgres",
                not(all(target_family = "wasm", target_os = "unknown"))
            ))]
            if let Some(ref pool) = postgres_pool {
                Arc::new(
                    crate::persist::postgres::PostgresStorage::new_with_pool(pool.clone())
                        .await
                        .map_err(|e| SdkError::Generic(e.to_string()))?,
                )
            } else {
                return Err(SdkError::Generic("No storage configured".to_string()));
            }
            #[cfg(not(all(
                feature = "postgres",
                not(all(target_family = "wasm", target_os = "unknown"))
            )))]
            {
                return Err(SdkError::Generic("No storage configured".to_string()));
            }
        };

        let user_agent = format!(
            "{}/{}",
            crate::built_info::PKG_NAME,
            crate::built_info::GIT_VERSION.unwrap_or(crate::built_info::PKG_VERSION),
        );
        info!("Building sdk with user agent: {}", user_agent);

        let breez_server = Arc::new(
            BreezServer::new(PRODUCTION_BREEZSERVER_URL, None, &user_agent)
                .map_err(|e| SdkError::Generic(e.to_string()))?,
        );

        let fiat_service: Arc<dyn breez_sdk_common::fiat::FiatService> = match self.fiat_service {
            Some(service) => Arc::new(FiatServiceWrapper::new(service)),
            None => breez_server.clone(),
        };

        let lnurl_client: Arc<dyn platform_utils::HttpClient> = match self.lnurl_client {
            Some(client) => client,
            None => Arc::new(DefaultHttpClient::default()),
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
        spark_wallet_config.leaf_auto_optimize_enabled =
            self.config.optimization_config.auto_enabled;
        spark_wallet_config.leaf_optimization_options.multiplicity =
            self.config.optimization_config.multiplicity;
        spark_wallet_config.max_concurrent_claims = self.config.max_concurrent_claims;

        let shutdown_sender = watch::channel::<()>(()).0;

        // Create tree store if configured
        #[allow(unused_mut)]
        let mut tree_store: Option<Arc<dyn TreeStore>> = self.tree_store;

        #[cfg(feature = "postgres")]
        if tree_store.is_none()
            && let Some(ref pool) = postgres_pool
        {
            tree_store =
                Some(crate::persist::postgres::create_postgres_tree_store(pool.clone()).await?);
        }

        // Create token output store if configured
        #[allow(unused_mut)]
        let mut token_output_store: Option<Arc<dyn TokenOutputStore>> = self.token_output_store;

        #[cfg(feature = "postgres")]
        if token_output_store.is_none()
            && let Some(ref pool) = postgres_pool
        {
            token_output_store =
                Some(crate::persist::postgres::create_postgres_token_store(pool.clone()).await?);
        }

        let mut wallet_builder =
            spark_wallet::WalletBuilder::new(spark_wallet_config, spark_signer)
                .with_cancellation_token(shutdown_sender.subscribe());
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
        let spark_wallet = Arc::new(wallet_builder.build().await?);

        let lnurl_server_client: Option<Arc<dyn LnurlServerClient>> = match self.lnurl_server_client
        {
            Some(client) => Some(client),
            None => match &self.config.lnurl_domain {
                Some(domain) => {
                    let http_client: Arc<dyn platform_utils::HttpClient> =
                        Arc::new(DefaultHttpClient::default());
                    Some(Arc::new(DefaultLnurlServerClient::new(
                        http_client,
                        domain.clone(),
                        self.config.api_key.clone(),
                        Arc::clone(&spark_wallet),
                    )))
                }
                None => None,
            },
        };

        let event_emitter = Arc::new(EventEmitter::new(
            self.config.real_time_sync_server_url.is_some(),
        ));

        let storage = if let Some(server_url) = &self.config.real_time_sync_server_url {
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
        } else {
            storage
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

        // Create the FlashnetTokenConverter (spawns its own refunder background task)
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
        // Build cross-chain providers. Each provider owns its own HTTP
        // client, route cache, and background monitor task.
        let mut cross_chain_providers = crate::cross_chain::CrossChainProviders::new();
        if let Some(orchestra_config) = &flashnet_config.orchestra {
            cross_chain_providers.insert(
                crate::cross_chain::CrossChainProvider::Orchestra,
                std::sync::Arc::new(crate::cross_chain::OrchestraService::new(
                    orchestra_config.clone(),
                    Arc::clone(&spark_wallet),
                    Arc::clone(&storage),
                    shutdown_sender.subscribe(),
                )),
            );
        }

        if let Some(boltz_config) = self.config.boltz.clone() {
            match build_boltz_service(
                &boltz_config,
                self.config.network,
                Arc::clone(&spark_wallet),
                Arc::clone(&storage),
                Arc::clone(&lightning_sender),
            )
            .await
            {
                Ok(Some(service)) => {
                    cross_chain_providers
                        .insert(crate::cross_chain::CrossChainProvider::Boltz, service);
                }
                Ok(None) => {
                    info!(
                        "Boltz provider skipped: no default configuration for network {:?}",
                        self.config.network
                    );
                }
                Err(e) => {
                    tracing::warn!("Failed to initialize Boltz provider: {e:?}");
                }
            }
        }

        let token_converter: Arc<dyn TokenConverter> = Arc::new(FlashnetTokenConverter::new(
            flashnet_config,
            Arc::clone(&storage),
            Arc::clone(&spark_wallet),
            self.config.network,
            shutdown_sender.subscribe(),
        ));

        // Create StableBalance if configured. It spawns its own background tasks
        // and registers itself as event middleware (must be before TokenConversionMiddleware
        // so it can see conversion child payment events for deferred task resolution)
        let stable_balance = if let Some(config) = &self.config.stable_balance_config {
            Some(Arc::new(
                StableBalance::new(
                    config.clone(),
                    Arc::clone(&token_converter),
                    Arc::clone(&spark_wallet),
                    Arc::clone(&storage),
                    shutdown_sender.subscribe(),
                    Arc::clone(&event_emitter),
                    sync_coordinator.clone(),
                )
                .await,
            ))
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
            spark_wallet,
            event_emitter,
            buy_bitcoin_provider,
            token_converter,
            stable_balance,
            sync_coordinator,
            cross_chain_providers,
            lightning_sender,
        })?;
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
    storage: &Arc<dyn Storage>,
) -> Result<BoltzInstanceHandle, SdkError> {
    use bitcoin::secp256k1::rand::{RngCore, thread_rng};

    const BOLTZ_INSTANCE_KEY: &str = "boltz_instance_current";

    if let Some(raw) = storage
        .get_cached_item(BOLTZ_INSTANCE_KEY.to_string())
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to read Boltz instance: {e}")))?
        && let Ok(handle) = serde_json::from_str::<BoltzInstanceHandle>(&raw)
    {
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
    config: &crate::models::BoltzConfig,
    network: Network,
    spark_wallet: Arc<spark_wallet::SparkWallet>,
    storage: Arc<dyn Storage>,
    lightning_sender: Arc<crate::sdk::LightningSender>,
) -> Result<Option<Arc<dyn crate::cross_chain::CrossChainService>>, SdkError> {
    let Some(client_config) = crate::cross_chain::BoltzService::default_client_config(
        network,
        config.referral_id.clone(),
    ) else {
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
        network,
        lightning_sender,
    ))))
}
