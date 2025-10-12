pub mod faucet;
pub mod helpers;

pub use faucet::RegtestFaucet;
pub use helpers::*;

use breez_sdk_spark::{BreezSdk, SdkEvent};
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
