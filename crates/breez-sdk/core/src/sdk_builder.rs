#![cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    allow(clippy::arc_with_non_send_sync)
)]
use std::sync::Arc;

use breez_sdk_common::breez_server::BreezServer;
use breez_sdk_common::buy::moonpay::MoonpayProvider;

use spark_wallet::{
    InMemorySessionStore, SessionStore, SparkSigner, SparkWallet, SparkWalletConfig,
};
use tokio::sync::watch;
use tracing::{debug, info};

use flashnet::{FlashnetConfig, IntegratorConfig};

use crate::{
    Credentials, EventEmitter, FiatService, FiatServiceWrapper, Network, Seed,
    chain::{
        BitcoinChainService,
        rest_client::{BasicAuth, ChainApiType, RestClientChainService},
    },
    error::SdkError,
    lnurl::{DefaultLnurlServerClient, LnurlServerClient},
    models::Config,
    payment_observer::{PaymentObserver, SparkTransferObserver},
    persist::backend::{ResolvedStores, StorageBackend},
    realtime_sync::{RealTimeSyncParams, init_and_start_real_time_sync},
    sdk::{BreezSdk, BreezSdkParams, SyncCoordinator, runtime_from_config},
    sdk_context::{SdkContext, SdkContextConfig, new_shared_sdk_context},
    signer::{breez::BreezSignerImpl, lnurl_auth::LnurlAuthSignerAdapter, rtsync::RTSyncSigner},
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
/// External SDK-layer signer, in one of its two capability profiles. The profile
/// is chosen by the type the integrator supplies at connect, not a runtime flag:
/// a [`SigningOnly`](ExternalBreez::SigningOnly) signer can't perform the SDK's
/// local ECIES/HMAC operations.
#[derive(Clone)]
enum ExternalBreez {
    Full(Arc<dyn crate::signer::ExternalBreezSigner>),
    SigningOnly(Arc<dyn crate::signer::ExternalSigningSigner>),
}

#[derive(Clone)]
enum SignerSource {
    Seed {
        seed: Seed,
        account_number: Option<u32>,
    },
    External {
        breez: ExternalBreez,
        spark: Arc<dyn crate::signer::ExternalSparkSigner>,
    },
}

/// The signers derived from a single signer source. `ecies` is absent for a
/// signing-only signer; the features that depend on it (`rtsync`, session-token
/// encryption, cross-chain) and on HMAC (`lnurl_auth`) are then absent too.
struct Signers {
    ecies: Option<Arc<dyn crate::signer::EciesSigner>>,
    spark: Arc<dyn SparkSigner>,
    rtsync: Option<Arc<RTSyncSigner>>,
    lnurl_auth: Option<Arc<LnurlAuthSignerAdapter>>,
}

/// Inputs to [`build_spark_wallet`] — bundled to avoid an >8-argument helper.
struct BuildSparkWalletParams {
    config: SparkWalletConfig,
    spark_signer: Arc<dyn SparkSigner>,
    session_store: Arc<dyn SessionStore>,
    /// `Some` only in client mode. Wiring it in server mode would leave the
    /// wallet holding a stored `cancellation_token` receiver that's never
    /// consumed (server mode never calls `start_background_processing`), which
    /// would stall `disconnect`'s `Sender::closed()` await forever.
    shutdown_receiver: Option<watch::Receiver<()>>,
    tree_store: Option<Arc<dyn spark_wallet::TreeStore>>,
    token_output_store: Option<Arc<dyn spark_wallet::TokenOutputStore>>,
    payment_observer: Option<Arc<dyn PaymentObserver>>,
    context: Arc<SdkContext>,
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

    /// Creates a new `SdkBuilder` with the provided configuration and external signers.
    ///
    /// # Arguments
    /// - `config`: The configuration to be used.
    /// - `breez_signer`: External signer for non-Spark SDK signing (LNURL-auth,
    ///   real-time sync, message signing, ECIES).
    /// - `spark_signer`: External high-level Spark signer for the Spark wallet.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new_with_signer(
        config: Config,
        breez_signer: Arc<dyn crate::signer::ExternalBreezSigner>,
        spark_signer: Arc<dyn crate::signer::ExternalSparkSigner>,
    ) -> Self {
        Self::with_external_signer(config, ExternalBreez::Full(breez_signer), spark_signer)
    }

    /// Creates a new `SdkBuilder` with a signing-only external signer.
    ///
    /// Use this for a signer that can't perform the SDK's local ECIES/HMAC
    /// operations (for example a policy-restricted enclave). The SDK keeps
    /// session tokens in plaintext and disables the features that rely on
    /// ECIES/HMAC.
    ///
    /// # Arguments
    /// - `config`: The configuration to be used.
    /// - `breez_signer`: Signing-only external signer for non-Spark SDK signing.
    /// - `spark_signer`: External high-level Spark signer for the Spark wallet.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new_with_signing_only_signer(
        config: Config,
        breez_signer: Arc<dyn crate::signer::ExternalSigningSigner>,
        spark_signer: Arc<dyn crate::signer::ExternalSparkSigner>,
    ) -> Self {
        Self::with_external_signer(
            config,
            ExternalBreez::SigningOnly(breez_signer),
            spark_signer,
        )
    }

    fn with_external_signer(
        config: Config,
        breez: ExternalBreez,
        spark_signer: Arc<dyn crate::signer::ExternalSparkSigner>,
    ) -> Self {
        SdkBuilder {
            config,
            signer_source: SignerSource::External {
                breez,
                spark: spark_signer,
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

    /// Sets the account number for key derivation. All wallet keys derive from
    /// the seed at `m/8797555'/<account number>'`, so each account number
    /// yields an independent wallet from the same seed.
    ///
    /// When unset, the account number defaults to 0 on Regtest and 1 on all
    /// other networks.
    ///
    /// Note: This only applies when using a seed-based signer. It has no effect
    /// when using an external signer (created with `new_with_signer`).
    ///
    /// # Arguments
    /// - `account_number`: The account number in the derivation path.
    #[must_use]
    pub fn with_account_number(mut self, account_number: u32) -> Self {
        if let SignerSource::Seed {
            account_number: ref mut an,
            ..
        } = self.signer_source
        {
            *an = Some(account_number);
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
    ) -> Result<SparkWalletConfig, SdkError> {
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
                let ca_cert = op.ca_cert_pem.as_ref().map(|pem| pem.as_bytes().to_vec());
                SparkWalletConfig::create_operator_config(
                    op.id as usize,
                    &op.identifier,
                    &op.address,
                    ca_cert.as_deref(),
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

    /// Builds the `BreezSdk` instance from the configured components, reading
    /// top-to-bottom as a sequence of named assembly steps.
    #[allow(clippy::too_many_lines)]
    pub async fn build(self) -> Result<BreezSdk, SdkError> {
        self.config.validate()?;
        let runtime = runtime_from_config(&self.config);
        let background_services_enabled = runtime.starts_background_services();
        validate_server_mode(&self.config, background_services_enabled)?;

        let signers = build_signers(&self.config, self.signer_source)?;
        validate_signer_capabilities(&self.config, signers.ecies.is_some())?;

        let context = resolve_context(self.context, &self.config).await?;
        let stores = resolve_storage(self.storage, &context, &signers.spark, &self.config).await?;
        let chain_service = resolve_chain_service(
            self.chain_service,
            self.rest_chain_service_config,
            &context,
            self.config.network,
        );

        let user_agent = crate::default_user_agent();
        info!("Building sdk with user agent: {}", user_agent);

        let fiat_service: Arc<dyn breez_sdk_common::fiat::FiatService> = match self.fiat_service {
            Some(service) => Arc::new(FiatServiceWrapper::new(service)),
            None => context.breez_server.clone(),
        };
        let lnurl_client: Arc<dyn platform_utils::HttpClient> = self
            .lnurl_client
            .unwrap_or_else(|| context.http_client.clone());

        let spark_wallet_config =
            finalize_spark_wallet_config(&self.config, &user_agent, background_services_enabled)?;
        let shutdown_sender = watch::channel::<()>(()).0;
        let session_store = wrap_session_store(
            stores.session_store.clone(),
            signers.ecies.clone(),
            self.config.network,
        )?;

        let spark_wallet = build_spark_wallet(BuildSparkWalletParams {
            config: spark_wallet_config,
            spark_signer: Arc::clone(&signers.spark),
            session_store,
            shutdown_receiver: background_services_enabled.then(|| shutdown_sender.subscribe()),
            tree_store: stores.tree_store.clone(),
            token_output_store: stores.token_output_store.clone(),
            payment_observer: self.payment_observer,
            context: Arc::clone(&context),
        })
        .await?;

        let lnurl_server_client = resolve_lnurl_server_client(
            self.lnurl_server_client,
            &self.config,
            &context,
            &spark_wallet,
        );

        let real_time_sync_active =
            background_services_enabled && self.config.real_time_sync_server_url.is_some();
        let event_emitter = Arc::new(EventEmitter::new(real_time_sync_active));

        let storage = maybe_wrap_storage_with_real_time_sync(
            Arc::clone(&stores.storage),
            &self.config,
            background_services_enabled,
            user_agent,
            signers.rtsync,
            shutdown_sender.subscribe(),
            Arc::clone(&event_emitter),
            lnurl_server_client.clone(),
        )
        .await?;

        let buy_bitcoin_provider = Arc::new(MoonpayProvider::new(context.breez_server.clone()));
        let token_converter =
            build_token_converter(&self.config, &storage, &spark_wallet, &context);

        let sync_coordinator = SyncCoordinator::new();

        // Shared lightning-send helper used by `send_bolt11_invoice` and
        // by cross-chain providers that pay LN invoices (currently: Boltz
        // reverse swap).
        let lightning_sender = Arc::new(crate::sdk::LightningSender::new(
            Arc::clone(&spark_wallet),
            Arc::clone(&storage),
            Arc::clone(&event_emitter),
            shutdown_sender.clone(),
        ));

        let cross_chain_context = build_cross_chain_context(
            &self.config,
            &context.breez_server,
            &spark_wallet,
            &storage,
            signers.ecies.clone(),
            &lightning_sender,
            Arc::clone(&fiat_service),
            shutdown_sender.subscribe(),
        )
        .await;

        let stable_balance = build_stable_balance(
            &self.config,
            &token_converter,
            &spark_wallet,
            &storage,
            &event_emitter,
        )
        .await;

        // Register TokenConversionMiddleware to suppress conversion child events
        // before they reach external listeners (after StableBalance middleware).
        event_emitter
            .add_middleware(Box::new(TokenConversionMiddleware))
            .await;

        let sdk = BreezSdk::init_and_start(BreezSdkParams {
            config: self.config,
            storage,
            chain_service,
            fiat_service,
            lnurl_client,
            lnurl_server_client,
            lnurl_auth_signer: signers.lnurl_auth,
            shutdown_sender,
            runtime,
            spark_wallet,
            event_emitter,
            buy_bitcoin_provider,
            token_converter,
            stable_balance,
            sync_coordinator,
            cross_chain_context,
            lightning_sender,
        })
        .await?;
        debug!("Initialized and started breez sdk.");

        Ok(sdk)
    }
}

/// Rejects server-mode configs that depend on background services.
fn validate_server_mode(
    config: &Config,
    background_services_enabled: bool,
) -> Result<(), SdkError> {
    if background_services_enabled {
        return Ok(());
    }
    if config.stable_balance_config.is_some() {
        return Err(SdkError::InvalidInput(
            "stable_balance_config is not supported when background_tasks_enabled is false"
                .to_string(),
        ));
    }
    if config.real_time_sync_server_url.is_some() {
        return Err(SdkError::InvalidInput(
            "real_time_sync_server_url must be None when background_tasks_enabled is false"
                .to_string(),
        ));
    }
    if config.leaf_optimization_config.auto_enabled {
        return Err(SdkError::InvalidInput(
            "leaf_optimization_config.auto_enabled must be false when background_tasks_enabled is false"
                .to_string(),
        ));
    }
    if config.token_optimization_config.auto_enabled {
        return Err(SdkError::InvalidInput(
            "token_optimization_config.auto_enabled must be false when background_tasks_enabled is false"
                .to_string(),
        ));
    }
    if config.cross_chain_config.is_some() {
        return Err(SdkError::InvalidInput(
            "Cross-chain config must be unset when background tasks are disabled".to_string(),
        ));
    }
    Ok(())
}

/// Rejects configs whose features need local encryption when the signer can't
/// perform ECIES.
fn validate_signer_capabilities(config: &Config, has_ecies: bool) -> Result<(), SdkError> {
    if has_ecies {
        return Ok(());
    }
    if config.real_time_sync_server_url.is_some() {
        return Err(SdkError::InvalidInput(
            "Real-time sync requires a signer that supports ECIES".to_string(),
        ));
    }
    if config.cross_chain_config.is_some() {
        return Err(SdkError::InvalidInput(
            "Cross-chain payments require a signer that supports ECIES".to_string(),
        ));
    }
    Ok(())
}

/// Derives the SDK-layer signers from one signer source: the Spark signer, and
/// (when the signer can perform ECIES/HMAC) the `ecies` signer plus the
/// real-time-sync and lnurl-auth signers. A signing-only external signer can do
/// neither, so `ecies`, `rtsync`, and `lnurl_auth` are all left `None`.
fn build_signers(config: &Config, signer_source: SignerSource) -> Result<Signers, SdkError> {
    use crate::signer::{
        BreezSigner, EciesSigner, ExternalBreezSignerAdapter, ExternalSigningSignerAdapter,
        ExternalSparkSignerAdapter, HmacSigner,
    };

    // The SDK-layer `BreezSigner` (`base`) roots at the identity master
    // (`base/0'`); the high-level Spark signer (the in-process `DefaultSigner`
    // wrapped in a `SparkSignerAdapter`) roots at the account master (`base`).
    #[allow(clippy::type_complexity)]
    let (base, ecies, hmac, spark): (
        Arc<dyn BreezSigner>,
        Option<Arc<dyn EciesSigner>>,
        Option<Arc<dyn HmacSigner>>,
        Arc<dyn SparkSigner>,
    ) = match signer_source {
        SignerSource::Seed {
            seed,
            account_number,
        } => {
            let seed_bytes = seed.to_bytes()?;
            let network = config.network.into();
            // One `BreezSignerImpl` implements all three capability traits.
            let signer = Arc::new(BreezSignerImpl::new(
                spark_wallet::identity_master_key(&seed_bytes, network, account_number)
                    .map_err(|e| SdkError::Generic(e.to_string()))?,
            ));
            let spark: Arc<dyn SparkSigner> = Arc::new(spark_wallet::SparkSignerAdapter::new(
                Arc::new(spark_wallet::DefaultSigner::from_master(
                    spark_wallet::account_master_key(&seed_bytes, network, account_number)
                        .map_err(|e| SdkError::Generic(e.to_string()))?,
                )),
            ));
            (signer.clone(), Some(signer.clone()), Some(signer), spark)
        }
        SignerSource::External { breez, spark } => {
            let spark: Arc<dyn SparkSigner> = Arc::new(ExternalSparkSignerAdapter::new(spark));
            match breez {
                ExternalBreez::Full(breez) => {
                    // The full adapter implements all three capability traits.
                    let signer = Arc::new(ExternalBreezSignerAdapter::new(breez));
                    (signer.clone(), Some(signer.clone()), Some(signer), spark)
                }
                ExternalBreez::SigningOnly(breez) => {
                    let base: Arc<dyn BreezSigner> =
                        Arc::new(ExternalSigningSignerAdapter::new(breez));
                    (base, None, None, spark)
                }
            }
        }
    };

    let rtsync = ecies
        .as_ref()
        .map(|ecies| {
            RTSyncSigner::new(base.clone(), ecies.clone(), config.network)
                .map(Arc::new)
                .map_err(|e| SdkError::Generic(e.to_string()))
        })
        .transpose()?;
    let lnurl_auth = hmac
        .as_ref()
        .map(|hmac| Arc::new(LnurlAuthSignerAdapter::new(base.clone(), hmac.clone())));

    Ok(Signers {
        ecies,
        spark,
        rtsync,
        lnurl_auth,
    })
}

/// Resolves the [`SdkContext`] — either the caller-supplied one or a fresh
/// default — and validates that its `network`/`api_key` match the SDK config.
async fn resolve_context(
    supplied: Option<Arc<SdkContext>>,
    config: &Config,
) -> Result<Arc<SdkContext>, SdkError> {
    let context = match supplied {
        Some(ctx) => ctx,
        None => {
            new_shared_sdk_context(SdkContextConfig {
                api_key: config.api_key.clone(),
                ..SdkContextConfig::new(config.network)
            })
            .await?
        }
    };
    if context.network != config.network || context.api_key != config.api_key {
        return Err(SdkError::Generic(
            "SdkContext network/api_key do not match SdkConfig".to_string(),
        ));
    }
    Ok(context)
}

/// Resolves the single [`StorageBackend`] — from the builder or the shared
/// context, never both — and asks it for the per-tenant store set.
async fn resolve_storage(
    supplied: Option<Arc<dyn StorageBackend>>,
    context: &SdkContext,
    spark_signer: &Arc<dyn SparkSigner>,
    config: &Config,
) -> Result<Arc<ResolvedStores>, SdkError> {
    let storage_backend: Arc<dyn StorageBackend> = match (supplied, context.storage_backend.clone())
    {
        (Some(storage), None) => storage,
        (None, Some(backend)) => backend,
        (Some(_), Some(_)) => {
            return Err(SdkError::Generic(
                "storage is configured on both the SdkBuilder and the shared SdkContext"
                    .to_string(),
            ));
        }
        (None, None) => return Err(SdkError::Generic("No storage configured".to_string())),
    };
    let identity_public_key = spark_signer
        .get_identity_public_key()
        .await
        .map_err(|e| SdkError::Generic(e.to_string()))?;
    storage_backend
        .create_stores(config.network, identity_public_key.serialize().to_vec())
        .await
}

/// Resolves the chain service: caller-supplied override → REST config → network
/// default (Esplora on mainnet, mempool.space on regtest).
fn resolve_chain_service(
    supplied: Option<Arc<dyn BitcoinChainService>>,
    rest_config: Option<RestChainServiceConfig>,
    context: &SdkContext,
    network: Network,
) -> Arc<dyn BitcoinChainService> {
    if let Some(service) = supplied {
        return service;
    }
    if let Some(cfg) = rest_config {
        return Arc::new(RestClientChainService::new(
            cfg.url,
            network,
            5,
            context.http_client.clone(),
            cfg.credentials
                .map(|c| BasicAuth::new(c.username, c.password)),
            cfg.api_type,
        ));
    }
    let inner_client: Arc<dyn platform_utils::HttpClient> = context.http_client.clone();
    match network {
        Network::Mainnet => Arc::new(RestClientChainService::new(
            "https://blockstream.info/api".to_string(),
            network,
            5,
            inner_client,
            None,
            ChainApiType::Esplora,
        )),
        Network::Regtest => Arc::new(RestClientChainService::new(
            "https://regtest-mempool.us-west-2.sparkinfra.net/api".to_string(),
            network,
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
}

/// Builds the full [`SparkWalletConfig`] with user-agent and SDK-level
/// optimization overrides applied. `background_services_enabled` gates the
/// auto-optimization flags so server-mode SDKs don't run background loops.
fn finalize_spark_wallet_config(
    config: &Config,
    user_agent: &str,
    background_services_enabled: bool,
) -> Result<SparkWalletConfig, SdkError> {
    let mut spark_wallet_config = if let Some(env_config) = &config.spark_config {
        SdkBuilder::build_spark_wallet_config(config.network.into(), env_config)?
    } else {
        SparkWalletConfig::default_config(config.network.into())
    };
    spark_wallet_config.operator_pool = spark_wallet_config
        .operator_pool
        .with_user_agent(Some(user_agent.to_string()));
    spark_wallet_config.service_provider_config.user_agent = Some(user_agent.to_string());
    spark_wallet_config.leaf_auto_optimize_enabled =
        background_services_enabled && config.leaf_optimization_config.auto_enabled;
    spark_wallet_config.leaf_optimization_options.multiplicity =
        config.leaf_optimization_config.multiplicity;

    let token_opt = &config.token_optimization_config;
    let token_options = &mut spark_wallet_config.token_outputs_optimization_options;
    token_options.target_output_count = token_opt.target_output_count;
    token_options.min_outputs_threshold = token_opt.min_outputs_threshold;
    // Only override when disabled; enabled keeps the network default interval.
    if !token_opt.auto_enabled || !background_services_enabled {
        token_options.auto_optimize_interval = None;
    }
    spark_wallet_config.max_concurrent_claims = config.max_concurrent_claims;
    Ok(spark_wallet_config)
}

/// Wraps the resolved session store (or an in-memory default) in the encrypting
/// and caching layers.
///
/// With an `ecies` signer the tokens are encrypted at rest, otherwise (a
/// signing-only signer) they are stored in plaintext. Either way each token is
/// tagged with its mode, so switching the signer's ECIES capability between runs
/// is caught on read and forces a fresh re-authentication.
fn wrap_session_store(
    session_store: Option<Arc<dyn SessionStore>>,
    ecies: Option<Arc<dyn crate::signer::EciesSigner>>,
    network: Network,
) -> Result<Arc<dyn SessionStore>, SdkError> {
    let inner = session_store.unwrap_or_else(|| Arc::new(InMemorySessionStore::default()));
    let store = crate::session_store::EncryptingSessionStore::new(inner, ecies, network)
        .map_err(|e| SdkError::Generic(format!("failed to set up the session store: {e}")))?;
    Ok(Arc::new(crate::session_store::CachingSessionStore::new(
        Arc::new(store),
    )))
}

/// Builds the [`SparkWallet`] from the assembled config, signers and stores.
async fn build_spark_wallet(params: BuildSparkWalletParams) -> Result<Arc<SparkWallet>, SdkError> {
    let mut wallet_builder = spark_wallet::WalletBuilder::new(params.config, params.spark_signer)
        .with_session_store(params.session_store);
    if let Some(receiver) = params.shutdown_receiver {
        wallet_builder = wallet_builder.with_cancellation_token(receiver);
    }
    if let Some(provider) = &params.context.jwt_header_provider {
        wallet_builder = wallet_builder.with_so_extra_header_provider(
            Arc::clone(provider) as Arc<dyn spark_wallet::HeaderProvider>
        );
    }
    if let Some(observer) = params.payment_observer {
        let observer: Arc<dyn spark_wallet::TransferObserver> =
            Arc::new(SparkTransferObserver::new(observer));
        wallet_builder = wallet_builder.with_transfer_observer(observer);
    }
    if let Some(tree_store) = params.tree_store {
        wallet_builder = wallet_builder.with_tree_store(tree_store);
    }
    if let Some(token_output_store) = params.token_output_store {
        wallet_builder = wallet_builder.with_token_output_store(token_output_store);
    }
    wallet_builder = wallet_builder.with_ssp_http_client(params.context.http_client.clone());
    wallet_builder =
        wallet_builder.with_connection_manager(params.context.connection_manager.clone());
    Ok(Arc::new(wallet_builder.build().await?))
}

/// Resolves the LNURL server client: explicit override → built from
/// `config.lnurl_domain` → none.
fn resolve_lnurl_server_client(
    explicit: Option<Arc<dyn LnurlServerClient>>,
    config: &Config,
    context: &SdkContext,
    spark_wallet: &Arc<SparkWallet>,
) -> Option<Arc<dyn LnurlServerClient>> {
    if let Some(client) = explicit {
        return Some(client);
    }
    config.lnurl_domain.as_ref().map(|domain| {
        Arc::new(DefaultLnurlServerClient::new(
            context.http_client.clone(),
            domain.clone(),
            config.api_key.clone(),
            Arc::clone(spark_wallet),
        )) as Arc<dyn LnurlServerClient>
    })
}

/// Wraps the base storage with the real-time-sync layer when configured and
/// background services are enabled. Otherwise returns the storage unchanged.
#[allow(clippy::too_many_arguments)]
async fn maybe_wrap_storage_with_real_time_sync(
    storage: Arc<dyn crate::persist::Storage>,
    config: &Config,
    background_services_enabled: bool,
    user_agent: String,
    rtsync_signer: Option<Arc<RTSyncSigner>>,
    shutdown_receiver: watch::Receiver<()>,
    event_emitter: Arc<EventEmitter>,
    lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
) -> Result<Arc<dyn crate::persist::Storage>, SdkError> {
    // `validate_signer_capabilities` rejects real-time sync without an
    // ECIES-capable signer, so `rtsync_signer` is present whenever the URL is
    // set; a missing signer can't be reached and falls through to the no-op arm.
    match (&config.real_time_sync_server_url, rtsync_signer) {
        (Some(server_url), Some(signer)) if background_services_enabled => {
            init_and_start_real_time_sync(RealTimeSyncParams {
                server_url: server_url.clone(),
                api_key: config.api_key.clone(),
                user_agent,
                signer,
                storage,
                shutdown_receiver,
                event_emitter,
                lnurl_server_client,
            })
            .await
        }
        _ => Ok(storage),
    }
}

/// Builds the [`FlashnetTokenConverter`] used for in-SDK token conversion.
fn build_token_converter(
    config: &Config,
    storage: &Arc<dyn crate::persist::Storage>,
    spark_wallet: &Arc<SparkWallet>,
    context: &SdkContext,
) -> Arc<dyn TokenConverter> {
    let flashnet_config = FlashnetConfig::default_config(
        config.network.into(),
        DEFAULT_INTEGRATOR_PUBKEY
            .parse()
            .ok()
            .map(|pubkey| IntegratorConfig {
                pubkey,
                fee_bps: DEFAULT_INTEGRATOR_FEE_BPS,
            }),
    );
    Arc::new(FlashnetTokenConverter::new(
        flashnet_config,
        Arc::clone(storage),
        Arc::clone(spark_wallet),
        config.network,
        context.http_client.clone(),
    ))
}

/// Builds the optional [`StableBalance`] middleware, which must be registered
/// before [`TokenConversionMiddleware`] so it can see conversion child events.
async fn build_stable_balance(
    config: &Config,
    token_converter: &Arc<dyn TokenConverter>,
    spark_wallet: &Arc<SparkWallet>,
    storage: &Arc<dyn crate::persist::Storage>,
    event_emitter: &Arc<EventEmitter>,
) -> Option<Arc<StableBalance>> {
    let stable_config = config.stable_balance_config.as_ref()?;
    Some(Arc::new(
        StableBalance::new(
            stable_config.clone(),
            Arc::clone(token_converter),
            Arc::clone(spark_wallet),
            Arc::clone(storage),
            Arc::clone(event_emitter),
        )
        .await,
    ))
}

/// Builds the cross-chain context: provider registry + shared cached fiat
/// service. Returns an empty registry when `config.cross_chain_config` is unset.
#[allow(clippy::too_many_arguments)]
async fn build_cross_chain_context(
    config: &Config,
    breez_server: &Arc<BreezServer>,
    spark_wallet: &Arc<SparkWallet>,
    storage: &Arc<dyn crate::persist::Storage>,
    ecies: Option<Arc<dyn crate::signer::EciesSigner>>,
    lightning_sender: &Arc<crate::sdk::LightningSender>,
    fiat_service: Arc<dyn breez_sdk_common::fiat::FiatService>,
    shutdown_receiver: watch::Receiver<()>,
) -> crate::cross_chain::CrossChainContext {
    // Cache scoped to cross-chain: providers + dispatcher share one TTL window.
    let cached_fiat: Arc<dyn breez_sdk_common::fiat::FiatService> =
        Arc::new(crate::cross_chain::CachedFiatService::new(
            fiat_service,
            crate::cross_chain::DEFAULT_FIAT_CACHE_TTL,
        ));
    let mut providers = crate::cross_chain::CrossChainContext::new(Arc::clone(&cached_fiat));
    if config.cross_chain_config.is_none() {
        return providers;
    }

    // Orchestra cross-chain is mainnet-only. Its base URL and API key are
    // fetched from Breez server (so the key can be rotated and revoked without
    // an SDK release) lazily on first cross-chain use, not at connect: see
    // `BreezServerOrchestraConfigResolver`. There is no bundled fallback key.
    if matches!(config.network, Network::Mainnet) {
        let config_resolver = Arc::new(
            crate::cross_chain::BreezServerOrchestraConfigResolver::new(Arc::clone(breez_server)),
        );
        providers.insert(
            crate::cross_chain::CrossChainProvider::Orchestra,
            Arc::new(crate::cross_chain::OrchestraService::new(
                config_resolver,
                Arc::clone(spark_wallet),
                Arc::clone(storage),
                Arc::clone(&cached_fiat),
                shutdown_receiver,
            )),
        );
    }

    match crate::cross_chain::BoltzService::build(
        config.network,
        Arc::clone(spark_wallet),
        Arc::clone(storage),
        ecies,
        cached_fiat,
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
    use crate::{Network, SdkError, default_config};

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

    #[test]
    fn validate_signer_capabilities_gates_encryption_features() {
        // ECIES/HMAC supported: encryption-dependent features are allowed.
        let mut config = default_config(Network::Regtest);
        config.real_time_sync_server_url = Some("https://example.com".to_string());
        assert!(super::validate_signer_capabilities(&config, true).is_ok());

        // No ECIES/HMAC + real-time sync: rejected with a clear error.
        let mut config = default_config(Network::Regtest);
        config.real_time_sync_server_url = Some("https://example.com".to_string());
        config.cross_chain_config = None;
        match super::validate_signer_capabilities(&config, false) {
            Err(SdkError::InvalidInput(m)) => {
                assert!(m.contains("Real-time sync"), "got: {m}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }

        // No ECIES/HMAC + no encryption-dependent feature: allowed, so a
        // payments-only wallet still builds.
        let mut config = default_config(Network::Regtest);
        config.real_time_sync_server_url = None;
        config.cross_chain_config = None;
        assert!(super::validate_signer_capabilities(&config, false).is_ok());
    }

    /// A signing-only signer yields no ECIES/HMAC, so `build_signers` leaves the
    /// dependent signers (real-time sync, lnurl-auth) unbuilt; a full/seed signer
    /// builds them.
    #[test]
    fn build_signers_signing_only_omits_ecies_dependent_signers() {
        use crate::error::SignerError;
        use crate::signer::external_types::{
            EcdsaSignatureBytes, MessageBytes, PublicKeyBytes, RecoverableEcdsaSignatureBytes,
            SchnorrSignatureBytes,
        };
        use crate::signer::{
            DefaultExternalSparkSigner, ExternalSigningSigner, ExternalSparkSigner,
        };
        use std::sync::Arc;

        // A signing-only signer; `build_signers` only wraps it, never calls it.
        struct StubSigningSigner;
        #[macros::async_trait]
        impl ExternalSigningSigner for StubSigningSigner {
            async fn derive_public_key(&self, _p: String) -> Result<PublicKeyBytes, SignerError> {
                unreachable!("build_signers must not call the signer")
            }
            async fn sign_ecdsa(
                &self,
                _m: MessageBytes,
                _p: String,
            ) -> Result<EcdsaSignatureBytes, SignerError> {
                unreachable!("build_signers must not call the signer")
            }
            async fn sign_ecdsa_recoverable(
                &self,
                _m: MessageBytes,
                _p: String,
            ) -> Result<RecoverableEcdsaSignatureBytes, SignerError> {
                unreachable!("build_signers must not call the signer")
            }
            async fn sign_hash_schnorr(
                &self,
                _h: Vec<u8>,
                _p: String,
            ) -> Result<SchnorrSignatureBytes, SignerError> {
                unreachable!("build_signers must not call the signer")
            }
        }

        let config = default_config(Network::Regtest);

        // Seed source: full capability, so rtsync + lnurl-auth signers are built.
        let full = super::build_signers(
            &config,
            super::SignerSource::Seed {
                seed: test_seed(),
                account_number: None,
            },
        )
        .unwrap();
        assert!(full.ecies.is_some());
        assert!(full.rtsync.is_some());
        assert!(full.lnurl_auth.is_some());

        // Signing-only external source: no ECIES/HMAC, so both are left unbuilt.
        let breez: Arc<dyn ExternalSigningSigner> = Arc::new(StubSigningSigner);
        let spark: Arc<dyn ExternalSparkSigner> = Arc::new(
            DefaultExternalSparkSigner::new(
                TEST_MNEMONIC.to_string(),
                None,
                Network::Regtest,
                None,
            )
            .unwrap(),
        );
        let signing_only = super::build_signers(
            &config,
            super::SignerSource::External {
                breez: super::ExternalBreez::SigningOnly(breez),
                spark,
            },
        )
        .unwrap();
        assert!(signing_only.ecies.is_none());
        assert!(signing_only.rtsync.is_none());
        assert!(signing_only.lnurl_auth.is_none());
    }

    /// End to end: an offline SDK built with a signing-only signer has no
    /// lnurl-auth signer, and `lnurl_auth` rejects with `InvalidInput`.
    #[tokio::test]
    async fn signing_only_signer_build_disables_lnurl_auth() {
        use crate::error::SignerError;
        use crate::signer::external_types::{
            EcdsaSignatureBytes, MessageBytes, PublicKeyBytes, RecoverableEcdsaSignatureBytes,
            SchnorrSignatureBytes,
        };
        use crate::signer::{
            DefaultExternalSigner, DefaultExternalSparkSigner, ExternalBreezSigner,
            ExternalSigningSigner, ExternalSparkSigner,
        };
        use std::sync::Arc;

        // A real signing-only signer: delegates its four methods to a
        // seed-derived reference signer.
        struct SigningOnly(DefaultExternalSigner);
        #[macros::async_trait]
        impl ExternalSigningSigner for SigningOnly {
            async fn derive_public_key(&self, path: String) -> Result<PublicKeyBytes, SignerError> {
                self.0.derive_public_key(path).await
            }
            async fn sign_ecdsa(
                &self,
                message: MessageBytes,
                path: String,
            ) -> Result<EcdsaSignatureBytes, SignerError> {
                self.0.sign_ecdsa(message, path).await
            }
            async fn sign_ecdsa_recoverable(
                &self,
                message: MessageBytes,
                path: String,
            ) -> Result<RecoverableEcdsaSignatureBytes, SignerError> {
                self.0.sign_ecdsa_recoverable(message, path).await
            }
            async fn sign_hash_schnorr(
                &self,
                hash: Vec<u8>,
                path: String,
            ) -> Result<SchnorrSignatureBytes, SignerError> {
                self.0.sign_hash_schnorr(hash, path).await
            }
        }

        let mut config = default_config(Network::Regtest);
        // Keep the build offline: no real-time sync, no network private-mode init.
        config.real_time_sync_server_url = None;
        config.private_enabled_default = false;

        let breez: Arc<dyn ExternalSigningSigner> = Arc::new(SigningOnly(
            DefaultExternalSigner::new(TEST_MNEMONIC.to_string(), None, Network::Regtest, None)
                .unwrap(),
        ));
        let spark: Arc<dyn ExternalSparkSigner> = Arc::new(
            DefaultExternalSparkSigner::new(
                TEST_MNEMONIC.to_string(),
                None,
                Network::Regtest,
                None,
            )
            .unwrap(),
        );

        let sdk = SdkBuilder::new_with_signing_only_signer(config, breez, spark)
            .with_default_storage(unique_storage_dir("signing-only-lnurl"))
            .build()
            .await
            .expect("signing-only build should succeed");

        assert!(
            sdk.lnurl_auth_signer.is_none(),
            "a signing-only signer must not build an lnurl-auth signer"
        );

        let err = sdk
            .lnurl_auth(crate::LnurlAuthRequestDetails {
                k1: "00".repeat(32),
                action: None,
                domain: "example.com".to_string(),
                url: "https://example.com/lnurl-auth".to_string(),
            })
            .await
            .expect_err("lnurl_auth must fail for a signing-only signer");
        assert!(
            matches!(err, SdkError::Generic(_)),
            "expected Generic, got {err:?}"
        );

        sdk.disconnect().await.expect("disconnect should succeed");
    }

    /// A signing-only signer is rejected at build when a feature that needs ECIES
    /// is configured. Here real-time sync is set, so `build()` returns
    /// `InvalidInput` before any signer method runs.
    #[tokio::test]
    async fn signing_only_signer_build_rejects_ecies_features() {
        use crate::error::SignerError;
        use crate::signer::external_types::{
            EcdsaSignatureBytes, MessageBytes, PublicKeyBytes, RecoverableEcdsaSignatureBytes,
            SchnorrSignatureBytes,
        };
        use crate::signer::{
            DefaultExternalSparkSigner, ExternalSigningSigner, ExternalSparkSigner,
        };
        use std::sync::Arc;

        // Never called: the build fails at capability validation before signing.
        struct StubSigningSigner;
        #[macros::async_trait]
        impl ExternalSigningSigner for StubSigningSigner {
            async fn derive_public_key(&self, _p: String) -> Result<PublicKeyBytes, SignerError> {
                unreachable!("build must fail before calling the signer")
            }
            async fn sign_ecdsa(
                &self,
                _m: MessageBytes,
                _p: String,
            ) -> Result<EcdsaSignatureBytes, SignerError> {
                unreachable!("build must fail before calling the signer")
            }
            async fn sign_ecdsa_recoverable(
                &self,
                _m: MessageBytes,
                _p: String,
            ) -> Result<RecoverableEcdsaSignatureBytes, SignerError> {
                unreachable!("build must fail before calling the signer")
            }
            async fn sign_hash_schnorr(
                &self,
                _h: Vec<u8>,
                _p: String,
            ) -> Result<SchnorrSignatureBytes, SignerError> {
                unreachable!("build must fail before calling the signer")
            }
        }

        let mut config = default_config(Network::Regtest);
        config.real_time_sync_server_url = Some("https://example.com".to_string());

        let breez: Arc<dyn ExternalSigningSigner> = Arc::new(StubSigningSigner);
        let spark: Arc<dyn ExternalSparkSigner> = Arc::new(
            DefaultExternalSparkSigner::new(
                TEST_MNEMONIC.to_string(),
                None,
                Network::Regtest,
                None,
            )
            .unwrap(),
        );

        let result = SdkBuilder::new_with_signing_only_signer(config, breez, spark)
            .with_default_storage(unique_storage_dir("signing-only-reject-rtsync"))
            .build()
            .await;
        match result {
            Err(SdkError::InvalidInput(_)) => {}
            Ok(_) => panic!("build must reject real-time sync for a signing-only signer"),
            Err(other) => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wrap_session_store_tags_tokens_by_mode() {
        use crate::Seed;
        use crate::signer::EciesSigner;
        use crate::signer::breez::BreezSignerImpl;
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        use spark_wallet::{Session, SessionStore, SessionStoreError};
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        // Inner store that exposes the raw bytes the wrapper writes through.
        #[derive(Default)]
        struct InspectableInner {
            sessions: Mutex<HashMap<PublicKey, Session>>,
        }
        #[macros::async_trait]
        impl SessionStore for InspectableInner {
            async fn get_session(&self, key: &PublicKey) -> Result<Session, SessionStoreError> {
                self.sessions
                    .lock()
                    .unwrap()
                    .get(key)
                    .cloned()
                    .ok_or(SessionStoreError::NotFound)
            }
            async fn set_session(
                &self,
                key: &PublicKey,
                session: Session,
            ) -> Result<(), SessionStoreError> {
                self.sessions.lock().unwrap().insert(*key, session);
                Ok(())
            }
        }

        fn seed_signer() -> Arc<dyn EciesSigner> {
            let seed_bytes = Seed::Entropy(vec![7u8; 32]).to_bytes().unwrap();
            let master =
                spark_wallet::identity_master_key(&seed_bytes, Network::Regtest.into(), None)
                    .unwrap();
            Arc::new(BreezSignerImpl::new(master))
        }
        fn key() -> PublicKey {
            let secp = Secp256k1::new();
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[3u8; 32]).unwrap())
        }

        let token = "bearer-token".to_string();

        // Encryption off: the inner store holds the plaintext token, tagged pln:.
        let inner = Arc::new(InspectableInner::default());
        let store = super::wrap_session_store(Some(inner.clone()), None, Network::Regtest).unwrap();
        store
            .set_session(
                &key(),
                Session {
                    token: token.clone(),
                    expiration: 1,
                },
            )
            .await
            .unwrap();
        let raw = inner.sessions.lock().unwrap().get(&key()).cloned().unwrap();
        assert_eq!(
            raw.token,
            format!("pln:{token}"),
            "plaintext mode must persist the token tagged with pln:"
        );

        // Encryption on: the inner store holds ciphertext, tagged enc:.
        let inner = Arc::new(InspectableInner::default());
        let store =
            super::wrap_session_store(Some(inner.clone()), Some(seed_signer()), Network::Regtest)
                .unwrap();
        store
            .set_session(
                &key(),
                Session {
                    token: token.clone(),
                    expiration: 1,
                },
            )
            .await
            .unwrap();
        let raw = inner.sessions.lock().unwrap().get(&key()).cloned().unwrap();
        assert!(
            raw.token.starts_with("enc:") && raw.token != token,
            "encrypt mode must persist ciphertext tagged with enc:"
        );
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

    /// Regtest + `cross_chain_config` trips the Mainnet-only gate in
    /// `Config::validate` before reaching the server-mode reject in
    /// `build`. The server-mode gate is still in place (verified by the
    /// inline check in `build`); this test pins the more specific failure.
    #[tokio::test]
    async fn build_rejects_cross_chain_config_on_regtest() {
        use crate::{CrossChainConfig, SdkError, default_config};
        let mut config = default_config(Network::Regtest);
        config.cross_chain_config = Some(CrossChainConfig::default());

        let seed = test_seed();
        let result = SdkBuilder::new(config, seed).build().await;
        match result {
            Err(SdkError::InvalidInput(m)) => {
                assert!(
                    m.contains("only available on Mainnet"),
                    "expected mainnet-only rejection, got: {m}"
                );
            }
            Err(err) => panic!("expected InvalidInput error, got {err:?}"),
            Ok(_) => panic!("expected regtest with cross_chain_config to fail"),
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

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn test_seed() -> crate::Seed {
        crate::Seed::Mnemonic {
            mnemonic: TEST_MNEMONIC.to_string(),
            passphrase: None,
        }
    }

    fn unique_storage_dir(name: &str) -> String {
        std::env::temp_dir()
            .join(format!("breez-sdk-test-{}-{}", name, uuid::Uuid::new_v4()))
            .to_string_lossy()
            .into_owned()
    }

    /// Waits for the SDK object graph to be released. Background tasks drop
    /// their `BreezSdk` clones asynchronously after the shutdown signal, so
    /// poll briefly instead of asserting immediately after `drop`.
    async fn assert_sdk_graph_freed(
        event_emitter: std::sync::Weak<crate::EventEmitter>,
        spark_wallet: std::sync::Weak<spark_wallet::SparkWallet>,
    ) {
        for _ in 0..100 {
            if event_emitter.upgrade().is_none() && spark_wallet.upgrade().is_none() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!(
            "SDK object graph leaked after drop (EventEmitter alive: {}, SparkWallet alive: {})",
            event_emitter.upgrade().is_some(),
            spark_wallet.upgrade().is_some(),
        );
    }

    /// Regression test for <https://github.com/breez/spark-sdk/issues/947>:
    /// each server-mode build → disconnect → drop lifecycle must release the
    /// whole per-instance object graph.
    #[tokio::test]
    async fn server_mode_sdk_graph_is_freed_on_drop() {
        use crate::default_server_config;

        let config = default_server_config(Network::Regtest);
        let sdk = SdkBuilder::new(config, test_seed())
            .with_default_storage(unique_storage_dir("leak-server"))
            .build()
            .await
            .expect("server-mode build should succeed");

        let event_emitter = std::sync::Arc::downgrade(&sdk.event_emitter);
        let spark_wallet = std::sync::Arc::downgrade(&sdk.spark_wallet);

        sdk.disconnect().await.expect("disconnect should succeed");
        drop(sdk);

        assert_sdk_graph_freed(event_emitter, spark_wallet).await;
    }

    /// Network-dependent variant of `server_mode_sdk_graph_is_freed_on_drop`
    /// that runs a real `sync_wallet` against the regtest operators before
    /// dropping, proving the sync path retains nothing either.
    #[tokio::test]
    #[ignore = "requires network access to the regtest operators"]
    async fn server_mode_sdk_graph_is_freed_on_drop_after_sync() {
        use crate::{SyncWalletRequest, default_server_config};

        let config = default_server_config(Network::Regtest);
        let sdk = SdkBuilder::new(config, test_seed())
            .with_default_storage(unique_storage_dir("leak-server-sync"))
            .build()
            .await
            .expect("server-mode build should succeed");

        let event_emitter = std::sync::Arc::downgrade(&sdk.event_emitter);
        let spark_wallet = std::sync::Arc::downgrade(&sdk.spark_wallet);

        sdk.sync_wallet(SyncWalletRequest {})
            .await
            .expect("sync_wallet should succeed");

        sdk.disconnect().await.expect("disconnect should succeed");
        drop(sdk);

        assert_sdk_graph_freed(event_emitter, spark_wallet).await;
    }

    /// Client-mode counterpart of the issue #947 regression test: after
    /// `disconnect`, dropping the SDK must release the whole object graph.
    #[tokio::test]
    async fn client_mode_sdk_graph_is_freed_after_disconnect_and_drop() {
        let mut config = default_config(Network::Regtest);
        // Keep the build offline: no real-time sync connection and no
        // network-backed private-mode setup on startup.
        config.real_time_sync_server_url = None;
        config.private_enabled_default = false;

        let sdk = SdkBuilder::new(config, test_seed())
            .with_default_storage(unique_storage_dir("leak-client"))
            .build()
            .await
            .expect("client-mode build should succeed");

        let event_emitter = std::sync::Arc::downgrade(&sdk.event_emitter);
        let spark_wallet = std::sync::Arc::downgrade(&sdk.spark_wallet);

        tokio::time::timeout(std::time::Duration::from_secs(30), sdk.disconnect())
            .await
            .expect("disconnect should not hang")
            .expect("disconnect should succeed");
        drop(sdk);

        assert_sdk_graph_freed(event_emitter, spark_wallet).await;
    }

    /// Variant of the client-mode leak regression test with real-time sync
    /// and stable balance enabled: the sync record handler and the conversion
    /// worker hold the emitter strongly, so their tasks must release it on
    /// `disconnect`.
    #[tokio::test]
    async fn client_mode_sdk_graph_is_freed_with_real_time_sync_enabled() {
        use crate::{StableBalanceConfig, StableBalanceToken};

        let mut config = default_config(Network::Regtest);
        // Unreachable sync server: the gRPC client connects lazily, so the
        // build succeeds offline and the sync tasks just retry in the
        // background until shutdown.
        config.real_time_sync_server_url = Some("http://127.0.0.1:9".to_string());
        config.private_enabled_default = false;
        // No active token, so the conversion worker spawns without making
        // network calls.
        config.stable_balance_config = Some(StableBalanceConfig {
            tokens: vec![StableBalanceToken {
                label: "USDB".to_string(),
                token_identifier: "btkn1test".to_string(),
            }],
            default_active_label: None,
            threshold_sats: None,
            max_slippage_bps: None,
        });

        let sdk = SdkBuilder::new(config, test_seed())
            .with_default_storage(unique_storage_dir("leak-client-rtsync"))
            .build()
            .await
            .expect("client-mode build with real-time sync should succeed");

        let event_emitter = std::sync::Arc::downgrade(&sdk.event_emitter);
        let spark_wallet = std::sync::Arc::downgrade(&sdk.spark_wallet);

        tokio::time::timeout(std::time::Duration::from_secs(30), sdk.disconnect())
            .await
            .expect("disconnect should not hang")
            .expect("disconnect should succeed");
        drop(sdk);

        assert_sdk_graph_freed(event_emitter, spark_wallet).await;
    }

    /// `disconnect` must unregister event listeners, so a listener that
    /// references the SDK instance cannot keep it alive afterwards.
    #[tokio::test]
    async fn disconnect_unregisters_event_listeners() {
        use crate::{SdkEvent, default_server_config, events::EventListener};

        struct NoopListener;

        #[macros::async_trait]
        impl EventListener for NoopListener {
            async fn on_event(&self, _event: SdkEvent) {}
        }

        let storage_dir = std::env::temp_dir()
            .join(format!(
                "breez-sdk-test-listener-clear-{}",
                uuid::Uuid::new_v4()
            ))
            .to_string_lossy()
            .into_owned();
        let sdk = SdkBuilder::new(default_server_config(Network::Regtest), test_seed())
            .with_default_storage(storage_dir)
            .build()
            .await
            .expect("server-mode build should succeed");

        let id = sdk.add_event_listener(Box::new(NoopListener)).await;

        sdk.disconnect().await.expect("disconnect should succeed");

        assert!(
            !sdk.remove_event_listener(&id).await,
            "listener should already be unregistered by disconnect"
        );
    }

    fn test_spark_signer() -> std::sync::Arc<dyn spark_wallet::SparkSigner> {
        use std::sync::Arc;

        let seed = test_seed();
        let seed_bytes = seed.to_bytes().unwrap();
        let master =
            spark_wallet::account_master_key(&seed_bytes, Network::Regtest.into(), None).unwrap();
        Arc::new(spark_wallet::SparkSignerAdapter::new(Arc::new(
            spark_wallet::DefaultSigner::from_master(master),
        )))
    }

    // ---- validate_server_mode ----

    #[test]
    fn validate_server_mode_ok_when_background_enabled() {
        use crate::{StableBalanceConfig, StableBalanceToken, default_server_config};
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
        config.real_time_sync_server_url = Some("https://example.com".to_string());
        config.leaf_optimization_config.auto_enabled = true;
        config.token_optimization_config.auto_enabled = true;
        // background_services_enabled = true → none of the gates fire.
        assert!(super::validate_server_mode(&config, true).is_ok());
    }

    #[test]
    fn validate_server_mode_ok_in_server_mode_without_background_features() {
        use crate::default_server_config;
        let config = default_server_config(Network::Regtest);
        assert!(super::validate_server_mode(&config, false).is_ok());
    }

    #[test]
    fn validate_server_mode_rejects_stable_balance_directly() {
        use crate::{StableBalanceConfig, StableBalanceToken, default_server_config};
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
        match super::validate_server_mode(&config, false) {
            Err(SdkError::InvalidInput(m)) => assert!(m.contains("stable_balance_config")),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn validate_server_mode_rejects_real_time_sync_directly() {
        use crate::default_server_config;
        let mut config = default_server_config(Network::Regtest);
        config.real_time_sync_server_url = Some("https://example.com".to_string());
        match super::validate_server_mode(&config, false) {
            Err(SdkError::InvalidInput(m)) => assert!(m.contains("real_time_sync_server_url")),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn validate_server_mode_rejects_leaf_auto_optimize_directly() {
        use crate::default_server_config;
        let mut config = default_server_config(Network::Regtest);
        config.leaf_optimization_config.auto_enabled = true;
        match super::validate_server_mode(&config, false) {
            Err(SdkError::InvalidInput(m)) => {
                assert!(m.contains("leaf_optimization_config.auto_enabled"));
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn validate_server_mode_rejects_token_auto_optimize_directly() {
        use crate::default_server_config;
        let mut config = default_server_config(Network::Regtest);
        config.token_optimization_config.auto_enabled = true;
        match super::validate_server_mode(&config, false) {
            Err(SdkError::InvalidInput(m)) => {
                assert!(m.contains("token_optimization_config.auto_enabled"));
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn validate_server_mode_rejects_cross_chain_directly() {
        use crate::{CrossChainConfig, default_server_config};
        let mut config = default_server_config(Network::Regtest);
        config.cross_chain_config = Some(CrossChainConfig::default());
        match super::validate_server_mode(&config, false) {
            Err(SdkError::InvalidInput(m)) => assert!(m.contains("Cross-chain config")),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    // ---- finalize_spark_wallet_config ----

    #[test]
    fn finalize_spark_wallet_config_disabled_background_forces_leaf_auto_off() {
        let mut config = default_config(Network::Regtest);
        config.leaf_optimization_config.auto_enabled = true;
        let result = super::finalize_spark_wallet_config(&config, "test-agent", false).unwrap();
        assert!(!result.leaf_auto_optimize_enabled);
    }

    #[test]
    fn finalize_spark_wallet_config_disabled_background_clears_token_auto_interval() {
        let mut config = default_config(Network::Regtest);
        config.token_optimization_config.auto_enabled = true;
        let result = super::finalize_spark_wallet_config(&config, "test-agent", false).unwrap();
        assert!(
            result
                .token_outputs_optimization_options
                .auto_optimize_interval
                .is_none()
        );
    }

    #[test]
    fn finalize_spark_wallet_config_enabled_background_respects_leaf_auto_optimize() {
        let mut config = default_config(Network::Regtest);
        config.leaf_optimization_config.auto_enabled = true;
        let result = super::finalize_spark_wallet_config(&config, "test-agent", true).unwrap();
        assert!(result.leaf_auto_optimize_enabled);
    }

    #[test]
    fn finalize_spark_wallet_config_applies_user_agent() {
        let config = default_config(Network::Regtest);
        let result = super::finalize_spark_wallet_config(&config, "my-app/1.0", true).unwrap();
        assert_eq!(
            result.service_provider_config.user_agent.as_deref(),
            Some("my-app/1.0")
        );
    }

    // ---- resolve_context ----

    #[tokio::test]
    async fn resolve_context_errors_on_network_mismatch() {
        use crate::{SdkContextConfig, new_shared_sdk_context};
        let config = default_config(Network::Mainnet);
        let ctx = new_shared_sdk_context(SdkContextConfig::new(Network::Regtest))
            .await
            .expect("regtest context");
        let err = super::resolve_context(Some(ctx), &config)
            .await
            .err()
            .expect("expected mismatch error");
        assert!(
            err.to_string().contains("network/api_key do not match"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn resolve_context_errors_on_api_key_mismatch() {
        use crate::{SdkContextConfig, new_shared_sdk_context};
        let mut config = default_config(Network::Mainnet);
        config.api_key = Some("intended-key".to_string());
        let ctx = new_shared_sdk_context(SdkContextConfig {
            api_key: Some("wrong-key".to_string()),
            ..SdkContextConfig::new(Network::Mainnet)
        })
        .await
        .expect("mainnet context");
        let err = super::resolve_context(Some(ctx), &config)
            .await
            .err()
            .expect("expected mismatch error");
        assert!(
            err.to_string().contains("network/api_key do not match"),
            "unexpected error: {err}"
        );
    }

    // ---- resolve_storage ----

    #[tokio::test]
    async fn resolve_storage_errors_when_neither_supplied() {
        use crate::{SdkContextConfig, new_shared_sdk_context};
        let config = default_config(Network::Regtest);
        let ctx = new_shared_sdk_context(SdkContextConfig::new(Network::Regtest))
            .await
            .expect("regtest context");
        let signer = test_spark_signer();
        let err = super::resolve_storage(None, &ctx, &signer, &config)
            .await
            .err()
            .expect("expected no-storage error");
        assert!(
            err.to_string().contains("No storage configured"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn resolve_storage_errors_when_supplied_on_both_builder_and_context() {
        use crate::{SdkContextConfig, default_storage, new_shared_sdk_context};
        let config = default_config(Network::Regtest);
        let ctx = new_shared_sdk_context(SdkContextConfig {
            storage: Some(default_storage(
                "/tmp/breez-sdk-test-resolve-storage-ctx".to_string(),
            )),
            ..SdkContextConfig::new(Network::Regtest)
        })
        .await
        .expect("regtest context");
        let signer = test_spark_signer();
        let builder_storage =
            default_storage("/tmp/breez-sdk-test-resolve-storage-builder".to_string());
        let err = super::resolve_storage(Some(builder_storage), &ctx, &signer, &config)
            .await
            .err()
            .expect("expected duplicate-storage error");
        assert!(
            err.to_string()
                .contains("storage is configured on both the SdkBuilder and the shared SdkContext"),
            "unexpected error: {err}"
        );
    }
}
