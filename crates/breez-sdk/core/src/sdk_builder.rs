use spark_wallet::DefaultSigner;
use tokio::sync::watch;

use crate::{error::SdkError, models::Config, persist::Storage, sdk::BreezSdk};

/// Builder for creating `BreezSdk` instances with customizable components.
pub struct SdkBuilder {
    config: Config,
    mnemonic: String,
    storage: Box<dyn Storage + Send + Sync>,
}

impl SdkBuilder {
    /// Creates a new `SdkBuilder` with the provided configuration.
    pub fn new(config: Config, mnemonic: String, storage: Box<dyn Storage + Send + Sync>) -> Self {
        SdkBuilder {
            config,
            mnemonic,
            storage,
        }
    }

    /// Builds the `BreezSdk` instance with the configured components.
    pub async fn build(self) -> Result<BreezSdk, SdkError> {
        // Create the signer from mnemonic
        let mnemonic = bip39::Mnemonic::parse(&self.mnemonic)
            .map_err(|e| SdkError::GenericError(e.to_string()))?;
        let signer = DefaultSigner::new(&mnemonic.to_seed(""), self.config.network.clone().into())
            .map_err(|e| SdkError::GenericError(e.to_string()))?;

        let (shutdown_sender, shutdown_receiver) = watch::channel::<()>(());
        // Create the SDK instance
        let sdk = BreezSdk::new(
            self.config,
            signer,
            self.storage.into(),
            shutdown_sender,
            shutdown_receiver,
        )
        .await?;

        sdk.start()?;
        Ok(sdk)
    }
}
