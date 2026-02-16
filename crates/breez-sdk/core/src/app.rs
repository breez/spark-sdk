use crate::{
    LeafOptimizationConfig, MaxFee, Network, SdkError,
    models::{ClientConfig, Config},
    sdk::BREEZ_SYNC_SERVICE_URL,
};

/// Breez SDK entry point.
///
/// Provides a static [`connect`](Self::connect) method that takes a
/// [`ClientConfig`] and returns a connected [`BreezClient`](crate::BreezClient).
///
/// # Examples
///
/// ```ignore
/// let client = Breez::connect(ClientConfig {
///     api_key: "brz_test_...".into(),
///     network: Network::Mainnet,
///     seed: Seed::Mnemonic { mnemonic: "...".into(), passphrase: None },
///     ..Default::default()
/// }).await?;
/// ```
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct Breez;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl Breez {
    /// Connect to the Breez SDK.
    ///
    /// Validates the configuration, resolves defaults, auto-derives the
    /// storage directory from the seed fingerprint (if not provided),
    /// and initializes the client.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let client = Breez::connect(ClientConfig {
    ///     api_key: "brz_test_...".into(),
    ///     network: Network::Mainnet,
    ///     seed: Seed::Mnemonic { mnemonic: "...".into(), passphrase: None },
    ///     ..Default::default()
    /// }).await?;
    /// ```
    pub async fn connect(
        client_config: ClientConfig,
    ) -> Result<crate::sdk::BreezClient, SdkError> {
        let config = resolve_config(&client_config)?;
        let storage_dir = derive_storage_dir(&client_config)?;

        let builder = crate::SdkBuilder::new(config, client_config.seed)
            .with_default_storage(storage_dir);
        let sdk = builder.build().await?;
        Ok(sdk)
    }
}

impl Breez {
    /// Create an [`SdkBuilder`](crate::SdkBuilder) from a [`ClientConfig`].
    ///
    /// Use this when you need to customize low-level components (storage,
    /// chain service, fiat service, LNURL client, payment observer, key set)
    /// before connecting.
    ///
    /// The returned builder has the resolved [`Config`](crate::models::Config)
    /// and default storage directory already configured. You can override
    /// individual components via the builder's fluent methods before calling
    /// `.build().await`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let client = Breez::builder(ClientConfig {
    ///     api_key: "brz_test_...".into(),
    ///     network: Network::Mainnet,
    ///     seed: Seed::Mnemonic { mnemonic: "...".into(), passphrase: None },
    ///     ..Default::default()
    /// })?
    /// .with_storage(my_custom_storage)
    /// .build()
    /// .await?;
    /// ```
    pub fn builder(client_config: ClientConfig) -> Result<crate::SdkBuilder, SdkError> {
        let config = resolve_config(&client_config)?;
        let storage_dir = derive_storage_dir(&client_config)?;

        Ok(crate::SdkBuilder::new(config, client_config.seed)
            .with_default_storage(storage_dir))
    }
}

/// Resolve a [`ClientConfig`] into a legacy [`Config`] for use with `SdkBuilder`.
///
/// Validates required fields and applies sensible defaults for any `None` values,
/// matching the legacy `default_config()` behavior.
pub fn resolve_config(config: &ClientConfig) -> Result<Config, SdkError> {
    // Validate
    if config.network == Network::Mainnet && config.api_key.is_empty() {
        return Err(SdkError::InvalidInput(
            "api_key is required for mainnet".to_string(),
        ));
    }

    if let Some(ref opt) = config.leaf_optimization_config {
        if opt.multiplicity > 5 {
            return Err(SdkError::InvalidInput(format!(
                "optimization multiplicity must be 0-5, got {}",
                opt.multiplicity
            )));
        }
    }

    let lnurl_domain = config.lnurl_domain.clone().or_else(|| match config.network {
        Network::Mainnet => Some("breez.tips".to_string()),
        Network::Regtest => None,
    });

    let max_deposit_claim_fee = config
        .max_deposit_claim_fee
        .clone()
        .map_or(Some(MaxFee::Rate { sat_per_vbyte: 1 }), Some);

    let real_time_sync_server_url = config
        .real_time_sync_server_url
        .clone()
        .or_else(|| Some(BREEZ_SYNC_SERVICE_URL.to_string()));

    Ok(Config {
        api_key: Some(config.api_key.clone()),
        network: config.network,
        sync_interval_secs: config.sync_interval_secs.unwrap_or(60),
        max_deposit_claim_fee,
        lnurl_domain,
        prefer_spark_over_lightning: config.prefer_spark_over_lightning.unwrap_or(false),
        external_input_parsers: config.external_input_parsers.clone(),
        use_default_external_input_parsers: config
            .use_default_external_input_parsers
            .unwrap_or(true),
        real_time_sync_server_url,
        private_enabled_default: config.private_mode.unwrap_or(true),
        optimization_config: config
            .leaf_optimization_config
            .clone()
            .unwrap_or(LeafOptimizationConfig {
                auto_enabled: true,
                multiplicity: 1,
            }),
    })
}

/// Derive a storage directory for a client based on its seed fingerprint.
///
/// If `ClientConfig.storage_dir` is `Some`, uses that directly.
/// Otherwise, derives a deterministic subdirectory under `storage_root`.
pub fn derive_storage_dir(config: &ClientConfig) -> Result<String, SdkError> {
    if let Some(ref dir) = config.storage_dir {
        return Ok(dir.clone());
    }

    let storage_root = config
        .storage_root
        .as_deref()
        .unwrap_or("./.breez");

    let seed_bytes = config.seed.to_bytes()?;
    let hash = sha256_bytes(&seed_bytes);
    let client_id = hex::encode(&hash[..8]);
    Ok(format!("{storage_root}/{client_id}"))
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
    fn test_resolve_config_minimal_mainnet() {
        let config = resolve_config(&ClientConfig {
            api_key: "brz_test_key".to_string(),
            network: Network::Mainnet,
            seed: Seed::Entropy(vec![0u8; 32]),
            ..Default::default()
        })
        .expect("should succeed");

        assert_eq!(config.api_key, Some("brz_test_key".to_string()));
        assert_eq!(config.network, Network::Mainnet);
        assert_eq!(config.sync_interval_secs, 60);
        assert_eq!(
            config.max_deposit_claim_fee,
            Some(MaxFee::Rate { sat_per_vbyte: 1 })
        );
        assert_eq!(config.lnurl_domain, Some("breez.tips".to_string()));
        assert!(!config.prefer_spark_over_lightning);
        assert!(config.use_default_external_input_parsers);
        assert!(config.real_time_sync_server_url.is_some());
        assert!(config.private_enabled_default);
        assert!(config.optimization_config.auto_enabled);
        assert_eq!(config.optimization_config.multiplicity, 1);
    }

    #[test]
    fn test_resolve_config_minimal_regtest() {
        let config = resolve_config(&ClientConfig {
            api_key: String::new(),
            network: Network::Regtest,
            seed: Seed::Entropy(vec![0u8; 32]),
            ..Default::default()
        })
        .expect("regtest should allow empty api_key");

        assert_eq!(config.lnurl_domain, None);
    }

    #[test]
    fn test_resolve_config_mainnet_requires_api_key() {
        let result = resolve_config(&ClientConfig {
            api_key: String::new(),
            network: Network::Mainnet,
            seed: Seed::Entropy(vec![0u8; 32]),
            ..Default::default()
        });

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("api_key"), "error should mention api_key: {err}");
    }

    #[test]
    fn test_resolve_config_invalid_multiplicity() {
        let result = resolve_config(&ClientConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            seed: Seed::Entropy(vec![0u8; 32]),
            leaf_optimization_config: Some(LeafOptimizationConfig {
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
    fn test_resolve_config_with_all_overrides() {
        let config = resolve_config(&ClientConfig {
            api_key: "my_key".to_string(),
            network: Network::Mainnet,
            seed: Seed::Entropy(vec![0u8; 32]),
            storage_root: Some("/custom/path".to_string()),
            sync_interval_secs: Some(120),
            max_deposit_claim_fee: Some(MaxFee::Fixed { amount: 500 }),
            lnurl_domain: Some("custom.tips".to_string()),
            prefer_spark_over_lightning: Some(true),
            external_input_parsers: None,
            use_default_external_input_parsers: Some(false),
            real_time_sync_server_url: Some("https://custom.sync".to_string()),
            private_mode: Some(false),
            leaf_optimization_config: Some(LeafOptimizationConfig {
                auto_enabled: false,
                multiplicity: 3,
            }),
            ..Default::default()
        })
        .unwrap();

        assert_eq!(config.sync_interval_secs, 120);
        assert_eq!(config.max_deposit_claim_fee, Some(MaxFee::Fixed { amount: 500 }));
        assert_eq!(config.lnurl_domain, Some("custom.tips".to_string()));
        assert!(config.prefer_spark_over_lightning);
        assert!(!config.use_default_external_input_parsers);
        assert_eq!(
            config.real_time_sync_server_url,
            Some("https://custom.sync".to_string())
        );
        assert!(!config.private_enabled_default);
        assert!(!config.optimization_config.auto_enabled);
        assert_eq!(config.optimization_config.multiplicity, 3);
    }

    #[test]
    fn test_derive_storage_dir_explicit() {
        let config = ClientConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            seed: Seed::Entropy(vec![0u8; 32]),
            storage_dir: Some("/explicit/path".to_string()),
            ..Default::default()
        };

        let dir = derive_storage_dir(&config).unwrap();
        assert_eq!(dir, "/explicit/path");
    }

    #[test]
    fn test_derive_storage_dir_auto() {
        let config = ClientConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            seed: Seed::Entropy(vec![1u8; 32]),
            storage_root: Some("/root".to_string()),
            ..Default::default()
        };

        let dir = derive_storage_dir(&config).unwrap();
        assert!(dir.starts_with("/root/"));
        // Should be deterministic — same seed → same dir
        let dir2 = derive_storage_dir(&config).unwrap();
        assert_eq!(dir, dir2);
    }

    #[test]
    fn test_derive_storage_dir_different_seeds_differ() {
        let config1 = ClientConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            seed: Seed::Entropy(vec![1u8; 32]),
            ..Default::default()
        };
        let config2 = ClientConfig {
            api_key: "key".to_string(),
            network: Network::Regtest,
            seed: Seed::Entropy(vec![2u8; 32]),
            ..Default::default()
        };

        let dir1 = derive_storage_dir(&config1).unwrap();
        let dir2 = derive_storage_dir(&config2).unwrap();
        assert_ne!(dir1, dir2);
    }

    #[test]
    fn test_defaults_match_legacy_default_config() {
        let config = resolve_config(&ClientConfig {
            api_key: "key".to_string(),
            network: Network::Mainnet,
            seed: Seed::Entropy(vec![0u8; 32]),
            ..Default::default()
        })
        .unwrap();

        let legacy_config = crate::default_config(Network::Mainnet);

        // All defaults should match
        assert_eq!(config.network, legacy_config.network);
        assert_eq!(config.sync_interval_secs, legacy_config.sync_interval_secs);
        assert_eq!(config.max_deposit_claim_fee, legacy_config.max_deposit_claim_fee);
        assert_eq!(config.lnurl_domain, legacy_config.lnurl_domain);
        assert_eq!(
            config.prefer_spark_over_lightning,
            legacy_config.prefer_spark_over_lightning
        );
        assert_eq!(
            config.use_default_external_input_parsers,
            legacy_config.use_default_external_input_parsers
        );
        assert_eq!(
            config.real_time_sync_server_url,
            legacy_config.real_time_sync_server_url
        );
        assert_eq!(
            config.private_enabled_default,
            legacy_config.private_enabled_default
        );
        assert_eq!(
            config.optimization_config.auto_enabled,
            legacy_config.optimization_config.auto_enabled
        );
        assert_eq!(
            config.optimization_config.multiplicity,
            legacy_config.optimization_config.multiplicity
        );
    }
}
