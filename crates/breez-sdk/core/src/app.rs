use std::sync::Arc;

use crate::{
    ExternalInputParser, MaxFee, Network, OptimizationConfig, SdkError,
    models::{AppConfig, Config, ConnectConfig, ClientConfig},
    sdk::BREEZ_SYNC_SERVICE_URL,
};

/// All Option fields from `AppConfig` resolved to concrete values.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedAppConfig {
    pub api_key: String,
    pub network: Network,
    pub storage_root: String,
    pub sync_interval_secs: u32,
    pub max_deposit_claim_fee: Option<MaxFee>,
    pub lnurl_domain: Option<String>,
    pub prefer_spark: bool,
    pub external_input_parsers: Option<Vec<ExternalInputParser>>,
    pub use_default_external_input_parsers: bool,
    pub real_time_sync_server_url: Option<String>,
    pub private_mode: bool,
    pub optimization: OptimizationConfig,
}

pub(crate) struct AppInner {
    pub(crate) config: ResolvedAppConfig,
}

/// Platform identity — holds validated, immutable configuration.
///
/// Created once per application. Shareable across multiple [`Wallet`](crate::Wallet)
/// instances. Holds no wallet state and no async resources.
///
/// # Examples
///
/// Minimal:
/// ```ignore
/// let app = App::new(AppConfig {
///     api_key: "brz_test_...".into(),
///     network: Network::Mainnet,
///     ..Default::default()
/// })?;
/// ```
///
/// With overrides:
/// ```ignore
/// let app = App::new(AppConfig {
///     api_key: "brz_test_...".into(),
///     network: Network::Mainnet,
///     optimization: Some(OptimizationConfig { auto_enabled: true, multiplicity: 2 }),
///     prefer_spark: Some(true),
///     ..Default::default()
/// })?;
/// ```
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct App {
    pub(crate) inner: Arc<AppInner>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("network", &self.inner.config.network)
            .field("api_key", &"***")
            .finish()
    }
}

impl App {
    /// Create a new `App` with the given configuration.
    ///
    /// Resolves all optional fields to sensible defaults (same defaults as the
    /// legacy `default_config()` function). Validates required fields.
    ///
    /// # Errors
    ///
    /// Returns `SdkError::InvalidInput` if:
    /// - `api_key` is empty (on mainnet)
    /// - `optimization.multiplicity` is greater than 5
    pub fn new(config: AppConfig) -> Result<Self, SdkError> {
        // Validate
        if config.network == Network::Mainnet && config.api_key.is_empty() {
            return Err(SdkError::InvalidInput(
                "api_key is required for mainnet".to_string(),
            ));
        }

        if let Some(ref opt) = config.optimization {
            if opt.multiplicity > 5 {
                return Err(SdkError::InvalidInput(format!(
                    "optimization multiplicity must be 0-5, got {}",
                    opt.multiplicity
                )));
            }
        }

        let lnurl_domain = config.lnurl_domain.or_else(|| match config.network {
            Network::Mainnet => Some("breez.tips".to_string()),
            Network::Regtest => None,
        });

        let max_deposit_claim_fee = config
            .max_deposit_claim_fee
            .map_or(Some(MaxFee::Rate { sat_per_vbyte: 1 }), Some);

        let real_time_sync_server_url = config
            .real_time_sync_server_url
            .or_else(|| Some(BREEZ_SYNC_SERVICE_URL.to_string()));

        let resolved = ResolvedAppConfig {
            api_key: config.api_key,
            network: config.network,
            storage_root: config.storage_root.unwrap_or_else(|| "./.breez".to_string()),
            sync_interval_secs: config.sync_interval_secs.unwrap_or(60),
            max_deposit_claim_fee,
            lnurl_domain,
            prefer_spark: config.prefer_spark.unwrap_or(false),
            external_input_parsers: config.external_input_parsers,
            use_default_external_input_parsers: config
                .use_default_external_input_parsers
                .unwrap_or(true),
            real_time_sync_server_url,
            private_mode: config.private_mode.unwrap_or(true),
            optimization: config.optimization.unwrap_or(OptimizationConfig {
                auto_enabled: true,
                multiplicity: 1,
            }),
        };

        Ok(Self {
            inner: Arc::new(AppInner { config: resolved }),
        })
    }

    /// Merge app-level defaults with per-wallet overrides to produce a legacy
    /// [`Config`] for use with `SdkBuilder`.
    pub fn to_config(&self, wallet: &ClientConfig) -> Config {
        let app = &self.inner.config;

        Config {
            api_key: Some(app.api_key.clone()),
            network: app.network,
            sync_interval_secs: app.sync_interval_secs,
            max_deposit_claim_fee: wallet
                .max_deposit_claim_fee
                .clone()
                .or_else(|| app.max_deposit_claim_fee.clone()),
            lnurl_domain: app.lnurl_domain.clone(),
            prefer_spark_over_lightning: wallet.prefer_spark.unwrap_or(app.prefer_spark),
            external_input_parsers: app.external_input_parsers.clone(),
            use_default_external_input_parsers: app.use_default_external_input_parsers,
            real_time_sync_server_url: app.real_time_sync_server_url.clone(),
            private_enabled_default: wallet.private_mode.unwrap_or(app.private_mode),
            optimization_config: wallet
                .optimization
                .clone()
                .unwrap_or_else(|| app.optimization.clone()),
        }
    }

    /// Derive a storage directory for a wallet based on its seed fingerprint.
    ///
    /// If `ClientConfig.storage_dir` is `Some`, uses that directly.
    /// Otherwise, derives a deterministic subdirectory under `app.storage_root`.
    pub fn derive_storage_dir(&self, wallet: &ClientConfig) -> Result<String, SdkError> {
        if let Some(ref dir) = wallet.storage_dir {
            return Ok(dir.clone());
        }

        let seed_bytes = wallet.seed.to_bytes()?;
        let hash = sha256_bytes(&seed_bytes);
        let wallet_id = hex::encode(&hash[..8]);
        Ok(format!("{}/{}", self.inner.config.storage_root, wallet_id))
    }

    /// Connect a wallet using this app's configuration.
    ///
    /// Merges wallet-level overrides with app-level defaults, auto-derives the
    /// storage directory from the seed fingerprint, and initializes the wallet.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let app = App::new(AppConfig {
    ///     api_key: "brz_test_...".into(),
    ///     network: Network::Mainnet,
    ///     ..Default::default()
    /// })?;
    ///
    /// let wallet = app.connect_wallet(ClientConfig {
    ///     seed: Seed::Mnemonic { mnemonic: "word1 word2 ...".into(), passphrase: None },
    ///     ..Default::default()
    /// }).await?;
    /// ```
    pub async fn connect_wallet(
        &self,
        wallet_config: ClientConfig,
    ) -> Result<crate::sdk::BreezClient, SdkError> {
        let config = self.to_config(&wallet_config);
        let storage_dir = self.derive_storage_dir(&wallet_config)?;

        let builder = crate::SdkBuilder::new(config, wallet_config.seed)
            .with_default_storage(storage_dir);
        let sdk = builder.build().await?;
        Ok(sdk)
    }

    /// Single-step wallet connection for the common case.
    ///
    /// Creates an `App` internally, derives the storage directory, and connects
    /// the wallet in one call. Equivalent to:
    ///
    /// ```ignore
    /// let app = App::new(app_config)?;
    /// let wallet = app.connect_wallet(wallet_config).await?;
    /// ```
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let wallet = App::connect(ConnectConfig {
    ///     api_key: "brz_test_...".into(),
    ///     network: Network::Mainnet,
    ///     seed: Seed::Mnemonic { mnemonic: "...".into(), passphrase: None },
    ///     ..Default::default()
    /// }).await?;
    /// ```
    pub async fn connect(config: ConnectConfig) -> Result<crate::sdk::BreezClient, SdkError> {
        let (app_config, wallet_config) = config.into_parts();
        let app = Self::new(app_config)?;
        app.connect_wallet(wallet_config).await
    }
}

fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    use bitcoin::hashes::{Hash, sha256};
    let hash = sha256::Hash::hash(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(hash.as_ref());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Seed;

    #[test]
    fn test_app_new_minimal_mainnet() {
        let app = App::new(AppConfig {
            api_key: "brz_test_key".to_string(),
            network: Network::Mainnet,
            ..Default::default()
        })
        .expect("should succeed");

        let cfg = &app.inner.config;
        assert_eq!(cfg.api_key, "brz_test_key");
        assert_eq!(cfg.network, Network::Mainnet);
        assert_eq!(cfg.storage_root, "./.breez");
        assert_eq!(cfg.sync_interval_secs, 60);
        assert_eq!(
            cfg.max_deposit_claim_fee,
            Some(MaxFee::Rate { sat_per_vbyte: 1 })
        );
        assert_eq!(cfg.lnurl_domain, Some("breez.tips".to_string()));
        assert!(!cfg.prefer_spark);
        assert!(cfg.use_default_external_input_parsers);
        assert!(cfg.real_time_sync_server_url.is_some());
        assert!(cfg.private_mode);
        assert!(cfg.optimization.auto_enabled);
        assert_eq!(cfg.optimization.multiplicity, 1);
    }

    #[test]
    fn test_app_new_minimal_regtest() {
        let app = App::new(AppConfig {
            api_key: String::new(),
            network: Network::Regtest,
            ..Default::default()
        })
        .expect("regtest should allow empty api_key");

        assert_eq!(app.inner.config.lnurl_domain, None);
    }

    #[test]
    fn test_app_new_mainnet_requires_api_key() {
        let result = App::new(AppConfig {
            api_key: String::new(),
            network: Network::Mainnet,
            ..Default::default()
        });

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("api_key"), "error should mention api_key: {err}");
    }

    #[test]
    fn test_app_new_invalid_multiplicity() {
        let result = App::new(AppConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            optimization: Some(OptimizationConfig {
                auto_enabled: true,
                multiplicity: 10,
            }),
            ..Default::default()
        });

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("multiplicity"),
            "error should mention multiplicity: {err}"
        );
    }

    #[test]
    fn test_app_new_with_all_overrides() {
        let app = App::new(AppConfig {
            api_key: "my_key".to_string(),
            network: Network::Mainnet,
            storage_root: Some("/custom/path".to_string()),
            sync_interval_secs: Some(120),
            max_deposit_claim_fee: Some(MaxFee::Fixed { amount: 500 }),
            lnurl_domain: Some("custom.tips".to_string()),
            prefer_spark: Some(true),
            external_input_parsers: None,
            use_default_external_input_parsers: Some(false),
            real_time_sync_server_url: Some("https://custom.sync".to_string()),
            private_mode: Some(false),
            optimization: Some(OptimizationConfig {
                auto_enabled: false,
                multiplicity: 3,
            }),
        })
        .unwrap();

        let cfg = &app.inner.config;
        assert_eq!(cfg.storage_root, "/custom/path");
        assert_eq!(cfg.sync_interval_secs, 120);
        assert_eq!(cfg.max_deposit_claim_fee, Some(MaxFee::Fixed { amount: 500 }));
        assert_eq!(cfg.lnurl_domain, Some("custom.tips".to_string()));
        assert!(cfg.prefer_spark);
        assert!(!cfg.use_default_external_input_parsers);
        assert_eq!(
            cfg.real_time_sync_server_url,
            Some("https://custom.sync".to_string())
        );
        assert!(!cfg.private_mode);
        assert!(!cfg.optimization.auto_enabled);
        assert_eq!(cfg.optimization.multiplicity, 3);
    }

    #[test]
    fn test_to_config_merges_wallet_overrides() {
        let app = App::new(AppConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            ..Default::default()
        })
        .unwrap();

        let wallet = ClientConfig {
            seed: Seed::Entropy(vec![0u8; 32]),
            prefer_spark: Some(true),
            optimization: Some(OptimizationConfig {
                auto_enabled: false,
                multiplicity: 0,
            }),
            ..Default::default()
        };

        let config = app.to_config(&wallet);
        assert!(config.prefer_spark_over_lightning);
        assert!(!config.optimization_config.auto_enabled);
        assert_eq!(config.optimization_config.multiplicity, 0);
    }

    #[test]
    fn test_to_config_uses_app_defaults_when_no_wallet_override() {
        let app = App::new(AppConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            prefer_spark: Some(true),
            ..Default::default()
        })
        .unwrap();

        let wallet = ClientConfig {
            seed: Seed::Entropy(vec![0u8; 32]),
            ..Default::default()
        };

        let config = app.to_config(&wallet);
        assert!(config.prefer_spark_over_lightning);
        assert!(config.optimization_config.auto_enabled);
    }

    #[test]
    fn test_derive_storage_dir_explicit() {
        let app = App::new(AppConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            ..Default::default()
        })
        .unwrap();

        let wallet = ClientConfig {
            seed: Seed::Entropy(vec![0u8; 32]),
            storage_dir: Some("/explicit/path".to_string()),
            ..Default::default()
        };

        let dir = app.derive_storage_dir(&wallet).unwrap();
        assert_eq!(dir, "/explicit/path");
    }

    #[test]
    fn test_derive_storage_dir_auto() {
        let app = App::new(AppConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            storage_root: Some("/root".to_string()),
            ..Default::default()
        })
        .unwrap();

        let wallet = ClientConfig {
            seed: Seed::Entropy(vec![1u8; 32]),
            ..Default::default()
        };

        let dir = app.derive_storage_dir(&wallet).unwrap();
        assert!(dir.starts_with("/root/"));
        // Should be deterministic — same seed → same dir
        let dir2 = app.derive_storage_dir(&wallet).unwrap();
        assert_eq!(dir, dir2);
    }

    #[test]
    fn test_derive_storage_dir_different_seeds_differ() {
        let app = App::new(AppConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            ..Default::default()
        })
        .unwrap();

        let wallet1 = ClientConfig {
            seed: Seed::Entropy(vec![1u8; 32]),
            ..Default::default()
        };
        let wallet2 = ClientConfig {
            seed: Seed::Entropy(vec![2u8; 32]),
            ..Default::default()
        };

        let dir1 = app.derive_storage_dir(&wallet1).unwrap();
        let dir2 = app.derive_storage_dir(&wallet2).unwrap();
        assert_ne!(dir1, dir2);
    }

    #[test]
    fn test_app_is_clone() {
        let app = App::new(AppConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            ..Default::default()
        })
        .unwrap();

        let app2 = app.clone();
        // Both point to the same inner data (Arc)
        assert!(Arc::ptr_eq(&app.inner, &app2.inner));
    }

    #[test]
    fn test_defaults_match_legacy_default_config() {
        let app = App::new(AppConfig {
            api_key: "key".to_string(),
            network: Network::Mainnet,
            ..Default::default()
        })
        .unwrap();

        let wallet = ClientConfig {
            seed: Seed::Entropy(vec![0u8; 32]),
            storage_dir: Some("test".to_string()),
            ..Default::default()
        };

        let new_config = app.to_config(&wallet);
        let legacy_config = crate::default_config(Network::Mainnet);

        // All defaults should match
        assert_eq!(new_config.network, legacy_config.network);
        assert_eq!(new_config.sync_interval_secs, legacy_config.sync_interval_secs);
        assert_eq!(new_config.max_deposit_claim_fee, legacy_config.max_deposit_claim_fee);
        assert_eq!(new_config.lnurl_domain, legacy_config.lnurl_domain);
        assert_eq!(
            new_config.prefer_spark_over_lightning,
            legacy_config.prefer_spark_over_lightning
        );
        assert_eq!(
            new_config.use_default_external_input_parsers,
            legacy_config.use_default_external_input_parsers
        );
        assert_eq!(
            new_config.real_time_sync_server_url,
            legacy_config.real_time_sync_server_url
        );
        assert_eq!(
            new_config.private_enabled_default,
            legacy_config.private_enabled_default
        );
        assert_eq!(
            new_config.optimization_config.auto_enabled,
            legacy_config.optimization_config.auto_enabled
        );
        assert_eq!(
            new_config.optimization_config.multiplicity,
            legacy_config.optimization_config.multiplicity
        );
    }
}
