use std::{
    fmt::{self, Write},
    str::FromStr,
};

use bitcoin::{
    bech32::{
        Bech32m, Fe32, Hrp,
        primitives::{
            decode::CheckedHrpstring,
            iter::{ByteIterExt, Fe32IterExt},
        },
    },
    secp256k1::{PublicKey, constants::PUBLIC_KEY_SIZE},
};

use crate::Network;

const HRP_MAINNET: Hrp = Hrp::parse_unchecked("sp");
const HRP_TESTNET: Hrp = Hrp::parse_unchecked("tsp");
const HRP_REGTEST: Hrp = Hrp::parse_unchecked("sprt");

/// A version 0 BIP-352 silent payment address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SilentPaymentAddress {
    pub scan_key: PublicKey,
    /// The receiver's spend key, including any label tweak.
    pub spend_key: PublicKey,
    prefix: Prefix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Prefix {
    Mainnet,
    Testnet,
    Regtest,
}

impl Prefix {
    fn hrp(self) -> Hrp {
        match self {
            Prefix::Mainnet => HRP_MAINNET,
            Prefix::Testnet => HRP_TESTNET,
            Prefix::Regtest => HRP_REGTEST,
        }
    }

    fn from_hrp(hrp: Hrp) -> Option<Self> {
        [Prefix::Mainnet, Prefix::Testnet, Prefix::Regtest]
            .into_iter()
            .find(|prefix| prefix.hrp() == hrp)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SilentPaymentAddressError {
    #[error("not a valid bech32m string: {0}")]
    Encoding(String),
    #[error("unknown silent payment address prefix: {0}")]
    UnknownPrefix(String),
    #[error("only version 0 silent payment addresses are supported")]
    UnsupportedVersion,
    #[error("a silent payment address has 66 bytes of keys, this one has {0}")]
    InvalidLength(usize),
    #[error("the address contains a value that is not a public key")]
    InvalidKey,
    #[error("the address is not in its canonical encoding")]
    NonCanonical,
}

impl SilentPaymentAddress {
    pub fn new(scan_key: PublicKey, spend_key: PublicKey, network: Network) -> Self {
        let prefix = match network {
            Network::Mainnet => Prefix::Mainnet,
            Network::Testnet | Network::Signet => Prefix::Testnet,
            Network::Regtest => Prefix::Regtest,
        };
        Self {
            scan_key,
            spend_key,
            prefix,
        }
    }

    /// Testnet and signet addresses share one prefix, so an address can be
    /// valid on more than one network.
    pub fn is_valid_for_network(&self, network: Network) -> bool {
        match network {
            Network::Mainnet => self.prefix == Prefix::Mainnet,
            Network::Testnet | Network::Signet => self.prefix == Prefix::Testnet,
            Network::Regtest => self.prefix == Prefix::Regtest,
        }
    }
}

impl FromStr for SilentPaymentAddress {
    type Err = SilentPaymentAddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut checked = CheckedHrpstring::new::<Bech32m>(s)
            .map_err(|e| SilentPaymentAddressError::Encoding(e.to_string()))?;
        let prefix = Prefix::from_hrp(checked.hrp())
            .ok_or_else(|| SilentPaymentAddressError::UnknownPrefix(checked.hrp().to_string()))?;
        if checked.data_part_ascii_no_checksum().is_empty() {
            return Err(SilentPaymentAddressError::InvalidLength(0));
        }
        if checked.remove_witness_version() != Some(Fe32::Q) {
            return Err(SilentPaymentAddressError::UnsupportedVersion);
        }

        let payload: Vec<u8> = checked.byte_iter().collect();
        if payload.len() != 2 * PUBLIC_KEY_SIZE {
            return Err(SilentPaymentAddressError::InvalidLength(payload.len()));
        }
        let (scan_key, spend_key) = payload.split_at(PUBLIC_KEY_SIZE);
        let address = Self {
            scan_key: PublicKey::from_slice(scan_key)
                .map_err(|_| SilentPaymentAddressError::InvalidKey)?,
            spend_key: PublicKey::from_slice(spend_key)
                .map_err(|_| SilentPaymentAddressError::InvalidKey)?,
            prefix,
        };

        // Decoding drops the padding bits below the last byte, so an address
        // with non-zero padding would decode to the same keys.
        if address.to_string() != s.to_ascii_lowercase() {
            return Err(SilentPaymentAddressError::NonCanonical);
        }
        Ok(address)
    }
}

impl fmt::Display for SilentPaymentAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut payload = [0u8; 2 * PUBLIC_KEY_SIZE];
        payload[..PUBLIC_KEY_SIZE].copy_from_slice(&self.scan_key.serialize());
        payload[PUBLIC_KEY_SIZE..].copy_from_slice(&self.spend_key.serialize());
        let hrp = self.prefix.hrp();
        let encoded = payload
            .iter()
            .copied()
            .bytes_to_fes()
            .with_checksum::<Bech32m>(&hrp)
            .with_witness_version(Fe32::Q)
            .chars();
        for c in encoded {
            f.write_char(c)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use macros::test_all;

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// From the BIP-352 test vectors.
    const VECTOR_ADDRESS: &str = "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv";

    fn keys() -> (PublicKey, PublicKey) {
        let secp = Secp256k1::new();
        let key = |seed: u8| {
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[seed; 32]).unwrap())
        };
        (key(1), key(2))
    }

    fn encode(hrp: &str, version: Fe32, payload: &[u8]) -> String {
        payload
            .iter()
            .copied()
            .bytes_to_fes()
            .with_checksum::<Bech32m>(&Hrp::parse(hrp).unwrap())
            .with_witness_version(version)
            .chars()
            .collect()
    }

    fn key_bytes() -> Vec<u8> {
        let (scan_key, spend_key) = keys();
        [scan_key.serialize(), spend_key.serialize()].concat()
    }

    #[test_all]
    fn test_decode_vector() {
        let address: SilentPaymentAddress = VECTOR_ADDRESS.parse().unwrap();
        assert!(address.is_valid_for_network(Network::Mainnet));
        assert_eq!(address.to_string(), VECTOR_ADDRESS);
    }

    #[test_all]
    fn test_uppercase() {
        let address: SilentPaymentAddress = VECTOR_ADDRESS.to_uppercase().parse().unwrap();
        assert_eq!(address.to_string(), VECTOR_ADDRESS);
    }

    #[test_all]
    fn test_mixed_case() {
        let mixed = format!("SP{}", &VECTOR_ADDRESS[2..]);
        assert!(matches!(
            mixed.parse::<SilentPaymentAddress>(),
            Err(SilentPaymentAddressError::Encoding(_))
        ));
    }

    #[test_all]
    fn test_networks() {
        let (scan_key, spend_key) = keys();
        let cases = [
            ("sp", Network::Mainnet, [true, false, false, false]),
            ("tsp", Network::Testnet, [false, true, true, false]),
            ("tsp", Network::Signet, [false, true, true, false]),
            ("sprt", Network::Regtest, [false, false, false, true]),
        ];
        let networks = [
            Network::Mainnet,
            Network::Testnet,
            Network::Signet,
            Network::Regtest,
        ];
        for (hrp, network, valid) in cases {
            let encoded = encode(hrp, Fe32::Q, &key_bytes());
            let address: SilentPaymentAddress = encoded.parse().unwrap();
            assert_eq!(
                address,
                SilentPaymentAddress::new(scan_key, spend_key, network)
            );
            assert_eq!(address.to_string(), encoded);
            for (network, valid) in networks.iter().zip(valid) {
                assert_eq!(
                    address.is_valid_for_network(*network),
                    valid,
                    "{hrp} {network}"
                );
            }
        }
    }

    #[test_all]
    fn test_unknown_prefix() {
        assert!(matches!(
            encode("bc", Fe32::Q, &key_bytes()).parse::<SilentPaymentAddress>(),
            Err(SilentPaymentAddressError::UnknownPrefix(_))
        ));
    }

    #[test_all]
    fn test_versions() {
        let mut longer = key_bytes();
        longer.push(0);
        for (version, payload) in [
            (Fe32::P, key_bytes()),
            (Fe32::P, longer),
            (Fe32::L, key_bytes()),
        ] {
            assert_eq!(
                encode("sp", version, &payload).parse::<SilentPaymentAddress>(),
                Err(SilentPaymentAddressError::UnsupportedVersion),
                "version {}",
                version.to_u8()
            );
        }
    }

    #[test_all]
    fn test_lengths() {
        let keys = key_bytes();
        let mut longer = keys.clone();
        longer.push(0);
        for payload in [&keys[..65], &longer[..], &[]] {
            assert_eq!(
                encode("sp", Fe32::Q, payload).parse::<SilentPaymentAddress>(),
                Err(SilentPaymentAddressError::InvalidLength(payload.len())),
            );
        }
    }

    #[test_all]
    fn test_invalid_key() {
        let mut payload = key_bytes();
        // No point on the curve has an x coordinate above the field size.
        payload[34..].fill(0xff);
        assert_eq!(
            encode("sp", Fe32::Q, &payload).parse::<SilentPaymentAddress>(),
            Err(SilentPaymentAddressError::InvalidKey)
        );
    }

    #[test_all]
    fn test_non_zero_padding() {
        let mut fes: Vec<Fe32> = key_bytes().into_iter().bytes_to_fes().collect();
        let last = fes.pop().unwrap();
        fes.push(Fe32::try_from(last.to_u8() | 1).unwrap());
        let encoded: String = fes
            .into_iter()
            .with_checksum::<Bech32m>(&HRP_MAINNET)
            .with_witness_version(Fe32::Q)
            .chars()
            .collect();
        assert_eq!(
            encoded.parse::<SilentPaymentAddress>(),
            Err(SilentPaymentAddressError::NonCanonical)
        );
    }
}
