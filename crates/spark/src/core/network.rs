use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use bitcoin::params::Params;
use serde::{Deserialize, Serialize};

use crate::operator::rpc as operator_rpc;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
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

impl Display for Network {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Network::Mainnet => write!(f, "mainnet"),
            Network::Regtest => write!(f, "regtest"),
            Network::Testnet => write!(f, "testnet"),
            Network::Signet => write!(f, "signet"),
        }
    }
}

impl FromStr for Network {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mainnet" => Ok(Network::Mainnet),
            "regtest" => Ok(Network::Regtest),
            "testnet" => Ok(Network::Testnet),
            "signet" => Ok(Network::Signet),
            _ => Err("Invalid network".to_string()),
        }
    }
}

impl Network {
    pub(crate) fn to_proto_network(self) -> operator_rpc::spark::Network {
        match self {
            Network::Mainnet => operator_rpc::spark::Network::Mainnet,
            Network::Regtest => operator_rpc::spark::Network::Regtest,
            Network::Testnet => operator_rpc::spark::Network::Testnet,
            Network::Signet => operator_rpc::spark::Network::Signet,
        }
    }

    pub(crate) fn from_proto_network(network_num: i32) -> Result<Self, String> {
        let network: operator_rpc::spark::Network =
            network_num.try_into().map_err(|_| "Invalid network")?;
        match network {
            operator_rpc::spark::Network::Mainnet => Ok(Network::Mainnet),
            operator_rpc::spark::Network::Regtest => Ok(Network::Regtest),
            operator_rpc::spark::Network::Testnet => Ok(Network::Testnet),
            operator_rpc::spark::Network::Signet => Ok(Network::Signet),
            _ => Err("Invalid network".to_string()),
        }
    }
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

impl From<Network> for bitcoin::NetworkKind {
    fn from(network: Network) -> Self {
        match network {
            Network::Mainnet => bitcoin::NetworkKind::Main,
            _ => bitcoin::NetworkKind::Test,
        }
    }
}

impl From<Network> for Params {
    fn from(value: Network) -> Self {
        let network: bitcoin::Network = value.into();
        network.into()
    }
}
