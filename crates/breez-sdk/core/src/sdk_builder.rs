#![cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    allow(clippy::arc_with_non_send_sync)
)]
use std::sync::Arc;

use breez_sdk_common::rest::{ReqwestRestClient as CommonRequestRestClient, RestClient};
use spark_wallet::DefaultSigner;
use tokio::sync::watch;

use crate::{
    Credentials, Network,
    chain::{
        BitcoinChainService,
        rest_client::{BasicAuth, RestClientChainService},
    },
    error::SdkError,
    models::Config,
    persist::Storage,
    sdk::BreezSdk,
};

/// Builder for creating `BreezSdk` instances with customizable components.
#[derive(Clone)]
pub struct SdkBuilder {
    config: Config,
    mnemonic: String,
    storage: Arc<dyn Storage>,
    chain_service: Option<Arc<dyn BitcoinChainService>>,
    lnurl_client: Option<Arc<dyn RestClient>>,
}

impl SdkBuilder {
    /// Creates a new `SdkBuilder` with the provided configuration.
    /// Arguments:
    /// - `config`: The configuration to be used.
    /// - `mnemonic`: The mnemonic phrase for the wallet.
    /// - `storage`: The storage backend to be used.
    pub fn new(config: Config, mnemonic: String, storage: Arc<dyn Storage>) -> Self {
        SdkBuilder {
            config,
            mnemonic,
            storage,
            chain_service: None,
            lnurl_client: None,
        }
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

    #[must_use]
    pub fn with_lnurl_client(mut self, lnurl_client: Arc<dyn RestClient>) -> Self {
        self.lnurl_client = Some(lnurl_client);
        self
    }

    /// Builds the `BreezSdk` instance with the configured components.
    pub async fn build(self) -> Result<BreezSdk, SdkError> {
        // Create the signer from mnemonic
        let mnemonic =
            bip39::Mnemonic::parse(&self.mnemonic).map_err(|e| SdkError::Generic(e.to_string()))?;
        let signer = DefaultSigner::new(&mnemonic.to_seed(""), self.config.network.into())
            .map_err(|e| SdkError::Generic(e.to_string()))?;
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
                        _ => None,
                    },
                )),
            }
        };
        let lnurl_client: Arc<dyn RestClient> = match self.lnurl_client {
            Some(client) => client,
            None => Arc::new(
                CommonRequestRestClient::new().map_err(|e| SdkError::Generic(e.to_string()))?,
            ),
        };
        let (shutdown_sender, shutdown_receiver) = watch::channel::<()>(());
        // Create the SDK instance
        let sdk = BreezSdk::new(
            self.config,
            signer,
            self.storage,
            chain_service,
            lnurl_client,
            shutdown_sender,
            shutdown_receiver,
        )
        .await?;

        sdk.start();
        Ok(sdk)
    }
}
