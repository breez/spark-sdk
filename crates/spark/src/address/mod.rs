pub mod error;

use std::{fmt::Debug, str::FromStr, time::Duration};

use crate::{
    operator::rpc::spark::{
        SatsPayment as ProtoSatsPayment, SparkAddress as ProtoSparkAddress,
        SparkInvoiceFields as ProtoSparkInvoiceFields, TokensPayment as ProtoTokensPayment,
        spark_invoice_fields::PaymentType as ProtoPaymentType,
    },
    services::{bech32m_decode_token_id, bech32m_encode_token_id},
    signer::Signer,
};
use bitcoin::{
    bech32::{self, Bech32m, Hrp},
    hashes::{Hash, HashEngine, sha256},
    key::Secp256k1,
    secp256k1::{Message, PublicKey, schnorr::Signature},
};

use bytes::BytesMut;
use prost::{Message as ProstMessage, encoding};

use error::AddressError;
use uuid::Uuid;
use web_time::{SystemTime, UNIX_EPOCH};

use crate::Network;

const HRP_MAINNET: Hrp = Hrp::parse_unchecked("spark");
const HRP_TESTNET: Hrp = Hrp::parse_unchecked("sparkt");
const HRP_REGTEST: Hrp = Hrp::parse_unchecked("sparkrt");
const HRP_SIGNET: Hrp = Hrp::parse_unchecked("sparks");

// TODO: Remove legacy HRPs for silent payment addresses
const HRP_LEGACY_MAINNET: Hrp = Hrp::parse_unchecked("sp");
const HRP_LEGACY_TESTNET: Hrp = Hrp::parse_unchecked("spt");
const HRP_LEGACY_REGTEST: Hrp = Hrp::parse_unchecked("sprt");
const HRP_LEGACY_SIGNET: Hrp = Hrp::parse_unchecked("sps");

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SparkAddress {
    pub identity_public_key: PublicKey,
    pub network: Network,
    pub spark_invoice_fields: Option<SparkInvoiceFields>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SparkInvoiceFields {
    pub id: Uuid,
    pub version: u32,
    pub memo: Option<String>,
    pub sender_public_key: Option<PublicKey>,
    pub expiry_time: Option<SystemTime>,
    pub payment_type: Option<SparkAddressPaymentType>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SparkAddressPaymentType {
    TokensPayment(TokensPayment),
    SatsPayment(SatsPayment),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SatsPayment {
    pub amount: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TokensPayment {
    /// Bech32m encoded token identifier
    pub token_identifier: Option<String>,
    pub amount: Option<u128>,
}

impl TryFrom<SparkAddressPaymentType> for ProtoPaymentType {
    type Error = AddressError;
    fn try_from(value: SparkAddressPaymentType) -> Result<Self, Self::Error> {
        let payment_type = match value {
            SparkAddressPaymentType::TokensPayment(tp) => {
                ProtoPaymentType::TokensPayment(ProtoTokensPayment {
                    amount: tp.amount.map(to_variable_length_be_bytes),
                    token_identifier: tp
                        .token_identifier
                        .map(|id| {
                            bech32m_decode_token_id(&id, None).map_err(|e| {
                                AddressError::Bech32mDecodeError(format!(
                                    "Invalid token identifier: {e}"
                                ))
                            })
                        })
                        .transpose()?,
                })
            }
            SparkAddressPaymentType::SatsPayment(sp) => {
                ProtoPaymentType::SatsPayment(ProtoSatsPayment { amount: sp.amount })
            }
        };
        Ok(payment_type)
    }
}

impl TryFrom<(ProtoPaymentType, Network)> for SparkAddressPaymentType {
    type Error = AddressError;
    fn try_from((value, network): (ProtoPaymentType, Network)) -> Result<Self, Self::Error> {
        match value {
            ProtoPaymentType::TokensPayment(tp) => {
                let amount = tp
                    .amount
                    .map(|amount| from_variable_length_be_bytes(&amount))
                    .transpose()?;

                Ok(SparkAddressPaymentType::TokensPayment(TokensPayment {
                    token_identifier: tp
                        .token_identifier
                        .map(|id| {
                            bech32m_encode_token_id(&id, network).map_err(|e| {
                                AddressError::Bech32EncodeError(format!(
                                    "Failed to encode token identifier: {e}"
                                ))
                            })
                        })
                        .transpose()?,
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

impl TryFrom<(ProtoSparkInvoiceFields, Network)> for SparkInvoiceFields {
    type Error = AddressError;

    fn try_from((proto, network): (ProtoSparkInvoiceFields, Network)) -> Result<Self, Self::Error> {
        let sender_public_key = match proto.sender_public_key {
            Some(pk) => Some(
                PublicKey::from_slice(&pk)
                    .map_err(|e| AddressError::InvalidPublicKey(e.to_string()))?,
            ),
            None => None,
        };

        let payment_type = match proto.payment_type {
            Some(pt) => Some((pt, network).try_into().map_err(|e| {
                AddressError::InvalidPaymentIntent(format!("Invalid payment type: {e}"))
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

impl TryFrom<SparkInvoiceFields> for ProtoSparkInvoiceFields {
    type Error = AddressError;

    fn try_from(val: SparkInvoiceFields) -> Result<Self, Self::Error> {
        let id = val.id.as_bytes().to_vec();

        let payment_type = val.payment_type.map(|pt| pt.try_into()).transpose()?;

        Ok(ProtoSparkInvoiceFields {
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
        })
    }
}

impl SparkAddress {
    pub fn new(
        identity_public_key: PublicKey,
        network: Network,
        spark_invoice_fields: Option<SparkInvoiceFields>,
    ) -> Self {
        SparkAddress {
            identity_public_key,
            network,
            spark_invoice_fields,
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
            hrp if hrp == &HRP_MAINNET || hrp == &HRP_LEGACY_MAINNET => Ok(Network::Mainnet),
            hrp if hrp == &HRP_TESTNET || hrp == &HRP_LEGACY_TESTNET => Ok(Network::Testnet),
            hrp if hrp == &HRP_REGTEST || hrp == &HRP_LEGACY_REGTEST => Ok(Network::Regtest),
            hrp if hrp == &HRP_SIGNET || hrp == &HRP_LEGACY_SIGNET => Ok(Network::Signet),
            _ => Err(AddressError::UnknownHrp(hrp.to_string())),
        }
    }

    pub fn is_invoice(&self) -> bool {
        self.spark_invoice_fields.is_some()
    }

    pub fn to_address_string(&self) -> Result<String, AddressError> {
        if self.is_invoice() {
            return Err(AddressError::Other(
                "Invoice addresses cannot be converted to address strings".to_string(),
            ));
        }

        let proto_address = ProtoSparkAddress {
            identity_public_key: self.identity_public_key.serialize().to_vec(),
            spark_invoice_fields: None,
            signature: None,
        };

        let payload_bytes = encode_spark_address_canonical(&proto_address);

        let hrp = Self::network_to_hrp(&self.network);

        // This is safe to unwrap, because we are using a valid HRP and payload
        let address = bech32::encode::<Bech32m>(hrp, &payload_bytes).unwrap();
        Ok(address)
    }

    pub fn to_invoice_string(&self, signer: &dyn Signer) -> Result<String, AddressError> {
        if !self.is_invoice() {
            return Err(AddressError::Other(
                "Non-invoice addresses cannot be converted to invoice strings".to_string(),
            ));
        }

        if self.identity_public_key
            != signer.get_identity_public_key().map_err(|e| {
                AddressError::Other(format!("Failed to get identity public key: {e}"))
            })?
        {
            return Err(AddressError::Other(
                "Cannot sign invoice for a different identity".to_string(),
            ));
        }

        let spark_invoice_fields: Option<ProtoSparkInvoiceFields> = self
            .spark_invoice_fields
            .clone()
            .map(|f| f.try_into())
            .transpose()?;

        let invoice_hash = self.compute_invoice_hash()?;

        let signature = signer
            .sign_hash_schnorr_with_identity_key(&invoice_hash)
            .map_err(|e| AddressError::Other(format!("Failed to sign invoice hash: {e}")))?;

        let proto_address = ProtoSparkAddress {
            identity_public_key: self.identity_public_key.serialize().to_vec(),
            spark_invoice_fields,
            signature: Some(signature.serialize().to_vec()),
        };

        // Use canonical encoding for server compatibility
        let payload_bytes = encode_spark_address_canonical(&proto_address);

        let hrp = Self::network_to_hrp(&self.network);

        // This is safe to unwrap, because we are using a valid HRP and payload
        let address = bech32::encode::<Bech32m>(hrp, &payload_bytes).unwrap();
        Ok(address)
    }

    fn compute_invoice_hash(&self) -> Result<Vec<u8>, AddressError> {
        let Some(invoice_fields) = &self.spark_invoice_fields else {
            return Err(AddressError::Other("No invoice fields".to_string()));
        };

        if invoice_fields.version != 1 {
            return Err(AddressError::Other(
                "Unsupported invoice version".to_string(),
            ));
        }

        let mut all_hashes = vec![
            sha256::Hash::hash(&invoice_fields.version.to_be_bytes())
                .to_byte_array()
                .to_vec(),
        ];

        all_hashes.push(
            sha256::Hash::hash(invoice_fields.id.as_bytes())
                .to_byte_array()
                .to_vec(),
        );

        all_hashes.push(
            sha256::Hash::hash(
                &sha256::Hash::hash(&get_magic_network_identifier(self.network)).to_byte_array(),
            )
            .to_byte_array()
            .to_vec(),
        );

        all_hashes.push(
            sha256::Hash::hash(&self.identity_public_key.serialize())
                .to_byte_array()
                .to_vec(),
        );

        match &invoice_fields.payment_type {
            Some(SparkAddressPaymentType::TokensPayment(payment)) => {
                all_hashes.push(sha256::Hash::hash(&[1]).to_byte_array().to_vec());

                if let Some(token_identifier) = &payment.token_identifier {
                    all_hashes.push(
                        sha256::Hash::hash(
                            &bech32m_decode_token_id(token_identifier, None)
                                .map_err(|e| {
                                    AddressError::Other(format!("Invalid token identifier: {e}"))
                                })?
                                .to_vec(),
                        )
                        .to_byte_array()
                        .to_vec(),
                    );
                } else {
                    all_hashes.push(sha256::Hash::hash(&[0; 32]).to_byte_array().to_vec());
                }

                all_hashes.push(
                    sha256::Hash::hash(&to_variable_length_be_bytes(payment.amount.unwrap_or(0)))
                        .to_byte_array()
                        .to_vec(),
                );
            }
            Some(SparkAddressPaymentType::SatsPayment(payment)) => {
                all_hashes.push(sha256::Hash::hash(&[2]).to_byte_array().to_vec());

                let amount = payment.amount.unwrap_or(0);
                all_hashes.push(
                    sha256::Hash::hash(&amount.to_be_bytes())
                        .to_byte_array()
                        .to_vec(),
                );
            }
            None => {
                return Err(AddressError::Other("No payment type".to_string()));
            }
        }

        if let Some(memo) = &invoice_fields.memo {
            all_hashes.push(sha256::Hash::hash(memo.as_bytes()).to_byte_array().to_vec());
        } else {
            all_hashes.push(sha256::Hash::hash(&[]).to_byte_array().to_vec());
        }

        if let Some(sender_public_key) = &invoice_fields.sender_public_key {
            all_hashes.push(
                sha256::Hash::hash(&sender_public_key.serialize())
                    .to_byte_array()
                    .to_vec(),
            );
        } else {
            all_hashes.push(sha256::Hash::hash(&[0; 33]).to_byte_array().to_vec());
        }

        let expiry = invoice_fields
            .expiry_time
            .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
            .unwrap_or(0);
        all_hashes.push(
            sha256::Hash::hash(&expiry.to_be_bytes())
                .to_byte_array()
                .to_vec(),
        );

        let mut engine = sha256::Hash::engine();
        for hash in all_hashes {
            engine.input(&hash);
        }
        let final_hash = sha256::Hash::from_engine(engine).to_byte_array().to_vec();

        Ok(final_hash)
    }
}

/// Returns 4 bytes of the magic network identifier for the given network.
fn get_magic_network_identifier(network: Network) -> Vec<u8> {
    let magic: i64 = match network {
        Network::Mainnet => 0xd9b4bef9,
        Network::Regtest => 0xdab5bffa,
        Network::Testnet => 0x0709110b,
        Network::Signet => 0x40cf030a,
    };
    magic.to_be_bytes()[4..].to_vec()
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
            .map(|f| (f, network).try_into())
            .transpose()?;

        let signature = proto_address
            .signature
            .map(|s| {
                Signature::from_slice(&s).map_err(|e| AddressError::InvalidSignature(e.to_string()))
            })
            .transpose()?;

        let address = SparkAddress::new(identity_public_key, network, invoice_fields);

        if address.is_invoice() {
            let hash = address.compute_invoice_hash()?;

            let Some(sig) = signature else {
                return Err(AddressError::Other("Invoice has no signature".to_string()));
            };

            let secp = Secp256k1::new();
            if secp
                .verify_schnorr(
                    &sig,
                    &Message::from_digest(hash.try_into().unwrap_or_default()),
                    &address.identity_public_key.x_only_public_key().0,
                )
                .is_err()
            {
                return Err(AddressError::Other("Invalid invoice signature".to_string()));
            }
        }

        Ok(address)
    }
}

fn to_variable_length_be_bytes(value: u128) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    bytes
        .iter()
        .skip_while(|&&b| b == 0)
        .copied()
        .collect::<Vec<u8>>()
}

fn from_variable_length_be_bytes(bytes: &[u8]) -> Result<u128, AddressError> {
    if bytes.len() > 16 {
        return Err(AddressError::InvalidPaymentIntent(
            "Invalid amount: length exceeds 16 bytes".to_string(),
        ));
    }
    let mut arr = [0u8; 16];
    let offset = 16 - bytes.len();
    arr[offset..].copy_from_slice(bytes);
    let amount_bytes = arr;
    Ok(u128::from_be_bytes(amount_bytes))
}

/// Encodes SparkInvoiceFields in canonical field order for server compatibility.
/// Canonical order: version(1), id(2), memo(5), sender_public_key(6), expiry_time(7), payment_type(3 or 4) last
fn encode_spark_invoice_fields_canonical(fields: &ProtoSparkInvoiceFields) -> Vec<u8> {
    let mut buf = BytesMut::new();

    // version (field 1)
    if fields.version != 0 {
        encoding::uint32::encode(1, &fields.version, &mut buf);
    }

    // id (field 2)
    if !fields.id.is_empty() {
        encoding::bytes::encode(2, &fields.id, &mut buf);
    }

    // memo (field 5)
    if let Some(memo) = &fields.memo {
        encoding::string::encode(5, memo, &mut buf);
    }

    // sender_public_key (field 6)
    if let Some(sender_key) = &fields.sender_public_key {
        encoding::bytes::encode(6, sender_key, &mut buf);
    }

    // expiry_time (field 7)
    if let Some(expiry) = &fields.expiry_time {
        encoding::message::encode(7, expiry, &mut buf);
    }

    // payment_type oneof: tokens (3) or sats (4) - encoded last
    if let Some(payment_type) = &fields.payment_type {
        match payment_type {
            ProtoPaymentType::TokensPayment(tokens) => {
                encoding::message::encode(3, tokens, &mut buf);
            }
            ProtoPaymentType::SatsPayment(sats) => {
                encoding::message::encode(4, sats, &mut buf);
            }
        }
    }

    buf.to_vec()
}

/// Encodes SparkAddress with canonical inner SparkInvoiceFields encoding.
fn encode_spark_address_canonical(address: &ProtoSparkAddress) -> Vec<u8> {
    let mut buf = BytesMut::new();

    // identity_public_key (field 1)
    encoding::bytes::encode(1, &address.identity_public_key, &mut buf);

    // spark_invoice_fields (field 2) with canonical inner order
    if let Some(invoice_fields) = &address.spark_invoice_fields {
        let inner = encode_spark_invoice_fields_canonical(invoice_fields);
        encoding::bytes::encode(2, &inner, &mut buf);
    }

    // signature (field 3)
    if let Some(signature) = &address.signature {
        encoding::bytes::encode(3, signature, &mut buf);
    }

    buf.to_vec()
}

#[cfg(test)]
mod tests {

    use crate::signer::create_test_signer;

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
        let original_address = SparkAddress::new(public_key, Network::Mainnet, None);

        let address_string = original_address.to_address_string().unwrap();
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
        let original_address = SparkAddress::new(public_key, Network::Testnet, None);

        let address_string = original_address.to_address_string().unwrap();
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
        let original_address = SparkAddress::new(public_key, Network::Regtest, None);

        let address_string = original_address.to_address_string().unwrap();
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
        let original_address = SparkAddress::new(public_key, Network::Signet, None);

        let address_string = original_address.to_address_string().unwrap();
        let parsed_address = SparkAddress::from_str(&address_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);
    }

    #[test_all]
    fn test_parse_specific_regtest_address() {
        let address_str = "sparkrt1pgssyuuuhnrrdjswal5c3s3rafw9w3y5dd4cjy3duxlf7hjzkp0rqx6dc0nltx";
        let address = SparkAddress::from_str(address_str).unwrap();

        assert_eq!(address.network, Network::Regtest);
        assert_eq!(
            address.identity_public_key,
            PublicKey::from_str(
                "02739cbcc636ca0eefe988c223ea5c5744946b6b89122de1be9f5e42b05e301b4d"
            )
            .unwrap()
        );
    }

    #[test_all]
    fn test_parse_specific_legacy_regtest_address() {
        let address_str = "sprt1pgssyuuuhnrrdjswal5c3s3rafw9w3y5dd4cjy3duxlf7hjzkp0rqx6dj6mrhu";
        let address = SparkAddress::from_str(address_str).unwrap();

        assert_eq!(address.network, Network::Regtest);
        assert_eq!(
            address.identity_public_key,
            PublicKey::from_str(
                "02739cbcc636ca0eefe988c223ea5c5744946b6b89122de1be9f5e42b05e301b4d"
            )
            .unwrap()
        );
    }

    #[test_all]
    fn test_parse_specific_sats_invoice() {
        let invoice_str = "sparkrt1pgss8cf4gru7ece2ryn8ym3vm3yz8leeend2589m7svq2mgv0xncfyx8zf8ssqgjzqqe5pmwfwyh9u4u6wgrepzk7j6j5prdv4kk7v3pqdur4y4c5nlcyr7lksm4mhrhdzakas9yt8gz4levtnfe49sgkqknywstpzxd8hk8qcgvp7x22q3qxz8gqudyp7rmuglc2axjqnlzz7d047gndmxff6ud02fvdgasdsq2en2aah6g52rq4qq7peler4s4d85s7prhm6sqzqj7gvc9nlzucy4yfh206fyqpk9zez";
        let invoice = SparkAddress::from_str(invoice_str).unwrap();

        assert_eq!(invoice.network, Network::Regtest);
        let invoice_fields = invoice.spark_invoice_fields.unwrap();
        assert_eq!(
            invoice_fields.payment_type,
            Some(SparkAddressPaymentType::SatsPayment(SatsPayment {
                amount: Some(1000)
            }))
        );
        assert_eq!(
            invoice_fields
                .expiry_time
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            1761061260
        );
        assert_eq!(invoice_fields.memo, Some("memo".to_string()));
        assert_eq!(
            invoice_fields.sender_public_key,
            Some(
                PublicKey::from_str(
                    "03783a92b8a4ff820fdfb4375ddc7768bb6ec0a459d02aff2c5cd39a9608b02d32"
                )
                .unwrap()
            )
        );
    }

    #[test_all]
    fn test_parse_specific_token_invoice() {
        let invoice_str = "sparkrt1pgss8cf4gru7ece2ryn8ym3vm3yz8leeend2589m7svq2mgv0xncfyx8zfeqsqgjzqqe5pmtue38y3avl89vac53nkzj5prdv4kk7v3pqdur4y4c5nlcyr7lksm4mhrhdzakas9yt8gz4levtnfe49sgkqknywstprharhk8qcgvpz8vtudzvz3q5xy2yxnpacs2yl00fnajxylsljq9y0uesr6qylyxq2lxnum8r63pyqsraqdyp57uf363avdlv59eqjfdszpwc2y3zfpww2cevcx92zw40qxf5fedvrlmnrmsg7pa3egggtw03kd0rz73lvgl5u3c02krhhcwc3haec4q2e582x";
        let invoice = SparkAddress::from_str(invoice_str).unwrap();

        assert_eq!(invoice.network, Network::Regtest);
        let invoice_fields = invoice.spark_invoice_fields.unwrap();
        assert_eq!(
            invoice_fields.payment_type,
            Some(SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier: Some(
                    "btknrt15xy2yxnpacs2yl00fnajxylsljq9y0uesr6qylyxq2lxnum8r63qfues7q".to_string(),
                ),
                amount: Some(1000)
            }))
        );
        assert_eq!(
            invoice_fields
                .expiry_time
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            1761061103
        );
        assert_eq!(invoice_fields.memo, Some("memo".to_string()));
        assert_eq!(
            invoice_fields.sender_public_key,
            Some(
                PublicKey::from_str(
                    "03783a92b8a4ff820fdfb4375ddc7768bb6ec0a459d02aff2c5cd39a9608b02d32"
                )
                .unwrap()
            )
        );
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

        // Use an unknown HRP "sparkx" instead of valid ones
        let address =
            bech32::encode::<Bech32m>(Hrp::parse("sparkx").unwrap(), &payload_bytes).unwrap();

        let result = SparkAddress::from_str(&address);
        assert!(result.is_err());
        match result {
            Err(AddressError::UnknownHrp(hrp)) => {
                assert_eq!(hrp, "sparkx");
            }
            _ => panic!("Expected UnknownHrp error"),
        }
    }

    #[test_all]
    fn test_invoice_roundtrip() {
        let signer = create_test_signer();
        let public_key = signer.get_identity_public_key().unwrap();
        let sender_public_key = create_test_public_key();
        let invoice_fields = SparkInvoiceFields {
            id: uuid::Uuid::now_v7(),
            version: 1,
            sender_public_key: Some(sender_public_key),
            expiry_time: Some(SystemTime::now()),
            payment_type: Some(SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier: Some(
                    "btknrt15xy2yxnpacs2yl00fnajxylsljq9y0uesr6qylyxq2lxnum8r63qfues7q".to_string(),
                ),
                amount: Some(100),
            })),
            memo: Some("Test payment".to_string()),
        };

        let original_address =
            SparkAddress::new(public_key, Network::Regtest, Some(invoice_fields.clone()));

        let invoice_string = original_address.to_invoice_string(&signer).unwrap();
        let parsed_address = SparkAddress::from_str(&invoice_string).unwrap();

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
    fn test_invoice_minimal_data() {
        let signer = create_test_signer();
        let public_key = signer.get_identity_public_key().unwrap();
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
            SparkAddress::new(public_key, Network::Testnet, Some(invoice_fields));

        let invoice_string = original_address.to_invoice_string(&signer).unwrap();
        let parsed_address = SparkAddress::from_str(&invoice_string).unwrap();

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
        let signer = create_test_signer();
        let public_key = signer.get_identity_public_key().unwrap();
        let sender_public_key = create_test_public_key();

        // Create address without invoice fields
        let address_without_intent = SparkAddress::new(public_key, Network::Mainnet, None);
        let string_without_intent = address_without_intent.to_address_string().unwrap();

        let invoice_fields = SparkInvoiceFields {
            id: uuid::Uuid::now_v7(),
            version: 1,
            sender_public_key: Some(sender_public_key),
            expiry_time: Some(SystemTime::now()),
            payment_type: Some(SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier: Some(
                    "btknrt15xy2yxnpacs2yl00fnajxylsljq9y0uesr6qylyxq2lxnum8r63qfues7q".to_string(),
                ),
                amount: Some(100),
            })),
            memo: Some("Test memo".to_string()),
        };
        let address_with_intent =
            SparkAddress::new(public_key, Network::Mainnet, Some(invoice_fields));
        let string_with_intent = address_with_intent.to_invoice_string(&signer).unwrap();

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

    #[test_all]
    fn test_to_variable_length_be_bytes() {
        assert_eq!(to_variable_length_be_bytes(0), Vec::<u8>::new());
        assert_eq!(to_variable_length_be_bytes(1), vec![1]);
        assert_eq!(to_variable_length_be_bytes(256), vec![1, 0]);
    }

    #[test_all]
    fn test_from_variable_length_be_bytes() {
        assert_eq!(from_variable_length_be_bytes(&[]).unwrap(), 0);
        assert_eq!(from_variable_length_be_bytes(&[1]).unwrap(), 1);
        assert_eq!(from_variable_length_be_bytes(&[1, 0]).unwrap(), 256);

        assert!(
            from_variable_length_be_bytes(&[
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17
            ])
            .is_err()
        );
    }
}
