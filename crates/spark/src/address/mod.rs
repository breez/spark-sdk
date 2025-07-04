pub mod error;

use std::str::FromStr;

use crate::operator::rpc::spark::SparkAddress as ProtoSparkAddress;
use bitcoin::{
    bech32::{self, Bech32m, Hrp},
    secp256k1::PublicKey,
};
use prost::Message;

use error::AddressError;

use crate::Network;

const HRP_MAINNET: &str = "sp";
const HRP_TESTNET: &str = "spt";
const HRP_REGTEST: &str = "sprt";
const HRP_SIGNET: &str = "sps";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SparkAddress {
    pub identity_public_key: PublicKey,
    pub network: Network,
}

impl SparkAddress {
    fn network_to_hrp(network: &Network) -> &'static str {
        match network {
            Network::Mainnet => HRP_MAINNET,
            Network::Testnet => HRP_TESTNET,
            Network::Regtest => HRP_REGTEST,
            Network::Signet => HRP_SIGNET,
        }
    }

    fn hrp_to_network(hrp: &str) -> Result<Network, AddressError> {
        match hrp {
            HRP_MAINNET => Ok(Network::Mainnet),
            HRP_TESTNET => Ok(Network::Testnet),
            HRP_REGTEST => Ok(Network::Regtest),
            HRP_SIGNET => Ok(Network::Signet),
            _ => Err(AddressError::UnknownHrp(hrp.to_string())),
        }
    }

    /// Convert to bech32m string representation
    pub fn to_address_string(&self) -> Result<String, AddressError> {
        let proto_address = ProtoSparkAddress {
            identity_public_key: self.identity_public_key.serialize().to_vec(),
            payment_intent_fields: None, // TODO: Add payment intent fields
        };

        let payload_bytes = proto_address.encode_to_vec();

        let hrp_str = Self::network_to_hrp(&self.network);
        let hrp = Hrp::parse(hrp_str)
            .map_err(|e| AddressError::Other(format!("Failed to parse HRP: {}", e)))?;

        let address = bech32::encode::<Bech32m>(hrp, &payload_bytes)
            .map_err(|e| AddressError::Bech32EncodeError(e.to_string()))?;

        Ok(address)
    }
}

impl FromStr for SparkAddress {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (hrp, payload_bytes) =
            bech32::decode(s).map_err(|_| AddressError::InvalidBech32mAddress(s.to_string()))?;

        let proto_address = ProtoSparkAddress::decode(&payload_bytes[..])
            .map_err(|e| AddressError::ProtobufDecodeError(e.to_string()))?;

        let identity_public_key = PublicKey::from_slice(&proto_address.identity_public_key)
            .map_err(|e| AddressError::InvalidPublicKey(e.to_string()))?;

        let network = Self::hrp_to_network(hrp.as_str())?;

        Ok(SparkAddress {
            identity_public_key,
            network,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Secp256k1;

    fn create_test_public_key() -> PublicKey {
        let secp = Secp256k1::new();
        let (secret_key, _) = secp.generate_keypair(&mut bitcoin::secp256k1::rand::thread_rng());
        PublicKey::from_slice(&secret_key.public_key(&secp).serialize()).unwrap()
    }

    #[test]
    fn test_address_roundtrip() {
        let public_key = create_test_public_key();
        let original_address = SparkAddress {
            identity_public_key: public_key,
            network: Network::Mainnet,
        };

        let address_string = original_address.to_address_string().unwrap();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);
    }

    #[test]
    fn test_address_roundtrip_testnet() {
        let public_key = create_test_public_key();
        let original_address = SparkAddress {
            identity_public_key: public_key,
            network: Network::Testnet,
        };

        let address_string = original_address.to_address_string().unwrap();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);
    }

    #[test]
    fn test_address_roundtrip_regtest() {
        let public_key = create_test_public_key();
        let original_address = SparkAddress {
            identity_public_key: public_key,
            network: Network::Regtest,
        };

        let address_string = original_address.to_address_string().unwrap();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);
    }

    #[test]
    fn test_address_roundtrip_signet() {
        let public_key = create_test_public_key();
        let original_address = SparkAddress {
            identity_public_key: public_key,
            network: Network::Signet,
        };

        let address_string = original_address.to_address_string().unwrap();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);
    }

    #[test]
    fn test_parse_specific_regtest_address() {
        let address_str = "sprt1pgssyuuuhnrrdjswal5c3s3rafw9w3y5dd4cjy3duxlf7hjzkp0rqx6dj6mrhu";
        let address = SparkAddress::from_str(address_str).unwrap();

        assert_eq!(address.network, Network::Regtest);
        assert_eq!(address.identity_public_key.serialize().len(), 33); // Compressed public key
    }

    #[test]
    fn test_invalid_bech32_address() {
        let result = SparkAddress::from_str("invalid-address");
        assert!(result.is_err());
        match result {
            Err(AddressError::InvalidBech32mAddress(_)) => {}
            _ => panic!("Expected InvalidBech32mAddress error"),
        }
    }

    #[test]
    fn test_unknown_hrp_address() {
        // Create a valid bech32m address but with an unknown HRP
        let public_key = create_test_public_key();
        let proto_address = ProtoSparkAddress {
            identity_public_key: public_key.serialize().to_vec(),
            payment_intent_fields: None,
        };
        let payload_bytes = proto_address.encode_to_vec();

        // Use an unknown HRP "spx" instead of valid ones
        let address =
            bech32::encode::<Bech32m>(Hrp::parse("spx").unwrap(), &payload_bytes).unwrap();

        let result = SparkAddress::from_str(&address);
        assert!(result.is_err());
        match result {
            Err(AddressError::UnknownHrp(hrp)) => {
                assert_eq!(hrp, "spx");
            }
            _ => panic!("Expected UnknownHrp error"),
        }
    }
}
