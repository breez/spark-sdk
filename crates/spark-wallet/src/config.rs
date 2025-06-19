use spark::{Network, operator::OperatorPool};

pub struct SparkWalletConfig {
    pub network: Network,
    pub operator_pool: OperatorPool,
}
