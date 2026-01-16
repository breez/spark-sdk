#![cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    allow(clippy::arc_with_non_send_sync)
)]
use std::sync::Arc;

use breez_sdk_common::{
    breez_server::{BreezServer, PRODUCTION_BREEZSERVER_URL},
    rest::ReqwestRestClient as CommonRequestRestClient,
};

#[cfg(not(target_family = "wasm"))]
use spark_wallet::Signer;
use tokio::sync::watch;
use tracing::{debug, info};

use crate::{
    Credentials, EventEmitter, FiatService, FiatServiceWrapper, KeySetType, Network, RestClient,
    RestClientWrapper, Seed,
    chain::{
        BitcoinChainService,
        rest_client::{BasicAuth, ChainApiType, RestClientChainService},
    },
    error::SdkError,
    lnurl::{LnurlServerClient, ReqwestLnurlServerClient},
    models::Config,
    nostr::NostrClient,
    payment_observer::{PaymentObserver, SparkTransferObserver},
    persist::Storage,
    realtime_sync::{RealTimeSyncParams, init_and_start_real_time_sync},
    sdk::{BreezSdk, BreezSdkParams},
    signer::{
        breez::BreezSignerImpl, lnurl_auth::LnurlAuthSignerAdapter, nostr::NostrSigner,
        rtsync::RTSyncSigner, spark::SparkSigner,
    },
    sync_storage::SyncStorage,
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
    chain_service: Option<Arc<dyn BitcoinChainService>>,
    fiat_service: Option<Arc<dyn FiatService>>,
    lnurl_client: Option<Arc<dyn RestClient>>,
    lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    payment_observer: Option<Arc<dyn PaymentObserver>>,
    sync_storage: Option<Arc<dyn SyncStorage>>,
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
            chain_service: None,
            fiat_service: None,
            lnurl_client: None,
            lnurl_server_client: None,
            payment_observer: None,
            sync_storage: None,
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
            chain_service: None,
            fiat_service: None,
            lnurl_client: None,
            lnurl_server_client: None,
            payment_observer: None,
            sync_storage: None,
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

    #[must_use]
    /// Sets the real-time sync storage implementation to be used by the SDK.
    /// Arguments:
    /// - `storage`: The sync storage implementation to be used.
    pub fn with_real_time_sync_storage(mut self, storage: Arc<dyn SyncStorage>) -> Self {
        self.sync_storage = Some(storage);
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
            Box::new(CommonRequestRestClient::new().unwrap()),
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
    pub fn with_lnurl_client(mut self, lnurl_client: Arc<dyn RestClient>) -> Self {
        self.lnurl_client = Some(lnurl_client);
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

    /// Builds the `BreezSdk` instance with the configured components.
    #[allow(clippy::too_many_lines)]
    pub async fn build(self) -> Result<BreezSdk, SdkError> {
        // Create the base signer based on the signer source
        let (signer, account_number) = match self.signer_source {
            SignerSource::Seed {
                seed,
                key_set_type,
                use_address_index,
                account_number,
            } => {
                let breez_signer = Arc::new(
                    BreezSignerImpl::new(
                        &self.config,
                        &seed,
                        key_set_type.into(),
                        use_address_index,
                        account_number,
                    )
                    .map_err(|e| SdkError::Generic(e.to_string()))?,
                );
                (
                    breez_signer as Arc<dyn crate::signer::BreezSigner>,
                    account_number,
                )
            }
            SignerSource::External(external_signer) => {
                use crate::signer::ExternalSignerAdapter;
                let adapter = Arc::new(ExternalSignerAdapter::new(external_signer));
                (adapter as Arc<dyn crate::signer::BreezSigner>, None)
            }
        };

        // Create the specialized signers
        let spark_signer = Arc::new(SparkSigner::new(signer.clone()));
        let rtsync_signer = Arc::new(
            RTSyncSigner::new(signer.clone(), self.config.network)
                .map_err(|e| SdkError::Generic(e.to_string()))?,
        );
        let nostr_signer = Arc::new(
            NostrSigner::new(signer.clone(), self.config.network, account_number)
                .await
                .map_err(|e| SdkError::Generic(format!("{e:?}")))?,
        );
        let lnurl_auth_signer = Arc::new(LnurlAuthSignerAdapter::new(signer.clone()));

        let chain_service = if let Some(service) = self.chain_service {
            service
        } else {
            let inner_client =
                CommonRequestRestClient::new().map_err(|e| SdkError::Generic(e.to_string()))?;
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

        let (storage, sync_storage) = match (self.storage, self.storage_dir) {
            // Use provided storages directly
            (Some(storage), _) => (storage, self.sync_storage),
            // Initialize default storages based on provided directory
            #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
            (None, Some(storage_dir)) => {
                let identity_pub_key = spark_signer
                    .get_identity_public_key()
                    .await
                    .map_err(|e| SdkError::Generic(e.to_string()))?;
                let storage =
                    default_storage(&storage_dir, self.config.network, &identity_pub_key)?;
                let sync_storage = match (self.sync_storage, &self.config.real_time_sync_server_url)
                {
                    // Use provided sync storage directly
                    (Some(sync_storage), _) => Some(sync_storage),
                    // Initialize default sync storage based on provided directory
                    // if real-time sync is enabled
                    (None, Some(_)) => Some(default_sync_storage(
                        &storage_dir,
                        self.config.network,
                        &identity_pub_key,
                    )?),
                    _ => None,
                };
                (storage, sync_storage)
            }
            _ => {
                return Err(SdkError::Generic(
                    "Either storage or storage_dir must be set before building the SDK".to_string(),
                ));
            }
        };

        let fiat_service: Arc<dyn breez_sdk_common::fiat::FiatService> = match self.fiat_service {
            Some(service) => Arc::new(FiatServiceWrapper::new(service)),
            None => Arc::new(
                BreezServer::new(PRODUCTION_BREEZSERVER_URL, None)
                    .map_err(|e| SdkError::Generic(e.to_string()))?,
            ),
        };

        let lnurl_client: Arc<dyn breez_sdk_common::rest::RestClient> = match self.lnurl_client {
            Some(client) => Arc::new(RestClientWrapper::new(client)),
            None => Arc::new(
                CommonRequestRestClient::new().map_err(|e| SdkError::Generic(e.to_string()))?,
            ),
        };
        let user_agent = format!(
            "{}/{}",
            crate::built_info::PKG_NAME,
            crate::built_info::GIT_VERSION.unwrap_or(crate::built_info::PKG_VERSION),
        );
        info!("Building SparkWallet with user agent: {}", user_agent);
        let mut spark_wallet_config =
            spark_wallet::SparkWalletConfig::default_config(self.config.network.into());
        spark_wallet_config.operator_pool = spark_wallet_config
            .operator_pool
            .with_user_agent(Some(user_agent.clone()));
        spark_wallet_config.service_provider_config.user_agent = Some(user_agent);
        spark_wallet_config.leaf_auto_optimize_enabled =
            self.config.optimization_config.auto_enabled;
        spark_wallet_config.leaf_optimization_options.multiplicity =
            self.config.optimization_config.multiplicity;

        let mut wallet_builder =
            spark_wallet::WalletBuilder::new(spark_wallet_config, spark_signer);
        if let Some(observer) = self.payment_observer {
            let observer: Arc<dyn spark_wallet::TransferObserver> =
                Arc::new(SparkTransferObserver::new(observer));
            wallet_builder = wallet_builder.with_transfer_observer(observer);
        }
        let spark_wallet = Arc::new(wallet_builder.build().await?);

        let lnurl_server_client: Option<Arc<dyn LnurlServerClient>> = match self.lnurl_server_client
        {
            Some(client) => Some(client),
            None => match &self.config.lnurl_domain {
                Some(domain) => {
                    // Get the SparkWallet instance for signing
                    Some(Arc::new(ReqwestLnurlServerClient::new(
                        domain.clone(),
                        self.config.api_key.clone(),
                        Arc::clone(&spark_wallet),
                    )?))
                }
                None => None,
            },
        };
        let shutdown_sender = watch::channel::<()>(()).0;

        let event_emitter = Arc::new(EventEmitter::new(
            self.config.real_time_sync_server_url.is_some(),
        ));
        let storage = if let Some(server_url) = &self.config.real_time_sync_server_url {
            let Some(sync_storage) = sync_storage else {
                return Err(SdkError::Generic(
                    "Real-time sync is enabled, but no sync storage is supplied".to_string(),
                ));
            };

            init_and_start_real_time_sync(RealTimeSyncParams {
                server_url: server_url.clone(),
                api_key: self.config.api_key.clone(),
                signer: rtsync_signer,
                storage: Arc::clone(&storage),
                sync_storage,
                shutdown_receiver: shutdown_sender.subscribe(),
                event_emitter: Arc::clone(&event_emitter),
            })
            .await?
        } else {
            storage
        };

        let nostr_client = Arc::new(NostrClient::new(nostr_signer));

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
            nostr_client,
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

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
fn default_sync_storage(
    data_dir: &str,
    network: Network,
    identity_pub_key: &spark_wallet::PublicKey,
) -> Result<Arc<dyn SyncStorage>, SdkError> {
    let db_path = crate::default_storage_path(data_dir, &network, identity_pub_key)?;
    let storage = Arc::new(crate::SqliteStorage::new(&db_path)?);
    Ok(storage)
}
