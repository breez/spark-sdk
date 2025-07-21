pub mod error;

use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

use crate::operator::rpc::spark::{
    PaymentIntentFields as ProtoPaymentIntentFields, SparkAddress as ProtoSparkAddress,
};
use bitcoin::{
    bech32::{self, Bech32m, Hrp},
    secp256k1::PublicKey,
};
use prost::Message;

use error::AddressError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Network;

const HRP_MAINNET: Hrp = Hrp::parse_unchecked("sp");
const HRP_TESTNET: Hrp = Hrp::parse_unchecked("spt");
const HRP_REGTEST: Hrp = Hrp::parse_unchecked("sprt");
const HRP_SIGNET: Hrp = Hrp::parse_unchecked("sps");

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SparkAddress {
    pub identity_public_key: PublicKey,
    pub network: Network,
    payment_intent: Option<PaymentIntentFields>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct PaymentIntentFields {
    pub id: Uuid,
    pub asset_identifier: Option<AssetIdentifier>,
    pub asset_amount: u64,
    pub memo: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetIdentifier(Vec<u8>);
impl std::fmt::Display for AssetIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}

impl FromStr for AssetIdentifier {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s).map_err(|_| {
            AddressError::InvalidPaymentIntent("Invalid asset identifier".to_string())
        })?;
        Ok(AssetIdentifier(bytes))
    }
}

impl PaymentIntentFields {
    pub fn new(
        asset_amount: u64,
        asset_identifier: Option<AssetIdentifier>,
        memo: Option<String>,
    ) -> Self {
        PaymentIntentFields {
            id: uuid::Uuid::now_v7(),
            asset_identifier,
            asset_amount,
            memo,
        }
    }
}

impl TryFrom<ProtoPaymentIntentFields> for PaymentIntentFields {
    type Error = AddressError;

    fn try_from(proto: ProtoPaymentIntentFields) -> Result<Self, Self::Error> {
        Ok(PaymentIntentFields {
            id: uuid::Uuid::from_bytes(proto.id.try_into().map_err(|_| {
                AddressError::InvalidPaymentIntent("Invalid UUID length".to_string())
            })?),
            asset_identifier: proto.asset_identifier.map(AssetIdentifier),
            asset_amount: u128::from_be_bytes(proto.asset_amount.try_into().map_err(|_| {
                AddressError::InvalidPaymentIntent("Invalid asset amount length".to_string())
            })?) as u64,
            memo: proto.memo,
        })
    }
}

impl From<&PaymentIntentFields> for ProtoPaymentIntentFields {
    fn from(val: &PaymentIntentFields) -> Self {
        let id = val.id.as_bytes().to_vec();
        let asset_identifier = val.asset_identifier.as_ref().map(|id| id.0.clone());

        ProtoPaymentIntentFields {
            id,
            asset_identifier,
            asset_amount: u128::to_be_bytes(val.asset_amount as u128).to_vec(),
            memo: val.memo.clone(),
        }
    }
}

impl SparkAddress {
    pub fn new(
        identity_public_key: PublicKey,
        network: Network,
        payment_intent: Option<PaymentIntentFields>,
    ) -> Self {
        SparkAddress {
            identity_public_key,
            network,
            payment_intent,
        }
    }

    fn network_to_hrp(network: &Network) -> Hrp {
        match network {
            Network::Mainnet => HRP_MAINNET,
            Network::Testnet => HRP_TESTNET,
            Network::Regtest => HRP_REGTEST,
            Network::Signet => HRP_SIGNET,
        }
    }

    fn hrp_to_network(hrp: &Hrp) -> Result<Network, AddressError> {
        match hrp {
            hrp if hrp == &HRP_MAINNET => Ok(Network::Mainnet),
            hrp if hrp == &HRP_TESTNET => Ok(Network::Testnet),
            hrp if hrp == &HRP_REGTEST => Ok(Network::Regtest),
            hrp if hrp == &HRP_SIGNET => Ok(Network::Signet),
            _ => Err(AddressError::UnknownHrp(hrp.to_string())),
        }
    }
}

impl Display for SparkAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let payment_intent_fields = self
            .payment_intent
            .as_ref()
            .map(|payment_intent| payment_intent.into());
        let proto_address = ProtoSparkAddress {
            identity_public_key: self.identity_public_key.serialize().to_vec(),
            payment_intent_fields,
        };

        let payload_bytes = proto_address.encode_to_vec();

        let hrp = Self::network_to_hrp(&self.network);

        // This is safe to unwrap, because we are using a valid HRP and payload
        let address = bech32::encode::<Bech32m>(hrp, &payload_bytes).unwrap();
        write!(f, "{address}")
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

        let network = Self::hrp_to_network(&hrp)?;

        let payment_intent = if let Some(fields) = proto_address.payment_intent_fields {
            Some(fields.try_into()?)
        } else {
            None
        };

        Ok(SparkAddress::new(
            identity_public_key,
            network,
            payment_intent,
        ))
    }
}

impl Debug for SparkAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl Serialize for SparkAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let address_string = self.to_string();
        serializer.serialize_str(&address_string)
    }
}

impl<'de> Deserialize<'de> for SparkAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let address_string = String::deserialize(deserializer)?;
        SparkAddress::from_str(&address_string).map_err(serde::de::Error::custom)
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
        let original_address = SparkAddress::new(public_key, Network::Mainnet, None);

        let address_string = original_address.to_string();
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
        let original_address = SparkAddress::new(public_key, Network::Testnet, None);

        let address_string = original_address.to_string();
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
        let original_address = SparkAddress::new(public_key, Network::Regtest, None);

        let address_string = original_address.to_string();
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
        let original_address = SparkAddress::new(public_key, Network::Signet, None);

        let address_string = original_address.to_string();
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

    #[test]
    fn test_payment_intent_address_roundtrip() {
        let public_key = create_test_public_key();
        let payment_intent = PaymentIntentFields {
            id: uuid::Uuid::now_v7(),
            asset_identifier: Some(AssetIdentifier(
                "1234567890abcdef1234567890abcdef".as_bytes().to_vec(),
            )),
            asset_amount: 1000000,
            memo: Some("Test payment".to_string()),
        };

        let original_address =
            SparkAddress::new(public_key, Network::Mainnet, Some(payment_intent.clone()));

        let address_string = original_address.to_string();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);

        // Check payment intent fields
        assert!(parsed_address.payment_intent.is_some());
        let parsed_payment_intent = parsed_address.payment_intent.unwrap();
        let original_payment_intent = original_address.payment_intent.unwrap();

        assert_eq!(parsed_payment_intent.id, original_payment_intent.id);
        assert_eq!(
            parsed_payment_intent.asset_identifier,
            original_payment_intent.asset_identifier
        );
        assert_eq!(
            parsed_payment_intent.asset_amount,
            original_payment_intent.asset_amount
        );
        assert_eq!(parsed_payment_intent.memo, original_payment_intent.memo);
    }

    #[test]
    fn test_payment_intent_minimal_data() {
        let public_key = create_test_public_key();
        let payment_intent = PaymentIntentFields {
            id: uuid::Uuid::now_v7(),
            asset_identifier: None,
            asset_amount: 500,
            memo: None,
        };

        let original_address =
            SparkAddress::new(public_key, Network::Testnet, Some(payment_intent));

        let address_string = original_address.to_string();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert!(parsed_address.payment_intent.is_some());
        let parsed_payment_intent = parsed_address.payment_intent.unwrap();

        assert_eq!(parsed_payment_intent.asset_identifier, None);
        assert_eq!(parsed_payment_intent.asset_amount, 500);
        assert_eq!(parsed_payment_intent.memo, None);
    }

    #[test]
    fn test_compare_addresses_with_and_without_payment_intent() {
        let public_key = create_test_public_key();

        // Create address without payment intent
        let address_without_intent = SparkAddress::new(public_key, Network::Mainnet, None);
        let string_without_intent = address_without_intent.to_string();

        // Create address with payment intent
        let payment_intent = PaymentIntentFields {
            id: uuid::Uuid::now_v7(),
            asset_identifier: Some(AssetIdentifier("abcdef1234567890".as_bytes().to_vec())),
            asset_amount: 1000,
            memo: Some("Test memo".to_string()),
        };
        let address_with_intent =
            SparkAddress::new(public_key, Network::Mainnet, Some(payment_intent));
        let string_with_intent = address_with_intent.to_string();

        // The strings should be different due to the payment intent data
        assert_ne!(string_without_intent, string_with_intent);

        // Parse both addresses and verify the core data matches
        let parsed_without_intent = SparkAddress::from_str(&string_without_intent).unwrap();
        let parsed_with_intent = SparkAddress::from_str(&string_with_intent).unwrap();

        assert_eq!(
            parsed_without_intent.identity_public_key,
            parsed_with_intent.identity_public_key
        );
        assert_eq!(parsed_without_intent.network, parsed_with_intent.network);
        assert!(parsed_without_intent.payment_intent.is_none());
        assert!(parsed_with_intent.payment_intent.is_some());
    }

    #[test]
    fn test_invalid_payment_intent_data() {
        let public_key = create_test_public_key();

        // Try to create invalid asset identifier
        let proto_fields = ProtoPaymentIntentFields {
            id: vec![1, 2, 3], // Too short to be a valid UUID
            asset_identifier: None,
            asset_amount: u128::to_be_bytes(1000u128).to_vec(),
            memo: Some("Test".to_string()),
        };

        let proto_address = ProtoSparkAddress {
            identity_public_key: public_key.serialize().to_vec(),
            payment_intent_fields: Some(proto_fields.clone()),
        };

        let payload_bytes = proto_address.encode_to_vec();
        let address = bech32::encode::<Bech32m>(Hrp::parse("sp").unwrap(), &payload_bytes).unwrap();

        // Parsing should work but then fail when converting the proto payment intent
        let result = SparkAddress::from_str(&address);
        assert!(result.is_err());
    }
}
