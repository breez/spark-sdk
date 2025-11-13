pub mod faucet;
pub mod helpers;

pub use faucet::RegtestFaucet;
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
        )
        .await
    }
}
