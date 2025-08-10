use spark_wallet::DefaultSigner;
use tokio::sync::watch;

use crate::{
    Network,
    chain::{BitcoinChainService, rest_client::RestClientChainService},
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
    chain_service: Option<Box<dyn BitcoinChainService + Send + Sync>>,
}

impl SdkBuilder {
    /// Creates a new `SdkBuilder` with the provided configuration.
    pub fn new(config: Config, mnemonic: String, storage: Box<dyn Storage + Send + Sync>) -> Self {
        SdkBuilder {
            config,
            mnemonic,
            storage,
            chain_service: None,
        }
    }

    pub fn with_chain_service(
        mut self,
        chain_service: Box<dyn BitcoinChainService + Send + Sync>,
    ) -> Self {
        self.chain_service = Some(chain_service);
        self
    }

    /// Builds the `BreezSdk` instance with the configured components.
    pub async fn build(self) -> Result<BreezSdk, SdkError> {
        // Create the signer from mnemonic
        let mnemonic = bip39::Mnemonic::parse(&self.mnemonic)
            .map_err(|e| SdkError::GenericError(e.to_string()))?;
        let signer = DefaultSigner::new(&mnemonic.to_seed(""), self.config.network.clone().into())
            .map_err(|e| SdkError::GenericError(e.to_string()))?;
        let chain_service = self
            .chain_service
            .unwrap_or_else(|| match self.config.network {
                Network::Mainnet => Box::new(RestClientChainService::new(
                    "https://blockstream.info/api".to_string(),
                    self.config.network.clone(),
                    5,
                )),
                Network::Regtest => Box::new(RestClientChainService::new(
                    "https://mempool.space/api".to_string(),
                    self.config.network.clone(),
                    5,
                )),
            });
        let (shutdown_sender, shutdown_receiver) = watch::channel::<()>(());
        // Create the SDK instance
        let sdk = BreezSdk::new(
            self.config,
            signer,
            self.storage.into(),
            chain_service.into(),
            shutdown_sender,
            shutdown_receiver,
        )
        .await?;

        sdk.start()?;
        Ok(sdk)
    }
}
