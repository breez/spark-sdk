use std::str::FromStr;

use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use spark::{
    Network,
    operator::{OperatorConfig, OperatorError, OperatorPoolConfig},
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
                service_provider_config: Self::create_service_provier_config(
                    "https://api.lightspark.com",
                    "023e33e2920326f64ea31058d44777442d97d7d5cbfcf54e3060bc1695e5261c93",
                )
                .unwrap(),
                split_secret_threshold: 2,
            },
            Network::Regtest => Self {
                network,
                operator_pool: Self::default_operator_pool_config(network),
                reconnect_interval_seconds: 1,
                service_provider_config: Self::create_service_provier_config(
                    "https://api.lightspark.com",
                    "023e33e2920326f64ea31058d44777442d97d7d5cbfcf54e3060bc1695e5261c93",
                )
                .unwrap(),
                split_secret_threshold: 2,
            },
            Network::Testnet => Self {
                network,
                operator_pool: Self::default_operator_pool_config(network),
                reconnect_interval_seconds: 1,
                service_provider_config: Self::create_service_provier_config(
                    "https://api.lightspark.com",
                    "023e33e2920326f64ea31058d44777442d97d7d5cbfcf54e3060bc1695e5261c93",
                )
                .unwrap(),
                split_secret_threshold: 2,
            },
            Network::Signet => Self {
                network,
                operator_pool: Self::default_operator_pool_config(network),
                reconnect_interval_seconds: 1,
                service_provider_config: Self::create_service_provier_config(
                    "https://api.lightspark.com",
                    "023e33e2920326f64ea31058d44777442d97d7d5cbfcf54e3060bc1695e5261c93",
                )
                .unwrap(),
                split_secret_threshold: 2,
            },
        }
    }

    pub fn default_operator_pool_config(network: Network) -> OperatorPoolConfig {
        let operators = vec![
            Self::create_opeartor_config(
                0,
                "0000000000000000000000000000000000000000000000000000000000000001",
                "https://0.spark.loadtest.dev.sparkinfra.net/",
                "03d8d2d331e07f572636dfd371a30dfa139a8bdc99ea98f1f48e27dcc664589ecc",
            )
            .unwrap(),
            Self::create_opeartor_config(
                1,
                "0000000000000000000000000000000000000000000000000000000000000002",
                "https://1.spark.loadtest.dev.sparkinfra.net/",
                "023b1f3e062137ffc541a8edeaab7a4648aafa506d0208956123507d66d3886ac6",
            )
            .unwrap(),
            Self::create_opeartor_config(
                2,
                "0000000000000000000000000000000000000000000000000000000000000003",
                "https://2.spark.loadtest.dev.sparkinfra.net/",
                "02a2c62aa3230d9a51759b3d67399f57223455656369d28120fb39ef062b4469c8",
            )
            .unwrap(),
        ];
        OperatorPoolConfig::new(0, operators).unwrap()
    }

    pub fn create_service_provier_config(
        url: &str,
        pubkey: &str,
    ) -> Result<ServiceProviderConfig, SparkWalletError> {
        Ok(ServiceProviderConfig {
            base_url: url.to_string(),
            schema_endpoint: None,
            identity_public_key: PublicKey::from_str(pubkey).map_err(|_| {
                SparkWalletError::ValidationError("Invalid identity public key".to_string())
            })?,
        })
    }

    pub fn create_opeartor_config(
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
            identity_public_key: PublicKey::from_str(&identity_public_key).map_err(|_| {
                SparkWalletError::ValidationError("Invalid identity public key".to_string())
            })?,
        })
    }
}
