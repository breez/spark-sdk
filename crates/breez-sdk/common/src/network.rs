use serde::{Deserialize, Serialize};
use strum::Display;

#[derive(Clone, Copy, Debug, Display, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
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
            bitcoin::Network::Bitcoin => BitcoinNetwork::Bitcoin,
            bitcoin::Network::Testnet => BitcoinNetwork::Testnet3,
            bitcoin::Network::Testnet4 => BitcoinNetwork::Testnet4,
            bitcoin::Network::Signet => BitcoinNetwork::Signet,
            bitcoin::Network::Regtest => BitcoinNetwork::Regtest,
            _ => BitcoinNetwork::Bitcoin, // Default to Bitcoin for other networks
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
