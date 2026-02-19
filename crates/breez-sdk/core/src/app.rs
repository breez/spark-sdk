use crate::{
    ExternalInputParser, InputType, Logger, Network, SparkStatus, error::SdkError, models::Config,
};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
use {
    crate::{
        ConnectOptions, ConnectRequest, Providers, SdkCredentials, models::KeySetConfig,
        sdk::BreezSdk, sdk_builder::SdkBuilder,
    },
    std::sync::Arc,
};

/// Top-level namespace for the Breez SDK.
///
/// `Breez` groups all static/global SDK functions that don't require a wallet
/// connection. Use [`Breez::connect`] (non-WASM) to obtain a [`BreezSdk`]
/// (also exported as [`BreezClient`]) instance.
///
/// # Examples
///
/// ```rust,no_run
/// use breez_sdk_spark::{Breez, SdkCredentials};
///
/// # async {
/// let sdk = Breez::connect(
///     SdkCredentials::Mnemonic {
///         api_key: "<breez api key>".into(),
///         mnemonic: "<mnemonic words>".into(),
///         passphrase: None,
///     },
///     None,
/// ).await.unwrap();
/// # };
/// ```
pub struct Breez;

#[allow(deprecated)]
impl Breez {
    /// Returns a default SDK configuration for the given network.
    ///
    /// This is equivalent to the [`default_config`](crate::default_config) free function.
    pub fn default_config(network: Network) -> Config {
        crate::default_config(network)
    }

    /// Parses a payment input string and returns the identified type.
    ///
    /// Supports BOLT11 invoices, Lightning addresses, LNURL variants, Bitcoin
    /// addresses, Spark addresses/invoices, BIP21 URIs, and more.
    ///
    /// This is equivalent to the [`parse_input`](crate::parse_input) free function.
    pub async fn parse(
        input: &str,
        external_input_parsers: Option<Vec<ExternalInputParser>>,
    ) -> Result<InputType, SdkError> {
        crate::parse_input(input, external_input_parsers).await
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

    /// Fetches the current status of Spark network services.
    ///
    /// This is equivalent to the [`get_spark_status`](crate::get_spark_status) free function.
    pub async fn get_spark_status() -> Result<SparkStatus, SdkError> {
        crate::get_spark_status().await
    }
}

// Non-WASM-only methods
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[allow(deprecated)]
impl Breez {
    /// Connects to the Spark network using credentials and optional configuration.
    ///
    /// This is the primary entry point for initializing the SDK. For most use cases,
    /// only credentials are needed — sensible defaults are applied automatically.
    ///
    /// # Arguments
    /// * `credentials` - API key + authentication (mnemonic or external signer)
    /// * `options` - Optional configuration overrides (network, storage, etc.)
    ///
    /// # Examples
    /// ```rust,no_run
    /// use breez_sdk_spark::{Breez, SdkCredentials};
    ///
    /// # async {
    /// let sdk = Breez::connect(
    ///     SdkCredentials::Mnemonic {
    ///         api_key: "<api key>".into(),
    ///         mnemonic: "<words>".into(),
    ///         passphrase: None,
    ///     },
    ///     None,
    /// ).await.unwrap();
    /// # };
    /// ```
    pub async fn connect(
        credentials: SdkCredentials,
        options: Option<ConnectOptions>,
    ) -> Result<BreezSdk, SdkError> {
        let opts = options.unwrap_or_default();
        Self::connect_internal(credentials, opts, Providers::default()).await
    }

    /// Connects using a legacy [`ConnectRequest`].
    ///
    /// Prefer [`Breez::connect()`] for new code.
    pub async fn connect_legacy(request: ConnectRequest) -> Result<BreezSdk, SdkError> {
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

    /// Returns a builder with custom service providers for advanced initialization.
    ///
    /// Use this when you need to inject custom storage, chain service, fiat service,
    /// or payment observers. For standard use cases, use [`Breez::connect()`] instead.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use breez_sdk_spark::{Breez, Providers, SdkCredentials};
    ///
    /// # async {
    /// let providers = Providers {
    ///     storage: None, // Some(custom_storage),
    ///     ..Default::default()
    /// };
    /// let sdk = Breez::with_providers(providers)
    ///     .connect(
    ///         SdkCredentials::Mnemonic {
    ///             api_key: "<api key>".into(),
    ///             mnemonic: "<words>".into(),
    ///             passphrase: None,
    ///         },
    ///         None,
    ///     )
    ///     .await
    ///     .unwrap();
    /// # };
    /// ```
    pub fn with_providers(providers: Providers) -> BreezWithProviders {
        BreezWithProviders { providers }
    }

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

    /// Internal shared connect implementation.
    async fn connect_internal(
        credentials: SdkCredentials,
        opts: ConnectOptions,
        providers: Providers,
    ) -> Result<BreezSdk, SdkError> {
        let storage_dir = opts
            .storage_dir
            .clone()
            .unwrap_or_else(|| "./.data".to_string());

        match credentials {
            SdkCredentials::Mnemonic { .. } => {
                let (config, seed) = credentials.to_config_and_seed(&opts)?;
                let mut builder = SdkBuilder::new(config, seed).with_default_storage(storage_dir);
                if let Some(key_set) = opts.key_set {
                    builder = builder.with_key_set(key_set);
                }
                builder = Self::apply_providers(builder, providers);
                builder.build().await
            }
            SdkCredentials::Signer {
                signer, api_key, ..
            } => {
                let network = opts.network.unwrap_or(Network::Mainnet);
                let mut config = crate::default_config(network);
                config.api_key = Some(api_key);
                config = opts.apply_to_config(config);
                let mut builder =
                    SdkBuilder::new_with_signer(config, signer).with_default_storage(storage_dir);
                builder = Self::apply_providers(builder, providers);
                builder.build().await
            }
        }
    }

    /// Applies custom providers to an SDK builder.
    fn apply_providers(mut builder: SdkBuilder, providers: Providers) -> SdkBuilder {
        if let Some(storage) = providers.storage {
            builder = builder.with_storage(storage);
        }
        if let Some(chain_service) = providers.chain_service {
            builder = builder.with_chain_service(chain_service);
        }
        if let Some(fiat_service) = providers.fiat_service {
            builder = builder.with_fiat_service(fiat_service);
        }
        if let Some(lnurl_client) = providers.lnurl_client {
            builder = builder.with_lnurl_client(lnurl_client);
        }
        if let Some(observer) = providers.payment_observer {
            builder = builder.with_payment_observer(observer);
        }
        builder
    }
}

/// Builder returned by [`Breez::with_providers()`] for connecting with custom providers.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub struct BreezWithProviders {
    providers: Providers,
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl BreezWithProviders {
    /// Connects with the pre-configured custom providers.
    pub async fn connect(
        self,
        credentials: SdkCredentials,
        options: Option<ConnectOptions>,
    ) -> Result<BreezSdk, SdkError> {
        let opts = options.unwrap_or_default();
        Breez::connect_internal(credentials, opts, self.providers).await
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    #[test]
    fn test_breez_default_config_matches_free_function() {
        let from_breez = Breez::default_config(Network::Mainnet);
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
        let config = Breez::default_config(Network::Regtest);

        assert!(
            matches!(config.network, Network::Regtest),
            "Expected Regtest network"
        );
        assert!(config.lnurl_domain.is_none());
    }

    #[test]
    fn test_breez_client_type_alias_compiles() {
        // Verify the type alias is usable at compile time
        fn _takes_breez_client(_sdk: &crate::BreezClient) {}
        fn _takes_breez_sdk(_sdk: &crate::BreezSdk) {}

        // Both should accept the same type — this is a compile-time test
    }
}
