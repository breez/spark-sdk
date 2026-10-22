use lightning::bolt11_invoice::Currency;

use crate::network::BitcoinNetwork;

/// A Lightning network in the `lnbtc` CAIP-2 namespace, whose reference is the
/// first 32 hex characters of the Bitcoin network's genesis block hash.
///
/// The specification defines mainnet and testnet3. Signet and regtest follow
/// the same convention but are not in it, so they only interoperate with
/// servers that use the same identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LnbtcNetwork {
    Mainnet,
    Testnet3,
    Signet,
    Regtest,
}

impl LnbtcNetwork {
    pub fn id(self) -> &'static str {
        match self {
            Self::Mainnet => "lnbtc:000000000019d6689c085ae165831e93",
            Self::Testnet3 => "lnbtc:000000000933ea01ad0ee984209779ba",
            Self::Signet => "lnbtc:00000008819873e925422c1ff0f99f7c",
            Self::Regtest => "lnbtc:0f9188f13cb7b2c71f2a335e3a4fc328",
        }
    }

    pub fn currency(self) -> Currency {
        match self {
            Self::Mainnet => Currency::Bitcoin,
            Self::Testnet3 => Currency::BitcoinTestnet,
            Self::Signet => Currency::Signet,
            Self::Regtest => Currency::Regtest,
        }
    }

    /// Testnet4 has no identifier: its invoices share testnet3's `tb` prefix,
    /// so the two could not be told apart.
    pub fn from_bitcoin_network(network: BitcoinNetwork) -> Option<Self> {
        match network {
            BitcoinNetwork::Bitcoin => Some(Self::Mainnet),
            BitcoinNetwork::Testnet3 => Some(Self::Testnet3),
            BitcoinNetwork::Testnet4 => None,
            BitcoinNetwork::Signet => Some(Self::Signet),
            BitcoinNetwork::Regtest => Some(Self::Regtest),
        }
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::constants::genesis_block;

    use super::*;

    #[macros::test_all]
    fn ids_are_genesis_hash_prefixes() {
        let cases = [
            (LnbtcNetwork::Mainnet, bitcoin::Network::Bitcoin),
            (LnbtcNetwork::Testnet3, bitcoin::Network::Testnet),
            (LnbtcNetwork::Signet, bitcoin::Network::Signet),
            (LnbtcNetwork::Regtest, bitcoin::Network::Regtest),
        ];
        for (lnbtc, network) in cases {
            let genesis = genesis_block(network).block_hash().to_string();
            assert_eq!(lnbtc.id(), format!("lnbtc:{}", &genesis[..32]));
        }
    }
}
