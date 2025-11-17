pub mod faucet;
pub mod fixtures;
pub mod helpers;
mod log;

use std::sync::Arc;

pub use faucet::RegtestFaucet;
pub use fixtures::data_sync::{DataSyncFixture, DataSyncImageConfig};
pub use fixtures::lnurl::{LnurlFixture, LnurlImageConfig};
pub use fixtures::*;
pub use helpers::*;

use anyhow::Result;
use breez_sdk_spark::{BreezSdk, Config, SdkEvent};
use rand::RngCore;
use tempdir::TempDir;
use tokio::sync::mpsc;

/// Container for SDK instance, event channel, and storage directory
/// The TempDir is kept alive to prevent premature deletion
pub struct SdkInstance {
    pub sdk: BreezSdk,
    pub events: mpsc::Receiver<SdkEvent>,
    #[allow(dead_code)]
    temp_dir: Option<TempDir>,
    #[allow(dead_code)]
    pub data_sync_fixture: Option<Arc<DataSyncFixture>>,
    #[allow(dead_code)]
    pub lnurl_fixture: Option<Arc<LnurlFixture>>,
}

/// Persistent SDK fixture that allows reinitialization with the same configuration
pub struct ReinitializableSdkInstance {
    pub seed: [u8; 32],
    pub storage_path: String,
    pub config: Config,
    _temp_dir: TempDir, // Keep TempDir alive to prevent directory deletion
}

impl ReinitializableSdkInstance {
    /// Create a new persistent SDK fixture with custom initial config
    pub fn new(config: Config, temp_dir: TempDir) -> Result<Self> {
        let storage_path = temp_dir.path().to_string_lossy().to_string();

        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);

        Ok(Self {
            seed,
            storage_path,
            config,
            _temp_dir: temp_dir,
        })
    }

    /// Build the SDK instance (can be called multiple times with the same config)
    pub async fn build_sdk(&self) -> Result<SdkInstance> {
        build_sdk_with_custom_config(
            self.storage_path.clone(),
            self.seed,
            self.config.clone(),
            None,
            true,
        )
        .await
    }
}
