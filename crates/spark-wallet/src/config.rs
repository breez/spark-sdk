use spark::{operator::OperatorPool, Network};

pub struct SparkWalletConfig {
    pub network: Network,
    pub operator_pool: OperatorPool,
}