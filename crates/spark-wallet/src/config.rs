use std::str::FromStr;

use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use spark::{
    Network,
    operator::{OperatorConfig, OperatorPoolConfig},
    services::TokensConfig,
    ssp::ServiceProviderConfig,
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
}

impl SparkWalletConfig {
    pub fn validate(&self) -> Result<(), SparkWalletError> {
        if self.split_secret_threshold > self.operator_pool.get_all_operators().count() as u32 {
            return Err(SparkWalletError::ValidationError(
                "split_secret_threshold must be less than or equal to the number of signing operators".to_string(),
            ));
        }

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
            },
        }
    }

    pub fn default_operator_pool_config(network: Network) -> OperatorPoolConfig {
        match network {
            Network::Mainnet => {
                let operators = vec![
                    Self::create_operator_config(
                        0,
                        "0000000000000000000000000000000000000000000000000000000000000001",
                        "https://0.spark.lightspark.com",
                        "03dfbdff4b6332c220f8fa2ba8ed496c698ceada563fa01b67d9983bfc5c95e763",
                    )
                    .unwrap(),
                    Self::create_operator_config(
                        1,
                        "0000000000000000000000000000000000000000000000000000000000000002",
                        "https://1.spark.lightspark.com",
                        "03e625e9768651c9be268e287245cc33f96a68ce9141b0b4769205db027ee8ed77",
                    )
                    .unwrap(),
                    Self::create_operator_config(
                        2,
                        "0000000000000000000000000000000000000000000000000000000000000003",
                        "https://2.spark.flashnet.xyz",
                        "022eda13465a59205413086130a65dc0ed1b8f8e51937043161f8be0c369b1a410",
                    )
                    .unwrap(),
                ];
                OperatorPoolConfig::new(0, operators).unwrap()
            }
            _ => {
                let operators = vec![
                    Self::create_operator_config(
                        0,
                        "0000000000000000000000000000000000000000000000000000000000000001",
                        "https://0.spark.lightspark.com",
                        "03dfbdff4b6332c220f8fa2ba8ed496c698ceada563fa01b67d9983bfc5c95e763",
                    )
                    .unwrap(),
                    Self::create_operator_config(
                        1,
                        "0000000000000000000000000000000000000000000000000000000000000002",
                        "https://1.spark.lightspark.com",
                        "03e625e9768651c9be268e287245cc33f96a68ce9141b0b4769205db027ee8ed77",
                    )
                    .unwrap(),
                    Self::create_operator_config(
                        2,
                        "0000000000000000000000000000000000000000000000000000000000000003",
                        "https://2.spark.flashnet.xyz",
                        "022eda13465a59205413086130a65dc0ed1b8f8e51937043161f8be0c369b1a410",
                    )
                    .unwrap(),
                ];
                OperatorPoolConfig::new(0, operators).unwrap()
            }
        }
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
        })
    }

    pub fn create_operator_config(
        id: usize,
        identifier: &str,
        address: &str,
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
            identity_public_key: PublicKey::from_str(identity_public_key).map_err(|_| {
                SparkWalletError::ValidationError("Invalid identity public key".to_string())
            })?,
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
