use serde::{Deserialize, Serialize};

use crate::{
    lnurl::{auth::LnurlAuthRequestDetails, pay::LnurlPayRequestDetails},
    network::BitcoinNetwork,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Amount {
    Bitcoin {
        amount_msat: u64,
    },
    /// An amount of currency specified using ISO 4712.
    Currency {
        /// The currency that the amount is denominated in.
        iso4217_code: String,
        /// The amount in the currency unit adjusted by the ISO 4712 exponent (e.g., USD cents).
        fractional_amount: u64,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bip21Details {
    pub amount_sat: Option<u64>,
    pub asset_id: Option<String>,
    pub uri: String,
    pub extras: Vec<Bip21Extra>,
    pub label: Option<String>,
    pub message: Option<String>,
    pub payment_methods: Vec<InputType>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bip21Extra {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BitcoinAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11Invoice {
    pub bolt11: String,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11RouteHint {
    pub hops: Vec<Bolt11RouteHintHop>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11RouteHintHop {
    /// The `node_id` of the non-target end of the route
    pub src_node_id: String,
    /// The `short_channel_id` of this channel
    pub short_channel_id: String,
    /// The fees which must be paid to use this channel
    pub fees_base_msat: u32,
    pub fees_proportional_millionths: u32,

    /// The difference in CLTV values between this node and the next node.
    pub cltv_expiry_delta: u16,
    /// The minimum value, in msat, which must be relayed to the next hop.
    pub htlc_minimum_msat: Option<u64>,
    /// The maximum value in msat available for routing with a single HTLC.
    pub htlc_maximum_msat: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12Invoice {
    pub invoice: String,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12InvoiceRequestDetails {
    // TODO: Fill fields
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12OfferBlindedPath {
    pub blinded_hops: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11InvoiceDetails {
    pub amount_msat: Option<u64>,
    pub description: Option<String>,
    pub description_hash: Option<String>,
    pub expiry: u64,
    pub invoice: Bolt11Invoice,
    pub min_final_cltv_expiry_delta: u64,
    pub network: BitcoinNetwork,
    pub payee_pubkey: String,
    pub payment_hash: String,
    pub payment_secret: String,
    pub routing_hints: Vec<Bolt11RouteHint>,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12InvoiceDetails {
    // TODO: Fill fields
    pub amount_msat: u64,
    pub invoice: Bolt12Invoice,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12Offer {
    pub offer: String,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12OfferDetails {
    pub absolute_expiry: Option<u64>,
    pub chains: Vec<String>,
    pub description: Option<String>,
    pub issuer: Option<String>,
    pub min_amount: Option<Amount>,
    pub offer: Bolt12Offer,
    pub paths: Vec<Bolt12OfferBlindedPath>,
    pub signing_pubkey: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum InputType {
    BitcoinAddress(BitcoinAddressDetails),
    Bolt11Invoice(Bolt11InvoiceDetails),
    Bolt12Invoice(Bolt12InvoiceDetails),
    Bolt12Offer(Bolt12OfferDetails),
    LightningAddress(LightningAddressDetails),
    LnurlPay(LnurlPayRequestDetails),
    SilentPaymentAddress(SilentPaymentAddressDetails),
    LnurlAuth(LnurlAuthRequestDetails),
    Url(String),
    Bip21(Bip21Details),
    Bolt12InvoiceRequest(Bolt12InvoiceRequestDetails),
    LnurlWithdraw(LnurlWithdrawRequestDetails),
    SparkAddress(SparkAddressDetails),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkAddressDetails {
    pub address: String,
    pub decoded_address: SparkAddress,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkAddress {
    pub identity_public_key: String,
    pub network: BitcoinNetwork,
    pub spark_invoice_fields: Option<SparkInvoiceFields>,
    pub signature: Option<String>,
}

impl From<spark_wallet::SparkAddress> for SparkAddress {
    fn from(spark_address: spark_wallet::SparkAddress) -> Self {
        SparkAddress {
            identity_public_key: spark_address.identity_public_key.to_string(),
            network: match spark_address.network {
                spark_wallet::Network::Mainnet => BitcoinNetwork::Bitcoin,
                spark_wallet::Network::Testnet => BitcoinNetwork::Testnet3,
                spark_wallet::Network::Regtest => BitcoinNetwork::Regtest,
                spark_wallet::Network::Signet => BitcoinNetwork::Signet,
            },
            spark_invoice_fields: spark_address.spark_invoice_fields.map(Into::into),
            signature: spark_address.signature.map(|sig| sig.to_string()),
        }
    }
}

impl From<spark::address::SparkInvoiceFields> for SparkInvoiceFields {
    fn from(fields: spark::address::SparkInvoiceFields) -> Self {
        SparkInvoiceFields {
            id: fields.id.to_string(),
            version: fields.version,
            memo: fields.memo,
            sender_public_key: fields.sender_public_key.map(|pk| pk.to_string()),
            expiry_time: fields.expiry_time.map(|time| {
                time.duration_since(web_time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            }),
            payment_type: fields.payment_type.map(Into::into),
        }
    }
}

impl From<spark::address::SparkAddressPaymentType> for SparkAddressPaymentType {
    fn from(payment_type: spark::address::SparkAddressPaymentType) -> Self {
        match payment_type {
            spark::address::SparkAddressPaymentType::TokensPayment(tp) => {
                SparkAddressPaymentType::TokensPayment(TokensPaymentDetails {
                    token_identifier: tp.token_identifier.map(|id| id.to_string()),
                    amount: tp.amount,
                })
            }
            spark::address::SparkAddressPaymentType::SatsPayment(sp) => {
                SparkAddressPaymentType::SatsPayment(SatsPaymentDetails { amount: sp.amount })
            }
        }
    }
}

impl From<spark::address::TokensPayment> for TokensPaymentDetails {
    fn from(tp: spark::address::TokensPayment) -> Self {
        TokensPaymentDetails {
            token_identifier: tp.token_identifier.map(|id| id.to_string()),
            amount: tp.amount,
        }
    }
}

impl From<spark::address::SatsPayment> for SatsPaymentDetails {
    fn from(sp: spark::address::SatsPayment) -> Self {
        SatsPaymentDetails { amount: sp.amount }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkInvoiceFields {
    pub id: String,
    pub version: u32,
    pub memo: Option<String>,
    pub sender_public_key: Option<String>,
    pub expiry_time: Option<u64>,
    pub payment_type: Option<SparkAddressPaymentType>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SparkAddressPaymentType {
    TokensPayment(TokensPaymentDetails),
    SatsPayment(SatsPaymentDetails),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TokensPaymentDetails {
    pub token_identifier: Option<String>,
    pub amount: Option<u128>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SatsPaymentDetails {
    pub amount: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LightningAddressDetails {
    pub address: String,
    pub pay_request: LnurlPayRequestDetails,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlWithdrawRequestDetails {
    pub callback: String,
    pub k1: String,
    pub default_description: String,
    /// The minimum amount, in millisats, that this LNURL-withdraw endpoint accepts
    pub min_withdrawable: u64,
    /// The maximum amount, in millisats, that this LNURL-withdraw endpoint accepts
    pub max_withdrawable: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentRequestSource {
    pub bip_21_uri: Option<String>,
    pub bip_353_address: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SilentPaymentAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

// Uniffi bindings have issues if multiple crates define the same custom type. This is a workaround.
#[allow(unused_imports)]
use u128 as common_u128;

#[cfg(feature = "uniffi")]
uniffi::custom_type!(common_u128, String);

#[cfg(feature = "uniffi")]
impl crate::UniffiCustomTypeConverter for u128 {
    type Builtin = String;

    fn into_custom(val: Self::Builtin) -> ::uniffi::Result<Self>
    where
        Self: ::std::marker::Sized,
    {
        val.parse::<u128>()
            .map_err(uniffi::deps::anyhow::Error::msg)
    }

    fn from_custom(obj: Self) -> Self::Builtin {
        obj.to_string()
    }
}
