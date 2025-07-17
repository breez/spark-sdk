use std::time::Duration;

use serde::{Deserialize, Serialize};
use spark::{Network, operator::OperatorPoolConfig, ssp::ServiceProviderConfig};

use crate::SparkWalletError;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkWalletConfig {
    pub network: Network,
    pub operator_pool: OperatorPoolConfig,
    pub reconnect_interval: Duration,
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
}
