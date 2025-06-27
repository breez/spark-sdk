use bitcoin::params::Params;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Network {
    #[serde(rename = "mainnet")]
    Mainnet,
    #[serde(rename = "regtest")]
    Regtest,
    #[serde(rename = "testnet")]
    Testnet,
    #[serde(rename = "signet")]
    Signet,
}

impl From<Network> for bitcoin::Network {
    fn from(network: Network) -> Self {
        match network {
            Network::Mainnet => bitcoin::Network::Bitcoin,
            Network::Regtest => bitcoin::Network::Regtest,
            Network::Testnet => bitcoin::Network::Testnet,
            Network::Signet => bitcoin::Network::Signet,
        }
    }
}

impl TryFrom<bitcoin::Network> for Network {
    type Error = String;

    fn try_from(value: bitcoin::Network) -> Result<Self, Self::Error> {
        match value {
            bitcoin::Network::Bitcoin => Ok(Network::Mainnet),
            bitcoin::Network::Regtest => Ok(Network::Regtest),
            bitcoin::Network::Testnet => Ok(Network::Testnet),
            bitcoin::Network::Signet => Ok(Network::Signet),
            _ => Err("Unsupported Bitcoin network".to_string()),
        }
    }
}

impl From<Network> for Params {
    fn from(value: Network) -> Self {
        let network: bitcoin::Network = value.into();
        network.into()
    }
}
