use std::sync::Arc;

use breez_sdk_spark::*;
use flutter_rust_bridge::frb;

use crate::events::BindingEventListener;
use crate::frb_generated::StreamSink;
use crate::logger::BindingLogger;
use crate::sdk::BreezSdk;

/// Top-level namespace for the Breez SDK.
///
/// Groups all static/global SDK functions that don't require a wallet
/// connection. Use `Breez.connect()` to obtain a `BreezSdk` instance.
pub struct Breez;

#[allow(deprecated)]
impl Breez {
    /// Returns a default SDK configuration for the given network.
    #[frb(sync)]
    pub fn default_config(network: Network) -> Config {
        breez_sdk_spark::default_config(network)
    }

    /// Connects to the Spark network using mnemonic credentials and optional configuration.
    ///
    /// This is the primary entry point for initializing the SDK.
    pub async fn connect(
        api_key: String,
        mnemonic: String,
        passphrase: Option<String>,
        options: Option<ConnectOptions>,
    ) -> Result<BreezSdk, SdkError> {
        let credentials = SdkCredentials::Mnemonic {
            api_key,
            mnemonic,
            passphrase,
        };
        let sdk = breez_sdk_spark::Breez::connect(credentials, options).await?;
        Ok(BreezSdk {
            inner: Arc::new(sdk),
        })
    }

    /// Connects using a legacy `ConnectRequest`.
    ///
    /// Prefer `Breez.connect()` for new code.
    pub async fn connect_legacy(request: ConnectRequest) -> Result<BreezSdk, SdkError> {
        let sdk = breez_sdk_spark::connect(request).await?;
        Ok(BreezSdk {
            inner: Arc::new(sdk),
        })
    }

    /// Initializes the SDK logging subsystem.
    #[frb(sync)]
    pub fn init_logging(
        log_dir: Option<String>,
        app_logger: StreamSink<LogEntry>,
        log_filter: Option<String>,
    ) -> Result<(), SdkError> {
        let app_logger: Box<dyn Logger> = Box::new(BindingLogger { logger: app_logger });
        breez_sdk_spark::init_logging(log_dir, Some(app_logger), log_filter)
    }

    /// Fetches the current status of Spark network services.
    pub async fn get_spark_status() -> Result<SparkStatus, SdkError> {
        breez_sdk_spark::get_spark_status().await
    }
}
