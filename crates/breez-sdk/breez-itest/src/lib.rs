pub mod concurrent_scenarios;
pub mod faucet;
pub mod fixtures;
pub mod helpers;
mod log;
pub mod session_store_scenarios;
#[cfg(feature = "turnkey")]
pub mod turnkey;

use std::sync::Arc;

pub use concurrent_scenarios::{
    RuntimeMode, run_concurrent_multi_instance_operations, run_concurrent_token_operations,
};
pub use faucet::RegtestFaucet;
pub use fixtures::data_sync::{DataSyncFixture, DataSyncImageConfig};
pub use fixtures::lnurl::{LnurlFixture, LnurlImageConfig};
pub use fixtures::*;
pub use helpers::*;
pub use rand;
pub use session_store_scenarios::{SessionRow, run_session_persistence_across_restart};
pub use tempfile;

use anyhow::Result;
use breez_sdk_spark::{BreezSdk, Config, SdkEvent};
use rand::RngCore;
use tempfile::TempDir;
use tokio::sync::{OnceCell, mpsc};

/// Container for SDK instance, event channel, and storage directory
/// The TempDir is kept alive to prevent premature deletion
pub struct SdkInstance {
    pub sdk: BreezSdk,
    pub events: mpsc::Receiver<SdkEvent>,
    /// Tracing span for attributing logs to this SDK instance (e.g. "alice" or "bob")
    pub span: tracing::Span,
    #[allow(dead_code)]
    temp_dir: Option<TempDir>,
    #[allow(dead_code)]
    pub data_sync_fixture: Option<Arc<DataSyncFixture>>,
    #[allow(dead_code)]
    pub lnurl_fixture: Option<Arc<LnurlFixture>>,
    /// Held only for its `Drop`: deletes a per-test Turnkey wallet on teardown.
    /// `None` for seed-backed instances.
    #[allow(dead_code)]
    turnkey_guard: Option<TurnkeyGuard>,
}

/// The per-test Turnkey wallet guard held by [`SdkInstance`]. Without the
/// `turnkey` feature this is an uninhabited placeholder, so the field exists
/// (and every construction site writes a plain `turnkey_guard: None`) without
/// feature-gating each one.
#[cfg(feature = "turnkey")]
type TurnkeyGuard = turnkey::TurnkeyWalletGuard;
#[cfg(not(feature = "turnkey"))]
type TurnkeyGuard = std::convert::Infallible;

/// Persistent SDK fixture that allows reinitialization with the same configuration
pub struct ReinitializableSdkInstance {
    pub seed: [u8; 32],
    pub storage_path: String,
    pub config: Config,
    _temp_dir: TempDir, // Keep TempDir alive to prevent directory deletion
    // Pinned on first `build_sdk()` so subsequent rebuilds reuse the same DB.
    // Without this, `USE_POSTGRES_TREE_STORE` / `USE_MYSQL_TREE_STORE` runs
    // would allocate a fresh `ts_N` database on every build and lose state.
    backend: OnceCell<BackendChoice>,
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
            backend: OnceCell::new(),
        })
    }

    /// Build the SDK instance (can be called multiple times with the same config)
    pub async fn build_sdk(&self) -> Result<SdkInstance> {
        let backend = self
            .backend
            .get_or_try_init(resolve_backend_choice)
            .await?
            .clone();
        build_sdk_with_custom_config_and_backend(
            self.storage_path.clone(),
            self.seed,
            self.config.clone(),
            None,
            true,
            Some(backend),
        )
        .await
    }
}
