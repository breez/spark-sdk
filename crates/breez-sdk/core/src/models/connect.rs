use crate::{ConnectOptions, Network, SdkCredentials, Seed, error::SdkError, models::Config};

impl ConnectOptions {
    /// Applies these options on top of a base config, returning the merged config.
    #[allow(deprecated)]
    pub fn apply_to_config(&self, mut config: Config) -> Config {
        if let Some(ref domain) = self.lnurl_domain {
            config.lnurl_domain = Some(domain.clone());
        }
        if let Some(secs) = self.sync_interval_secs {
            config.sync_interval_secs = secs;
        }
        if let Some(ref fee) = self.max_deposit_claim_fee {
            config.max_deposit_claim_fee = Some(fee.clone());
        }
        if let Some(prefer) = self.prefer_spark_over_lightning {
            config.prefer_spark_over_lightning = prefer;
        }
        if let Some(private) = self.private_mode {
            config.private_enabled_default = private;
        }
        if let Some(ref opt_config) = self.optimization_config {
            config.optimization_config = opt_config.clone();
        }
        if let Some(ref parsers) = self.external_input_parsers {
            config.external_input_parsers = Some(parsers.clone());
        }
        if let Some(use_default) = self.use_default_external_input_parsers {
            config.use_default_external_input_parsers = use_default;
        }
        if let Some(ref url) = self.real_time_sync_server_url {
            config.real_time_sync_server_url = Some(url.clone());
        }
        if let Some(ref sbc) = self.stable_balance_config {
            config.stable_balance_config = Some(sbc.clone());
        }
        config
    }
}

impl SdkCredentials {
    /// Builds a `Config` and `Seed` from mnemonic credentials + options.
    ///
    /// Returns an error if called on the `Signer` variant (use the signer path instead).
    #[allow(deprecated)]
    pub fn to_config_and_seed(&self, options: &ConnectOptions) -> Result<(Config, Seed), SdkError> {
        let network = options.network.unwrap_or(Network::Mainnet);
        let mut config = crate::default_config(network);
        config.api_key = Some(self.api_key().to_string());
        config = options.apply_to_config(config);

        let seed = match self {
            SdkCredentials::Mnemonic {
                mnemonic,
                passphrase,
                ..
            } => Seed::Mnemonic {
                mnemonic: mnemonic.clone(),
                passphrase: passphrase.clone(),
            },
            #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
            SdkCredentials::Signer { .. } => {
                return Err(SdkError::InvalidInput(
                    "Cannot extract seed from Signer credentials. Use the signer connect path instead.".to_string(),
                ));
            }
        };
        Ok((config, seed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(deprecated)]
    fn test_connect_options_defaults_match_default_config() {
        let opts = ConnectOptions::default();
        let base_config = crate::default_config(Network::Mainnet);
        let result = opts.apply_to_config(base_config.clone());

        // All None options should leave the config unchanged
        assert_eq!(result.sync_interval_secs, base_config.sync_interval_secs);
        assert_eq!(result.lnurl_domain, base_config.lnurl_domain);
        assert_eq!(
            result.prefer_spark_over_lightning,
            base_config.prefer_spark_over_lightning
        );
        assert_eq!(
            result.private_enabled_default,
            base_config.private_enabled_default
        );
        assert_eq!(
            result.use_default_external_input_parsers,
            base_config.use_default_external_input_parsers
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_connect_options_apply_overrides() {
        let opts = ConnectOptions {
            lnurl_domain: Some("custom.domain".to_string()),
            sync_interval_secs: Some(120),
            prefer_spark_over_lightning: Some(true),
            private_mode: Some(false),
            ..Default::default()
        };
        let base_config = crate::default_config(Network::Mainnet);
        let result = opts.apply_to_config(base_config);

        assert_eq!(result.lnurl_domain, Some("custom.domain".to_string()));
        assert_eq!(result.sync_interval_secs, 120);
        assert!(result.prefer_spark_over_lightning);
        assert!(!result.private_enabled_default);
    }

    #[test]
    fn test_sdk_credentials_api_key() {
        let creds = SdkCredentials::Mnemonic {
            api_key: "test-key".to_string(),
            mnemonic: "test words".to_string(),
            passphrase: None,
        };
        assert_eq!(creds.api_key(), "test-key");
    }

    #[test]
    fn test_sdk_credentials_to_config_and_seed() {
        let creds = SdkCredentials::Mnemonic {
            api_key: "my-api-key".to_string(),
            mnemonic: "test mnemonic".to_string(),
            passphrase: Some("pass".to_string()),
        };
        let opts = ConnectOptions {
            network: Some(Network::Regtest),
            ..Default::default()
        };

        let (config, seed) = creds.to_config_and_seed(&opts).unwrap();
        assert_eq!(config.api_key, Some("my-api-key".to_string()));
        assert!(matches!(config.network, Network::Regtest));
        assert!(matches!(
            seed,
            Seed::Mnemonic {
                mnemonic,
                passphrase: Some(p)
            } if mnemonic == "test mnemonic" && p == "pass"
        ));
    }

    #[test]
    fn test_sdk_credentials_debug_redacts() {
        let creds = SdkCredentials::Mnemonic {
            api_key: "secret-key-123".to_string(),
            mnemonic: "abandon abandon abandon".to_string(),
            passphrase: None,
        };
        let debug = format!("{creds:?}");
        // Should NOT contain the actual key or mnemonic
        assert!(!debug.contains("secret-key-123"));
        assert!(!debug.contains("abandon"));
        // Should contain the redacted markers
        assert!(debug.contains("<redacted>"));
    }
}
