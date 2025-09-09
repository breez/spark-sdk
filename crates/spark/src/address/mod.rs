pub mod error;

use std::{
    fmt::{Debug, Display},
    str::FromStr,
    time::Duration,
};

use crate::operator::rpc::spark::{
    SatsPayment as ProtoSatsPayment, SparkAddress as ProtoSparkAddress,
    SparkInvoiceFields as ProtoSparkInvoiceFields, TokensPayment as ProtoTokensPayment,
    spark_invoice_fields::PaymentType as ProtoPaymentType,
};
use bitcoin::{
    bech32::{self, Bech32m, Hrp},
    secp256k1::PublicKey,
    secp256k1::ecdsa::Signature,
};

use prost::Message;

use error::AddressError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use web_time::{SystemTime, UNIX_EPOCH};

use crate::Network;

const HRP_MAINNET: Hrp = Hrp::parse_unchecked("sp");
const HRP_TESTNET: Hrp = Hrp::parse_unchecked("spt");
const HRP_REGTEST: Hrp = Hrp::parse_unchecked("sprt");
const HRP_SIGNET: Hrp = Hrp::parse_unchecked("sps");

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SparkAddress {
    pub identity_public_key: PublicKey,
    pub network: Network,
    pub spark_invoice_fields: Option<SparkInvoiceFields>,
    pub signature: Option<Signature>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SparkInvoiceFields {
    pub id: Uuid,
    pub version: u32,
    pub memo: Option<String>,
    pub sender_public_key: Option<PublicKey>,
    pub expiry_time: Option<SystemTime>,
    pub payment_type: Option<SparkAddressPaymentType>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SparkAddressPaymentType {
    TokensPayment(TokensPayment),
    SatsPayment(SatsPayment),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SatsPayment {
    pub amount: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TokensPayment {
    pub token_identifier: Option<AssetIdentifier>,
    pub amount: Option<u128>,
}

impl From<SparkAddressPaymentType> for ProtoPaymentType {
    fn from(value: SparkAddressPaymentType) -> Self {
        match value {
            SparkAddressPaymentType::TokensPayment(tp) => {
                ProtoPaymentType::TokensPayment(ProtoTokensPayment {
                    amount: tp.amount.map(|amount| amount.to_be_bytes().to_vec()),
                    token_identifier: tp.token_identifier.map(|id| id.0.to_vec()),
                })
            }
            SparkAddressPaymentType::SatsPayment(sp) => {
                ProtoPaymentType::SatsPayment(ProtoSatsPayment { amount: sp.amount })
            }
        }
    }
}

impl TryFrom<ProtoPaymentType> for SparkAddressPaymentType {
    type Error = AddressError;
    fn try_from(value: ProtoPaymentType) -> Result<Self, Self::Error> {
        match value {
            ProtoPaymentType::TokensPayment(tp) => {
                let amount = match tp.amount {
                    Some(amount) => {
                        let amount_bytes: [u8; 16] = amount.try_into().map_err(|_| {
                            AddressError::InvalidPaymentIntent("Invalid amount".to_string())
                        })?;
                        Some(u128::from_be_bytes(amount_bytes))
                    }
                    None => None,
                };

                Ok(SparkAddressPaymentType::TokensPayment(TokensPayment {
                    token_identifier: tp.token_identifier.map(AssetIdentifier),
                    amount,
                }))
            }
            ProtoPaymentType::SatsPayment(sp) => {
                Ok(SparkAddressPaymentType::SatsPayment(SatsPayment {
                    amount: sp.amount,
                }))
            }
        }
    }
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

impl TryFrom<ProtoSparkInvoiceFields> for SparkInvoiceFields {
    type Error = AddressError;

    fn try_from(proto: ProtoSparkInvoiceFields) -> Result<Self, Self::Error> {
        let sender_public_key = match proto.sender_public_key {
            Some(pk) => Some(
                PublicKey::from_slice(&pk)
                    .map_err(|e| AddressError::InvalidPublicKey(e.to_string()))?,
            ),
            None => None,
        };

        let payment_type = match proto.payment_type {
            Some(pt) => Some(pt.try_into().map_err(|_| {
                AddressError::InvalidPaymentIntent("Invalid payment type".to_string())
            })?),
            None => None,
        };

        Ok(SparkInvoiceFields {
            id: uuid::Uuid::from_bytes(proto.id.try_into().map_err(|_| {
                AddressError::InvalidPaymentIntent("Invalid UUID length".to_string())
            })?),
            version: proto.version,
            sender_public_key,
            expiry_time: proto.expiry_time.map(|t| {
                UNIX_EPOCH
                    + Duration::from_secs(t.seconds as u64)
                    + Duration::from_nanos(t.nanos as u64)
            }),
            payment_type,
            memo: proto.memo,
        })
    }
}

impl From<SparkInvoiceFields> for ProtoSparkInvoiceFields {
    fn from(val: SparkInvoiceFields) -> Self {
        let id = val.id.as_bytes().to_vec();

        let payment_type = val.payment_type.map(|pt| pt.into());

        ProtoSparkInvoiceFields {
            id,
            version: val.version,
            sender_public_key: val.sender_public_key.map(|pk| pk.serialize().to_vec()),
            expiry_time: val.expiry_time.map(|t| ::prost_types::Timestamp {
                seconds: t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64,
                nanos: t
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as i32,
            }),
            payment_type,
            memo: val.memo.clone(),
        }
    }
}

impl SparkAddress {
    pub fn new(
        identity_public_key: PublicKey,
        network: Network,
        spark_invoice_fields: Option<SparkInvoiceFields>,
        signature: Option<Signature>,
    ) -> Self {
        SparkAddress {
            identity_public_key,
            network,
            spark_invoice_fields,
            signature,
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
        let spark_invoice_fields: Option<ProtoSparkInvoiceFields> =
            self.spark_invoice_fields.clone().map(|f| f.into());

        let proto_address = ProtoSparkAddress {
            identity_public_key: self.identity_public_key.serialize().to_vec(),
            spark_invoice_fields,
            signature: None,
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

        let invoice_fields: Option<SparkInvoiceFields> = proto_address
            .spark_invoice_fields
            .map(|f| f.try_into())
            .transpose()?;

        let signature = proto_address
            .signature
            .map(|s| {
                Signature::from_compact(&s)
                    .map_err(|e| AddressError::InvalidSignature(e.to_string()))
            })
            .transpose()?;

        Ok(SparkAddress::new(
            identity_public_key,
            network,
            invoice_fields,
            signature,
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
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn create_test_public_key() -> PublicKey {
        let secp = Secp256k1::new();
        let (secret_key, _) = secp.generate_keypair(&mut bitcoin::secp256k1::rand::thread_rng());
        PublicKey::from_slice(&secret_key.public_key(&secp).serialize()).unwrap()
    }

    #[test_all]
    fn test_address_roundtrip() {
        let public_key = create_test_public_key();
        let original_address = SparkAddress::new(public_key, Network::Mainnet, None, None);

        let address_string = original_address.to_string();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);
    }

    #[test_all]
    fn test_address_roundtrip_testnet() {
        let public_key = create_test_public_key();
        let original_address = SparkAddress::new(public_key, Network::Testnet, None, None);

        let address_string = original_address.to_string();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);
    }

    #[test_all]
    fn test_address_roundtrip_regtest() {
        let public_key = create_test_public_key();
        let original_address = SparkAddress::new(public_key, Network::Regtest, None, None);

        let address_string = original_address.to_string();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);
    }

    #[test_all]
    fn test_address_roundtrip_signet() {
        let public_key = create_test_public_key();
        let original_address = SparkAddress::new(public_key, Network::Signet, None, None);

        let address_string = original_address.to_string();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);
    }

    #[test_all]
    fn test_parse_specific_regtest_address() {
        let address_str = "sprt1pgssyuuuhnrrdjswal5c3s3rafw9w3y5dd4cjy3duxlf7hjzkp0rqx6dj6mrhu";
        let address = SparkAddress::from_str(address_str).unwrap();

        assert_eq!(address.network, Network::Regtest);
        assert_eq!(address.identity_public_key.serialize().len(), 33); // Compressed public key
    }

    #[test_all]
    fn test_invalid_bech32_address() {
        let result = SparkAddress::from_str("invalid-address");
        assert!(result.is_err());
        match result {
            Err(AddressError::InvalidBech32mAddress(_)) => {}
            _ => panic!("Expected InvalidBech32mAddress error"),
        }
    }

    #[test_all]
    fn test_unknown_hrp_address() {
        // Create a valid bech32m address but with an unknown HRP
        let public_key = create_test_public_key();
        let proto_address = ProtoSparkAddress {
            identity_public_key: public_key.serialize().to_vec(),
            spark_invoice_fields: None,
            signature: None,
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

    #[test_all]
    fn test_invoice_fields_address_roundtrip() {
        let public_key = create_test_public_key();
        let sender_public_key = create_test_public_key();
        let invoice_fields = SparkInvoiceFields {
            id: uuid::Uuid::now_v7(),
            version: 1,
            sender_public_key: Some(sender_public_key),
            expiry_time: Some(SystemTime::now()),
            payment_type: Some(SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier: Some(AssetIdentifier(
                    "1234567890abcdef1234567890abcdef".as_bytes().to_vec(),
                )),
                amount: Some(100),
            })),
            memo: Some("Test payment".to_string()),
        };

        let original_address = SparkAddress::new(
            public_key,
            Network::Mainnet,
            Some(invoice_fields.clone()),
            None,
        );

        let address_string = original_address.to_string();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);

        // Check payment intent fields
        assert!(parsed_address.spark_invoice_fields.is_some());
        let parsed_invoice_fields = parsed_address.spark_invoice_fields.unwrap();
        let original_invoice_fields = original_address.spark_invoice_fields.unwrap();

        assert_eq!(parsed_invoice_fields.id, original_invoice_fields.id);
        assert_eq!(
            parsed_invoice_fields.expiry_time,
            original_invoice_fields.expiry_time
        );
        assert_eq!(parsed_invoice_fields.id, original_invoice_fields.id);
        assert_eq!(parsed_invoice_fields.memo, original_invoice_fields.memo);

        let Some(SparkAddressPaymentType::TokensPayment(tokens_payment1)) =
            parsed_invoice_fields.payment_type
        else {
            panic!("Expected TokensPayment");
        };
        let Some(SparkAddressPaymentType::TokensPayment(tokens_payment2)) =
            original_invoice_fields.payment_type
        else {
            panic!("Expected TokensPayment");
        };
        assert_eq!(
            tokens_payment1.token_identifier,
            tokens_payment2.token_identifier
        );
        assert_eq!(tokens_payment1.amount, tokens_payment2.amount);
    }

    #[test_all]
    fn test_invoice_fields_minimal_data() {
        let public_key = create_test_public_key();
        let invoice_fields = SparkInvoiceFields {
            id: uuid::Uuid::now_v7(),
            version: 1,
            sender_public_key: None,
            expiry_time: None,
            payment_type: Some(SparkAddressPaymentType::SatsPayment(SatsPayment {
                amount: Some(500),
            })),
            memo: None,
        };

        let original_address =
            SparkAddress::new(public_key, Network::Testnet, Some(invoice_fields), None);

        let address_string = original_address.to_string();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert!(parsed_address.spark_invoice_fields.is_some());
        let parsed_invoice_fields = parsed_address.spark_invoice_fields.unwrap();

        let Some(SparkAddressPaymentType::SatsPayment(sp)) = parsed_invoice_fields.payment_type
        else {
            panic!("Invalid payment type");
        };

        assert_eq!(sp.amount.unwrap(), 500);
        assert_eq!(parsed_invoice_fields.memo, None);
    }

    #[test_all]
    fn test_compare_addresses_with_and_without_invoice_fields() {
        let public_key = create_test_public_key();
        let sender_public_key = create_test_public_key();

        // Create address without invoice fields
        let address_without_intent = SparkAddress::new(public_key, Network::Mainnet, None, None);
        let string_without_intent = address_without_intent.to_string();

        let invoice_fields = SparkInvoiceFields {
            id: uuid::Uuid::now_v7(),
            version: 1,
            sender_public_key: Some(sender_public_key),
            expiry_time: Some(SystemTime::now()),
            payment_type: Some(SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier: Some(AssetIdentifier("abcdef1234567890".as_bytes().to_vec())),
                amount: Some(100),
            })),
            memo: Some("Test memo".to_string()),
        };
        let address_with_intent =
            SparkAddress::new(public_key, Network::Mainnet, Some(invoice_fields), None);
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
        assert!(parsed_without_intent.spark_invoice_fields.is_none());
        assert!(parsed_with_intent.spark_invoice_fields.is_some());
    }

    #[test_all]
    fn test_invalid_invoice_fields_data() {
        let public_key = create_test_public_key();

        // Try to create invalid asset identifier
        let proto_fields = ProtoSparkInvoiceFields {
            id: vec![1, 2, 3], // Too short to be a valid UUID
            version: 1,
            sender_public_key: None,
            expiry_time: None,
            payment_type: Some(ProtoPaymentType::TokensPayment(ProtoTokensPayment {
                token_identifier: None,
                amount: Some(u128::to_be_bytes(1000u128).to_vec()),
            })),
            // asset_identifier: None,
            // asset_amount: u128::to_be_bytes(1000u128).to_vec(),
            memo: Some("Test".to_string()),
        };

        let proto_address = ProtoSparkAddress {
            identity_public_key: public_key.serialize().to_vec(),
            spark_invoice_fields: Some(proto_fields.clone()),
            signature: None,
        };

        let payload_bytes = proto_address.encode_to_vec();
        let address = bech32::encode::<Bech32m>(Hrp::parse("sp").unwrap(), &payload_bytes).unwrap();

        // Parsing should work but then fail when converting the proto payment intent
        let result = SparkAddress::from_str(&address);
        assert!(result.is_err());
    }
}
