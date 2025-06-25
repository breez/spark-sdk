use spark::{operator::OperatorPool, ssp::ServiceProviderConfig, Network};

pub struct SparkWalletConfig {
    pub network: Network,
    pub operator_pool: OperatorPool,
    pub service_provider_config: ServiceProviderConfig,
}
