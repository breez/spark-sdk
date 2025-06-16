mod deposit;

pub use deposit::{DepositAddress, DepositService};

mod spark {
    tonic::include_proto!("spark");
}

impl From<crate::Network> for spark::Network {
    fn from(network: crate::Network) -> Self {
        match network {
            crate::Network::Mainnet => spark::Network::Mainnet,
            crate::Network::Regtest => spark::Network::Regtest,
            crate::Network::Testnet => spark::Network::Testnet,
            crate::Network::Signet => spark::Network::Signet,
        }
    }
}
