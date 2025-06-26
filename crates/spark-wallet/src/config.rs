use spark::{Network, operator::OperatorPool, ssp::ServiceProviderConfig};

pub struct SparkWalletConfig {
    pub network: Network,
    pub operator_pool: OperatorPool,
    pub service_provider_config: ServiceProviderConfig,
}
