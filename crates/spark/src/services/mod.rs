mod deposit;

pub use deposit::{DepositAddress, DepositService, DepositServiceError};

impl From<crate::Network> for spark_protos::spark::Network {
    fn from(network: crate::Network) -> Self {
        match network {
            crate::Network::Mainnet => spark_protos::spark::Network::Mainnet,
            crate::Network::Regtest => spark_protos::spark::Network::Regtest,
            crate::Network::Testnet => spark_protos::spark::Network::Testnet,
            crate::Network::Signet => spark_protos::spark::Network::Signet,
        }
    }
}
