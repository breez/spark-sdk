use serde::{Deserialize, Serialize};
use spark::{Network, operator::OperatorPool, ssp::ServiceProviderConfig};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkWalletConfig {
    pub network: Network,
    pub operator_pool: OperatorPool,
    pub service_provider_config: ServiceProviderConfig,
    pub split_secret_threshold: u32,
}
