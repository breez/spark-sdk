use crate::{
    ExternalInputParser, InputType, Logger, Network, SparkStatus, error::SdkError, models::Config,
};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
use {
    crate::{ConnectRequest, models::KeySetConfig, sdk::BreezSdk, sdk_builder::SdkBuilder},
    std::sync::Arc,
};

/// Top-level namespace for the Breez SDK Spark.
///
/// `BreezSdkSpark` groups all static/global SDK functions that don't require a wallet
/// connection. Use [`BreezSdkSpark::connect`] (non-WASM) or the existing [`connect`](crate::connect)
/// free function to obtain a [`BreezSparkClient`](crate::BreezSparkClient) instance.
///
/// # Examples
///
/// ```rust,no_run
/// use breez_sdk_spark::{BreezSdkSpark, Network};
///
/// let config = BreezSdkSpark::default_config(Network::Mainnet);
/// ```
pub struct BreezSdkSpark;

#[allow(deprecated)] // delegates to deprecated free functions (default_config, init_logging, etc.)
impl BreezSdkSpark {
    /// Returns a default SDK configuration for the given network.
    ///
    /// This is equivalent to the [`default_config`](crate::default_config) free function.
    pub fn default_config(network: Network) -> Config {
        crate::default_config(network)
    }

    /// Initializes the SDK logging subsystem.
    ///
    /// This is equivalent to the [`init_logging`](crate::init_logging) free function.
    pub fn init_logging(
        log_dir: Option<String>,
        app_logger: Option<Box<dyn Logger>>,
        log_filter: Option<String>,
    ) -> Result<(), SdkError> {
        crate::init_logging(log_dir, app_logger, log_filter)
    }

    /// Parses a payment input string and returns the identified type.
    ///
    /// Supports BOLT11 invoices, Lightning addresses, LNURL variants, Bitcoin
    /// addresses, Spark addresses/invoices, BIP21 URIs, and more.
    ///
    /// This is equivalent to the [`parse`](crate::parse) free function.
    pub async fn parse(
        input: &str,
        external_input_parsers: Option<Vec<ExternalInputParser>>,
    ) -> Result<InputType, SdkError> {
        crate::parse(input, external_input_parsers).await
    }

    /// Fetches the current status of Spark network services.
    ///
    /// This is equivalent to the [`get_spark_status`](crate::get_spark_status) free function.
    pub async fn get_spark_status() -> Result<SparkStatus, SdkError> {
        crate::get_spark_status().await
    }
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[allow(deprecated)] // delegates to deprecated free functions (connect, connect_with_signer, etc.)
impl BreezSdkSpark {
    /// Creates a default external signer from a mnemonic phrase.
    ///
    /// This is equivalent to the [`default_external_signer`](crate::default_external_signer) free function.
    pub fn default_external_signer(
        mnemonic: String,
        passphrase: Option<String>,
        network: Network,
        key_set_config: Option<KeySetConfig>,
    ) -> Result<Arc<dyn crate::signer::ExternalSigner>, SdkError> {
        crate::default_external_signer(mnemonic, passphrase, network, key_set_config)
    }

    /// Creates an SDK builder for advanced configuration.
    ///
    /// Use this when you need to customize storage, chain services, or other
    /// provider implementations.
    pub fn builder(config: Config, seed: crate::Seed) -> SdkBuilder {
        SdkBuilder::new(config, seed)
    }

    /// Connects to the Spark network using the provided configuration and seed.
    ///
    /// This is equivalent to the [`connect`](crate::connect) free function.
    ///
    /// # Arguments
    ///
    /// * `request` - The connection request containing config, seed, and storage directory
    ///
    /// # Returns
    ///
    /// An initialized [`BreezSparkClient`](crate::BreezSparkClient) instance
    pub async fn connect(request: ConnectRequest) -> Result<BreezSdk, SdkError> {
        crate::connect(request).await
    }

    /// Connects to the Spark network using an external signer.
    ///
    /// This is equivalent to the [`connect_with_signer`](crate::connect_with_signer) free function.
    pub async fn connect_with_signer(
        request: crate::ConnectWithSignerRequest,
    ) -> Result<BreezSdk, SdkError> {
        crate::connect_with_signer(request).await
    }
}

#[cfg(test)]
#[allow(deprecated)] // tests call deprecated default_config to verify parity
mod tests {
    use super::*;

    #[test]
    fn test_breez_default_config_matches_free_function() {
        let from_breez = BreezSdkSpark::default_config(Network::Mainnet);
        let from_free = crate::default_config(Network::Mainnet);

        assert!(
            matches!(from_breez.network, Network::Mainnet),
            "Expected Mainnet network"
        );
        assert_eq!(from_breez.api_key, from_free.api_key);
        assert_eq!(from_breez.sync_interval_secs, from_free.sync_interval_secs);
        assert_eq!(from_breez.lnurl_domain, from_free.lnurl_domain);
        assert_eq!(
            from_breez.prefer_spark_over_lightning,
            from_free.prefer_spark_over_lightning
        );
        assert_eq!(
            from_breez.private_enabled_default,
            from_free.private_enabled_default
        );
    }

    #[test]
    fn test_breez_default_config_regtest() {
        let config = BreezSdkSpark::default_config(Network::Regtest);

        assert!(
            matches!(config.network, Network::Regtest),
            "Expected Regtest network"
        );
        assert!(config.lnurl_domain.is_none());
    }

    #[test]
    fn test_breez_spark_client_type_alias_compiles() {
        // Verify the type aliases are usable at compile time
        fn _takes_spark_client(_sdk: &crate::BreezSparkClient) {}
        fn _takes_breez_sdk(_sdk: &crate::BreezSdk) {}

        // Both should accept the same type — this is a compile-time test
    }
}
