use std::{str::FromStr, time::Duration};

use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use spark::{
    Network,
    operator::{OperatorConfig, OperatorPoolConfig},
    ssp::{RetryConfig, ServiceProviderConfig},
    token::TokensConfig,
    tree::LeafOptimizationOptions,
};

use crate::SparkWalletError;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkWalletConfig {
    pub network: Network,
    pub operator_pool: OperatorPoolConfig,
    pub reconnect_interval_seconds: u64,
    pub service_provider_config: ServiceProviderConfig,
    pub split_secret_threshold: u32,
    pub tokens_config: TokensConfig,
    pub leaf_optimization_options: LeafOptimizationOptions,
    pub leaf_auto_optimize_enabled: bool,
    pub token_outputs_optimization_options: TokenOutputsOptimizationOptions,
    pub self_payment_allowed: bool,
    /// Maximum number of concurrent transfer claims.
    ///
    /// Controls how many pending Spark transfers can be claimed in parallel.
    /// Default is 1 (sequential claiming). Increase for server environments
    /// with high incoming payment volume to improve throughput.
    pub max_concurrent_claims: u32,
}

impl SparkWalletConfig {
    pub fn validate(&self) -> Result<(), SparkWalletError> {
        if self.split_secret_threshold > self.operator_pool.get_all_operators().count() as u32 {
            return Err(SparkWalletError::ValidationError(
                "split_secret_threshold must be less than or equal to the number of signing operators".to_string(),
            ));
        }

        self.leaf_optimization_options
            .validate()
            .map_err(|e| SparkWalletError::ValidationError(e.to_string()))?;

        self.token_outputs_optimization_options.validate()?;

        Ok(())
    }

    pub fn default_config(network: Network) -> Self {
        match network {
            Network::Mainnet => Self {
                network,
                operator_pool: Self::default_operator_pool_config(network),
                reconnect_interval_seconds: 1,
                service_provider_config: Self::create_service_provider_config(
                    "https://api.lightspark.com",
                    "023e33e2920326f64ea31058d44777442d97d7d5cbfcf54e3060bc1695e5261c93",
                    None,
                )
                .unwrap(),
                split_secret_threshold: 2,
                tokens_config: Self::default_tokens_config(),
                leaf_optimization_options: LeafOptimizationOptions::default(),
                leaf_auto_optimize_enabled: true,
                token_outputs_optimization_options: TokenOutputsOptimizationOptions {
                    min_outputs_threshold: 50,
                    target_output_count: 5,
                    auto_optimize_interval: Some(Duration::from_secs(60 * 2)),
                },
                self_payment_allowed: false,
                max_concurrent_claims: 1,
            },
            _ => Self {
                network,
                operator_pool: Self::default_operator_pool_config(network),
                reconnect_interval_seconds: 1,
                service_provider_config: Self::create_service_provider_config(
                    "https://api.lightspark.com",
                    "022bf283544b16c0622daecb79422007d167eca6ce9f0c98c0c49833b1f7170bfe",
                    Some("graphql/spark/rc".to_string()),
                )
                .unwrap(),
                split_secret_threshold: 2,
                tokens_config: Self::default_tokens_config(),
                leaf_optimization_options: LeafOptimizationOptions::default(),
                leaf_auto_optimize_enabled: true,
                token_outputs_optimization_options: TokenOutputsOptimizationOptions {
                    min_outputs_threshold: 50,
                    target_output_count: 5,
                    auto_optimize_interval: Some(Duration::from_secs(60 * 2)),
                },
                self_payment_allowed: false,
                max_concurrent_claims: 1,
            },
        }
    }

    pub fn default_operator_pool_config(_: Network) -> OperatorPoolConfig {
        let operators = vec![
            Self::create_operator_config(
                0,
                "0000000000000000000000000000000000000000000000000000000000000001",
                "https://0.spark.lightspark.com",
                None,
                "03dfbdff4b6332c220f8fa2ba8ed496c698ceada563fa01b67d9983bfc5c95e763",
            )
            .unwrap(),
            Self::create_operator_config(
                1,
                "0000000000000000000000000000000000000000000000000000000000000002",
                "https://spark-operator.breez.technology",
                None,
                "03e625e9768651c9be268e287245cc33f96a68ce9141b0b4769205db027ee8ed77",
            )
            .unwrap(),
            Self::create_operator_config(
                2,
                "0000000000000000000000000000000000000000000000000000000000000003",
                "https://2.spark.flashnet.xyz",
                None,
                "022eda13465a59205413086130a65dc0ed1b8f8e51937043161f8be0c369b1a410",
            )
            .unwrap(),
        ];
        OperatorPoolConfig::new(0, operators).unwrap()
    }

    pub fn create_service_provider_config(
        url: &str,
        pubkey: &str,
        schema_endpoint: Option<String>,
    ) -> Result<ServiceProviderConfig, SparkWalletError> {
        Ok(ServiceProviderConfig {
            base_url: url.to_string(),
            schema_endpoint,
            identity_public_key: PublicKey::from_str(pubkey).map_err(|_| {
                SparkWalletError::ValidationError("Invalid identity public key".to_string())
            })?,
            user_agent: None,
            retry_config: RetryConfig::default(),
        })
    }

    pub fn create_operator_config(
        id: usize,
        identifier: &str,
        address: &str,
        ca_cert: Option<&[u8]>,
        identity_public_key: &str,
    ) -> Result<OperatorConfig, SparkWalletError> {
        Ok(OperatorConfig {
            id,
            identifier: frost_secp256k1_tr::Identifier::deserialize(
                &hex::decode(identifier).map_err(|_| {
                    SparkWalletError::ValidationError("Invalid identifier".to_string())
                })?,
            )
            .map_err(|_| SparkWalletError::ValidationError("Invalid identifier".to_string()))?,
            address: address
                .parse()
                .map_err(|_| SparkWalletError::ValidationError("Invalid address".to_string()))?,
            ca_cert: ca_cert.map(|cert| cert.to_vec()),
            identity_public_key: PublicKey::from_str(identity_public_key).map_err(|_| {
                SparkWalletError::ValidationError("Invalid identity public key".to_string())
            })?,
            user_agent: None,
        })
    }

    pub fn default_tokens_config() -> TokensConfig {
        TokensConfig {
            expected_withdraw_bond_sats: 10_000,
            expected_withdraw_relative_block_locktime: 1_000,
            transaction_validity_duration_seconds: 180,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenOutputsOptimizationOptions {
    /// Auto-consolidation fires for a token when its available output count
    /// strictly exceeds this threshold.
    pub min_outputs_threshold: u32,
    /// Number of outputs to produce when consolidation runs. The summed input
    /// amount is split into this many roughly-equal outputs (any remainder
    /// becomes a change output to the same self-address). Must be >= 1 and
    /// strictly less than `min_outputs_threshold` so consolidation can
    /// converge below the trigger.
    pub target_output_count: u32,
    pub auto_optimize_interval: Option<Duration>,
}

impl TokenOutputsOptimizationOptions {
    pub fn validate(&self) -> Result<(), SparkWalletError> {
        if self.min_outputs_threshold <= 1 {
            return Err(SparkWalletError::ValidationError(
                "min_outputs_threshold must be greater than 1".to_string(),
            ));
        }
        if self.target_output_count < 1 {
            return Err(SparkWalletError::ValidationError(
                "target_output_count must be at least 1".to_string(),
            ));
        }
        if self.target_output_count >= self.min_outputs_threshold {
            return Err(SparkWalletError::ValidationError(
                "target_output_count must be strictly less than min_outputs_threshold".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(
        min_outputs_threshold: u32,
        target_output_count: u32,
    ) -> TokenOutputsOptimizationOptions {
        TokenOutputsOptimizationOptions {
            min_outputs_threshold,
            target_output_count,
            auto_optimize_interval: None,
        }
    }

    #[test]
    fn default_config_is_valid() {
        SparkWalletConfig::default_config(Network::Mainnet)
            .validate()
            .expect("mainnet default must be valid");
        SparkWalletConfig::default_config(Network::Regtest)
            .validate()
            .expect("regtest default must be valid");
    }

    #[test]
    fn rejects_target_equal_to_threshold() {
        let err = opts(5, 5).validate().unwrap_err();
        assert!(
            matches!(err, SparkWalletError::ValidationError(_)),
            "expected ValidationError, got {err:?}"
        );
    }

    #[test]
    fn rejects_target_above_threshold() {
        opts(5, 6)
            .validate()
            .expect_err("target_output_count >= min_outputs_threshold must fail");
    }

    #[test]
    fn rejects_target_zero() {
        opts(50, 0)
            .validate()
            .expect_err("target_output_count of 0 must fail");
    }

    #[test]
    fn rejects_threshold_le_one() {
        opts(1, 1)
            .validate()
            .expect_err("min_outputs_threshold <= 1 must fail");
    }

    #[test]
    fn accepts_target_below_threshold() {
        opts(50, 5).validate().expect("5 < 50 must pass");
        opts(3, 1).validate().expect("1 < 3 must pass");
    }
}
