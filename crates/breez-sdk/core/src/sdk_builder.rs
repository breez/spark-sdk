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
pub struct SdkBuilder {
    config: Config,
    mnemonic: String,
    storage: Box<dyn Storage + Send + Sync>,
    chain_service: Option<Box<dyn BitcoinChainService>>,
    lnurl_client: Option<Box<dyn RestClient>>,
}

impl SdkBuilder {
    /// Creates a new `SdkBuilder` with the provided configuration.
    pub fn new(config: Config, mnemonic: String, storage: Box<dyn Storage + Send + Sync>) -> Self {
        SdkBuilder {
            config,
            mnemonic,
            storage,
            chain_service: None,
            lnurl_client: None,
        }
    }

    pub fn with_chain_service(
        mut self,
        chain_service: Box<dyn BitcoinChainService + Send + Sync>,
    ) -> Self {
        self.chain_service = Some(chain_service);
        self
    }

    pub fn with_rest_chain_service(
        mut self,
        url: String,
        credentials: Option<Credentials>,
    ) -> Self {
        self.chain_service = Some(Box::new(RestClientChainService::new(
            url,
            self.config.network,
            5,
            Box::new(CommonRequestRestClient::new().unwrap()),
            credentials.map(|c| BasicAuth::new(c.username, c.password)),
        )));
        self
    }

    pub fn with_lnurl_client(mut self, lnurl_client: Box<dyn RestClient>) -> Self {
        self.lnurl_client = Some(lnurl_client);
        self
    }

    /// Builds the `BreezSdk` instance with the configured components.
    pub async fn build(self) -> Result<BreezSdk, SdkError> {
        // Create the signer from mnemonic
        let mnemonic = bip39::Mnemonic::parse(&self.mnemonic)
            .map_err(|e| SdkError::GenericError(e.to_string()))?;
        let signer = DefaultSigner::new(&mnemonic.to_seed(""), self.config.network.into())
            .map_err(|e| SdkError::GenericError(e.to_string()))?;
        let chain_service = match self.chain_service {
            Some(service) => service,
            None => {
                let inner_client = CommonRequestRestClient::new()
                    .map_err(|e| SdkError::GenericError(e.to_string()))?;
                match self.config.network {
                    Network::Mainnet => Box::new(RestClientChainService::new(
                        "https://blockstream.info/api".to_string(),
                        self.config.network,
                        5,
                        Box::new(inner_client),
                        None,
                    )),
                    Network::Regtest => Box::new(RestClientChainService::new(
                        "https://regtest-mempool.loadtest.dev.sparkinfra.net/api".to_string(),
                        self.config.network,
                        5,
                        Box::new(inner_client),
                        match (
                            std::env::var("CHAIN_SERVICE_USERNAME"),
                            std::env::var("CHAIN_SERVICE_PASSWORD"),
                        ) {
                            (Ok(username), Ok(password)) => {
                                Some(BasicAuth::new(username, password))
                            }
                            _ => None,
                        },
                    )),
                }
            }
        };
        let lnurl_client: Arc<dyn RestClient> = match self.lnurl_client {
            Some(client) => client.into(),
            None => Arc::new(
                CommonRequestRestClient::new()
                    .map_err(|e| SdkError::GenericError(e.to_string()))?,
            ),
        };
        let (shutdown_sender, shutdown_receiver) = watch::channel::<()>(());
        // Create the SDK instance
        let sdk = BreezSdk::new(
            self.config,
            signer,
            self.storage.into(),
            chain_service.into(),
            lnurl_client,
            shutdown_sender,
            shutdown_receiver,
        )
        .await?;

        sdk.start()?;
        Ok(sdk)
    }
}
