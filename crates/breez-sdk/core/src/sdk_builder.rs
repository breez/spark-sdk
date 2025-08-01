use spark_wallet::DefaultSigner;
use std::{path::PathBuf, str::FromStr, sync::Arc};
use tokio::sync::watch;

use crate::{
    error::SdkError,
    models::Config,
    persist::{SqliteStorage, Storage},
    sdk::BreezSdk,
};

/// Builder for creating `BreezSdk` instances with customizable components.
pub struct SdkBuilder {
    config: Config,
    storage: Option<Arc<dyn Storage + Send + Sync>>,
}

impl SdkBuilder {
    /// Creates a new `SdkBuilder` with the provided configuration.
    pub fn new(config: Config) -> Self {
        SdkBuilder {
            config,
            storage: None,
        }
    }

    /// Sets a custom storage implementation.
    #[must_use]
    pub fn with_storage(mut self, storage: Arc<dyn Storage + Send + Sync>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Builds the `BreezSdk` instance with the configured components.
    pub async fn build(self) -> Result<BreezSdk, SdkError> {
        // Create the signer from mnemonic
        let mnemonic = bip39::Mnemonic::parse(&self.config.mnemonic)
            .map_err(|e| SdkError::GenericError(e.to_string()))?;
        let signer = DefaultSigner::new(&mnemonic.to_seed(""), self.config.network.clone().into())
            .map_err(|e| SdkError::GenericError(e.to_string()))?;

        // Use provided storage or create default SqliteStorage
        let storage = if let Some(storage) = self.storage {
            storage
        } else {
            // Create default SQLite storage in the data directory
            let db_path = PathBuf::from_str(&self.config.data_dir)?;
            let storage = SqliteStorage::new(&db_path)?;
            Arc::new(storage)
        };

        let (shutdown_sender, shutdown_receiver) = watch::channel::<()>(());
        // Create the SDK instance
        let sdk = BreezSdk::new(
            self.config,
            signer,
            storage,
            shutdown_sender,
            shutdown_receiver,
        )
        .await?;

        Ok(sdk)
    }
}
