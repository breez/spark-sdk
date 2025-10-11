#![cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    allow(clippy::arc_with_non_send_sync)
)]
use std::sync::Arc;

use breez_sdk_common::{
    breez_server::{BreezServer, PRODUCTION_BREEZSERVER_URL},
    fiat::FiatService,
    rest::{ReqwestRestClient as CommonRequestRestClient, RestClient},
    sync::{client::BreezSyncerClient, signing_client::SigningClient},
};
use spark_wallet::{DefaultSigner, Signer};
use tokio::sync::{mpsc, watch};
use tracing::debug;
use uuid::Uuid;

use crate::{
    Credentials, KeySetType, Network,
    chain::{
        BitcoinChainService,
        rest_client::{BasicAuth, RestClientChainService},
    },
    error::SdkError,
    lnurl::{LnurlServerClient, ReqwestLnurlServerClient},
    models::Config,
    payment_observer::{PaymentObserver, SparkTransferObserver},
    persist::Storage,
    realtime_sync::{DefaultSyncSigner, SyncProcessor, SyncService, SyncedStorage},
    sdk::{BreezSdk, BreezSdkParams},
};

/// Represents the seed for wallet generation, either as a mnemonic phrase with an optional
/// passphrase or as raw entropy bytes.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Seed {
    /// A BIP-39 mnemonic phrase with an optional passphrase.
    Mnemonic {
        /// The mnemonic phrase. 12 or 24 words.
        mnemonic: String,
        /// An optional passphrase for the mnemonic.
        passphrase: Option<String>,
    },
    /// Raw entropy bytes.
    Entropy(Vec<u8>),
}

/// Builder for creating `BreezSdk` instances with customizable components.
#[derive(Clone)]
pub struct SdkBuilder {
    config: Config,
    seed: Seed,
    storage: Arc<dyn Storage>,
    chain_service: Option<Arc<dyn BitcoinChainService>>,
    fiat_service: Option<Arc<dyn FiatService>>,
    lnurl_client: Option<Arc<dyn RestClient>>,
    lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    payment_observer: Option<Arc<dyn PaymentObserver>>,
    key_set_type: KeySetType,
    use_address_index: bool,
    account_number: Option<u32>,
    real_time_sync_server_url: Option<String>,
}

impl SdkBuilder {
    /// Creates a new `SdkBuilder` with the provided configuration.
    /// Arguments:
    /// - `config`: The configuration to be used.
    /// - `seed`: The seed for wallet generation.
    /// - `storage`: The storage backend to be used.
    pub fn new(config: Config, seed: Seed, storage: Arc<dyn Storage>) -> Self {
        SdkBuilder {
            config,
            seed,
            storage,
            chain_service: None,
            fiat_service: None,
            lnurl_client: None,
            lnurl_server_client: None,
            payment_observer: None,
            key_set_type: KeySetType::Default,
            use_address_index: false,
            account_number: None,
            real_time_sync_server_url: None,
        }
    }

    /// Sets the key set type to be used by the SDK.
    /// Arguments:
    /// - `key_set_type`: The key set type which determines the derivation path.
    /// - `use_address_index`: Controls the structure of the BIP derivation path.
    #[must_use]
    pub fn with_key_set(
        mut self,
        key_set_type: KeySetType,
        use_address_index: bool,
        account_number: Option<u32>,
    ) -> Self {
        self.key_set_type = key_set_type;
        self.use_address_index = use_address_index;
        self.account_number = account_number;
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
    /// - `credentials`: Optional credentials for basic authentication.
    #[must_use]
    pub fn with_rest_chain_service(
        mut self,
        url: String,
        credentials: Option<Credentials>,
    ) -> Self {
        self.chain_service = Some(Arc::new(RestClientChainService::new(
            url,
            self.config.network,
            5,
            Box::new(CommonRequestRestClient::new().unwrap()),
            credentials.map(|c| BasicAuth::new(c.username, c.password)),
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

    #[must_use]
    pub fn with_real_time_sync(mut self, server_url: String) -> Self {
        self.real_time_sync_server_url = Some(server_url);
        self
    }

    /// Builds the `BreezSdk` instance with the configured components.
    #[allow(clippy::too_many_lines)]
    pub async fn build(self) -> Result<BreezSdk, SdkError> {
        // Create the signer from seed
        let seed = match self.seed {
            Seed::Mnemonic {
                mnemonic,
                passphrase,
            } => {
                let mnemonic = bip39::Mnemonic::parse(&mnemonic)
                    .map_err(|e| SdkError::Generic(e.to_string()))?;

                mnemonic
                    .to_seed(passphrase.as_deref().unwrap_or(""))
                    .to_vec()
            }
            Seed::Entropy(entropy) => entropy,
        };

        let signer: Arc<dyn Signer> = Arc::new(
            DefaultSigner::with_keyset_type(
                &seed,
                self.config.network.into(),
                self.key_set_type.into(),
                self.use_address_index,
                self.account_number,
            )
            .map_err(|e| SdkError::Generic(e.to_string()))?,
        );
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
                )),
            }
        };

        let fiat_service: Arc<dyn FiatService> = match self.fiat_service {
            Some(service) => service,
            None => Arc::new(
                BreezServer::new(PRODUCTION_BREEZSERVER_URL, None)
                    .map_err(|e| SdkError::Generic(e.to_string()))?,
            ),
        };

        let lnurl_client: Arc<dyn RestClient> = match self.lnurl_client {
            Some(client) => client,
            None => Arc::new(
                CommonRequestRestClient::new().map_err(|e| SdkError::Generic(e.to_string()))?,
            ),
        };
        let spark_wallet_config =
            spark_wallet::SparkWalletConfig::default_config(self.config.network.into());

        let mut wallet_builder =
            spark_wallet::WalletBuilder::new(spark_wallet_config, Arc::clone(&signer));
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

        let (storage, sync_processor) = if let Some(server_url) = &self.real_time_sync_server_url {
            debug!("Real-time sync is enabled.");
            let sync_service = Arc::new(SyncService::new(Arc::clone(&self.storage)));
            let synced_storage = Arc::new(SyncedStorage::new(
                Arc::clone(&self.storage),
                Arc::clone(&sync_service),
            ));

            let (incoming_callback_sender, incoming_callback_receiver) = mpsc::channel(10);
            let (outgoing_callback_sender, outgoing_callback_receiver) = mpsc::channel(10);

            synced_storage.listen(incoming_callback_receiver, outgoing_callback_receiver);
            let storage: Arc<dyn Storage> = synced_storage;
            let sync_client = BreezSyncerClient::new(server_url, self.config.api_key.as_deref())
                .map_err(|e| SdkError::Generic(e.to_string()))?;
            let sync_signer = DefaultSyncSigner::new(
                Arc::clone(&signer),
                "m/448201320'/0'/0'/0/0".parse().map_err(|_| {
                    SdkError::Generic(
                        "Someone put an invalid static derivation path here".to_string(),
                    )
                })?,
            );
            let signing_sync_client = SigningClient::new(
                Arc::new(sync_client),
                Arc::new(sync_signer),
                Uuid::now_v7().to_string(),
            );
            let sync_processor = Arc::new(SyncProcessor::new(
                signing_sync_client,
                sync_service.get_sync_trigger(),
                incoming_callback_sender,
                outgoing_callback_sender,
                Arc::clone(&storage),
            ));
            (storage, Some(sync_processor))
        } else {
            (Arc::clone(&self.storage), None)
        };

        // Create the SDK instance
        let sdk = BreezSdk::init_and_start(BreezSdkParams {
            config: self.config,
            storage,
            chain_service,
            fiat_service,
            lnurl_client,
            lnurl_server_client,
            shutdown_sender,
            spark_wallet,
            sync_processor,
        })
        .await?;

        debug!("Initialized and started breez sdk.");

        Ok(sdk)
    }
}
