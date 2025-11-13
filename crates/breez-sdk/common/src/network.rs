use serde::{Deserialize, Serialize};
use spark::Network;
use strum::Display;

#[derive(Clone, Copy, Debug, Display, Eq, PartialEq, Serialize, Deserialize)]
pub enum BitcoinNetwork {
    /// Mainnet
    Bitcoin,
    Testnet3,
    Testnet4,
    Signet,
    Regtest,
}

impl From<bitcoin::Network> for BitcoinNetwork {
    fn from(network: bitcoin::Network) -> Self {
        match network {
            bitcoin::Network::Testnet => BitcoinNetwork::Testnet3,
            bitcoin::Network::Testnet4 => BitcoinNetwork::Testnet4,
            bitcoin::Network::Signet => BitcoinNetwork::Signet,
            bitcoin::Network::Regtest => BitcoinNetwork::Regtest,
            bitcoin::Network::Bitcoin => BitcoinNetwork::Bitcoin,
        }
    }
}

impl From<BitcoinNetwork> for bitcoin::Network {
    fn from(network: BitcoinNetwork) -> Self {
        match network {
            BitcoinNetwork::Bitcoin => bitcoin::Network::Bitcoin,
            BitcoinNetwork::Testnet3 => bitcoin::Network::Testnet,
            BitcoinNetwork::Testnet4 => bitcoin::Network::Testnet4,
            BitcoinNetwork::Signet => bitcoin::Network::Signet,
            BitcoinNetwork::Regtest => bitcoin::Network::Regtest,
        }
    }
}

impl From<Network> for BitcoinNetwork {
    fn from(network: Network) -> Self {
        match network {
            spark_wallet::Network::Mainnet => BitcoinNetwork::Bitcoin,
            spark_wallet::Network::Testnet => BitcoinNetwork::Testnet3,
            spark_wallet::Network::Regtest => BitcoinNetwork::Regtest,
            spark_wallet::Network::Signet => BitcoinNetwork::Signet,
        }
    }
}
