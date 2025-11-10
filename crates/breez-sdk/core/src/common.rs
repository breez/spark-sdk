use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum BitcoinNetwork {
    /// Mainnet
    Bitcoin,
    Testnet3,
    Testnet4,
    Signet,
    Regtest,
}

impl From<breez_sdk_common::network::BitcoinNetwork> for BitcoinNetwork {
    fn from(value: breez_sdk_common::network::BitcoinNetwork) -> Self {
        match value {
            breez_sdk_common::network::BitcoinNetwork::Bitcoin => BitcoinNetwork::Bitcoin,
            breez_sdk_common::network::BitcoinNetwork::Testnet3 => BitcoinNetwork::Testnet3,
            breez_sdk_common::network::BitcoinNetwork::Testnet4 => BitcoinNetwork::Testnet4,
            breez_sdk_common::network::BitcoinNetwork::Signet => BitcoinNetwork::Signet,
            breez_sdk_common::network::BitcoinNetwork::Regtest => BitcoinNetwork::Regtest,
        }
    }
}

#[derive(Debug, Error, Clone)]
pub enum ParseError {
    #[error("empty input")]
    EmptyInput,
    #[error("Bip-21 error: {0}")]
    Bip21Error(Bip21Error),
    #[error("invalid input")]
    InvalidInput,
    #[error("Lnurl error: {0}")]
    LnurlError(LnurlError),
    #[error("Service connectivity error: {0}")]
    ServiceConnectivity(ServiceConnectivityError),
    #[error("Invalid external input parser: {0}")]
    InvalidExternalInputParser(String),
}

impl From<breez_sdk_common::input::ParseError> for ParseError {
    fn from(value: breez_sdk_common::input::ParseError) -> Self {
        match value {
            breez_sdk_common::input::ParseError::EmptyInput => ParseError::EmptyInput,
            breez_sdk_common::input::ParseError::Bip21Error(e) => ParseError::Bip21Error(e.into()),
            breez_sdk_common::input::ParseError::InvalidInput => ParseError::InvalidInput,
            breez_sdk_common::input::ParseError::LnurlError(e) => ParseError::LnurlError(e.into()),
            breez_sdk_common::input::ParseError::ServiceConnectivity(e) => {
                ParseError::ServiceConnectivity(e.into())
            }
            breez_sdk_common::input::ParseError::InvalidExternalInputParser(s) => {
                ParseError::InvalidExternalInputParser(s)
            }
        }
    }
}

#[derive(Debug, Error, Clone)]
pub enum Bip21Error {
    #[error("bip21 contains invalid address")]
    InvalidAddress,
    #[error("bip21 contains invalid amount")]
    InvalidAmount,
    #[error("bip21 contains invalid parameter value for '{0}'")]
    InvalidParameter(String),
    #[error("bip21 parameter missing equals character")]
    MissingEquals,
    #[error("bip21 contains parameter '{0}' multiple times")]
    MultipleParams(String),
    #[error("bip21 contains unknown required parameter '{0}'")]
    UnknownRequiredParameter(String),
    #[error("bip21 does not contain any payment methods")]
    NoPaymentMethods,
}

impl From<breez_sdk_common::input::Bip21Error> for Bip21Error {
    fn from(value: breez_sdk_common::input::Bip21Error) -> Self {
        match value {
            breez_sdk_common::input::Bip21Error::InvalidAddress => Bip21Error::InvalidAddress,
            breez_sdk_common::input::Bip21Error::InvalidAmount => Bip21Error::InvalidAmount,
            breez_sdk_common::input::Bip21Error::InvalidParameter(name) => {
                Bip21Error::InvalidParameter(name)
            }
            breez_sdk_common::input::Bip21Error::MissingEquals => Bip21Error::MissingEquals,
            breez_sdk_common::input::Bip21Error::MultipleParams(name) => {
                Bip21Error::MultipleParams(name)
            }
            breez_sdk_common::input::Bip21Error::UnknownRequiredParameter(name) => {
                Bip21Error::UnknownRequiredParameter(name)
            }
            breez_sdk_common::input::Bip21Error::NoPaymentMethods => Bip21Error::NoPaymentMethods,
        }
    }
}

#[derive(Debug, Error, Clone)]
pub enum LnurlError {
    #[error("lnurl missing k1 parameter")]
    MissingK1,
    #[error("lnurl contains invalid k1 parameter")]
    InvalidK1,
    #[error("lnurl contains unsupported action")]
    UnsupportedAction,
    #[error("lnurl missing domain")]
    MissingDomain,
    #[error("error calling lnurl endpoint: {0}")]
    ServiceConnectivity(#[from] ServiceConnectivityError),
    #[error("endpoint error: {0}")]
    EndpointError(String),
    #[error("lnurl has http scheme without onion domain")]
    HttpSchemeWithoutOnionDomain,
    #[error("lnurl has https scheme with onion domain")]
    HttpsSchemeWithOnionDomain,
    #[error("lnurl error: {0}")]
    General(String),
    #[error("lnurl has unknown scheme")]
    UnknownScheme,
    #[error("lnurl has invalid uri: {0}")]
    InvalidUri(String),
    #[error("lnurl has invalid invoice: {0}")]
    InvalidInvoice(String),
    #[error("lnurl has invalid response: {0}")]
    InvalidResponse(String),
}

impl From<breez_sdk_common::lnurl::error::LnurlError> for LnurlError {
    fn from(value: breez_sdk_common::lnurl::error::LnurlError) -> Self {
        match value {
            breez_sdk_common::lnurl::error::LnurlError::MissingK1 => LnurlError::MissingK1,
            breez_sdk_common::lnurl::error::LnurlError::InvalidK1 => LnurlError::InvalidK1,
            breez_sdk_common::lnurl::error::LnurlError::UnsupportedAction => {
                LnurlError::UnsupportedAction
            }
            breez_sdk_common::lnurl::error::LnurlError::MissingDomain => LnurlError::MissingDomain,
            breez_sdk_common::lnurl::error::LnurlError::ServiceConnectivity(e) => {
                LnurlError::ServiceConnectivity(e.into())
            }
            breez_sdk_common::lnurl::error::LnurlError::EndpointError(s) => {
                LnurlError::EndpointError(s)
            }
            breez_sdk_common::lnurl::error::LnurlError::HttpSchemeWithoutOnionDomain => {
                LnurlError::HttpSchemeWithoutOnionDomain
            }
            breez_sdk_common::lnurl::error::LnurlError::HttpsSchemeWithOnionDomain => {
                LnurlError::HttpsSchemeWithOnionDomain
            }
            breez_sdk_common::lnurl::error::LnurlError::General(s) => LnurlError::General(s),
            breez_sdk_common::lnurl::error::LnurlError::UnknownScheme => LnurlError::UnknownScheme,
            breez_sdk_common::lnurl::error::LnurlError::InvalidUri(s) => LnurlError::InvalidUri(s),
            breez_sdk_common::lnurl::error::LnurlError::InvalidInvoice(s) => {
                LnurlError::InvalidInvoice(s)
            }
            breez_sdk_common::lnurl::error::LnurlError::InvalidResponse(s) => {
                LnurlError::InvalidResponse(s)
            }
        }
    }
}

#[derive(Clone, Debug, Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum ServiceConnectivityError {
    #[error("Builder error: {0}")]
    Builder(String),
    #[error("Redirect error: {0}")]
    Redirect(String),
    #[error("Status error: {status} - {body}")]
    Status { status: u16, body: String },
    #[error("Timeout error: {0}")]
    Timeout(String),
    #[error("Request error: {0}")]
    Request(String),
    #[error("Connect error: {0}")]
    Connect(String),
    #[error("Body error: {0}")]
    Body(String),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Json error: {0}")]
    Json(String),
    #[error("Other error: {0}")]
    Other(String),
}

impl From<breez_sdk_common::error::ServiceConnectivityError> for ServiceConnectivityError {
    fn from(value: breez_sdk_common::error::ServiceConnectivityError) -> Self {
        match value {
            breez_sdk_common::error::ServiceConnectivityError::Builder(s) => {
                ServiceConnectivityError::Builder(s)
            }
            breez_sdk_common::error::ServiceConnectivityError::Redirect(s) => {
                ServiceConnectivityError::Redirect(s)
            }
            breez_sdk_common::error::ServiceConnectivityError::Status { status, body } => {
                ServiceConnectivityError::Status { status, body }
            }
            breez_sdk_common::error::ServiceConnectivityError::Timeout(s) => {
                ServiceConnectivityError::Timeout(s)
            }
            breez_sdk_common::error::ServiceConnectivityError::Request(s) => {
                ServiceConnectivityError::Request(s)
            }
            breez_sdk_common::error::ServiceConnectivityError::Connect(s) => {
                ServiceConnectivityError::Connect(s)
            }
            breez_sdk_common::error::ServiceConnectivityError::Body(s) => {
                ServiceConnectivityError::Body(s)
            }
            breez_sdk_common::error::ServiceConnectivityError::Decode(s) => {
                ServiceConnectivityError::Decode(s)
            }
            breez_sdk_common::error::ServiceConnectivityError::Json(s) => {
                ServiceConnectivityError::Json(s)
            }
            breez_sdk_common::error::ServiceConnectivityError::Other(s) => {
                ServiceConnectivityError::Other(s)
            }
        }
    }
}

impl From<ServiceConnectivityError> for breez_sdk_common::error::ServiceConnectivityError {
    fn from(value: ServiceConnectivityError) -> Self {
        match value {
            ServiceConnectivityError::Builder(s) => {
                breez_sdk_common::error::ServiceConnectivityError::Builder(s)
            }
            ServiceConnectivityError::Redirect(s) => {
                breez_sdk_common::error::ServiceConnectivityError::Redirect(s)
            }
            ServiceConnectivityError::Status { status, body } => {
                breez_sdk_common::error::ServiceConnectivityError::Status { status, body }
            }
            ServiceConnectivityError::Timeout(s) => {
                breez_sdk_common::error::ServiceConnectivityError::Timeout(s)
            }
            ServiceConnectivityError::Request(s) => {
                breez_sdk_common::error::ServiceConnectivityError::Request(s)
            }
            ServiceConnectivityError::Connect(s) => {
                breez_sdk_common::error::ServiceConnectivityError::Connect(s)
            }
            ServiceConnectivityError::Body(s) => {
                breez_sdk_common::error::ServiceConnectivityError::Body(s)
            }
            ServiceConnectivityError::Decode(s) => {
                breez_sdk_common::error::ServiceConnectivityError::Decode(s)
            }
            ServiceConnectivityError::Json(s) => {
                breez_sdk_common::error::ServiceConnectivityError::Json(s)
            }
            ServiceConnectivityError::Other(s) => {
                breez_sdk_common::error::ServiceConnectivityError::Other(s)
            }
        }
    }
}

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

impl From<breez_sdk_common::input::Amount> for Amount {
    fn from(value: breez_sdk_common::input::Amount) -> Self {
        match value {
            breez_sdk_common::input::Amount::Bitcoin { amount_msat } => {
                Amount::Bitcoin { amount_msat }
            }
            breez_sdk_common::input::Amount::Currency {
                iso4217_code,
                fractional_amount,
            } => Amount::Currency {
                iso4217_code,
                fractional_amount,
            },
        }
    }
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

impl From<breez_sdk_common::input::Bip21Details> for Bip21Details {
    fn from(value: breez_sdk_common::input::Bip21Details) -> Self {
        Bip21Details {
            amount_sat: value.amount_sat,
            asset_id: value.asset_id,
            uri: value.uri,
            extras: value.extras.into_iter().map(From::from).collect(),
            label: value.label,
            message: value.message,
            payment_methods: value.payment_methods.into_iter().map(From::from).collect(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bip21Extra {
    pub key: String,
    pub value: String,
}

impl From<breez_sdk_common::input::Bip21Extra> for Bip21Extra {
    fn from(value: breez_sdk_common::input::Bip21Extra) -> Self {
        Bip21Extra {
            key: value.key,
            value: value.value,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BitcoinAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

impl From<breez_sdk_common::input::BitcoinAddressDetails> for BitcoinAddressDetails {
    fn from(value: breez_sdk_common::input::BitcoinAddressDetails) -> Self {
        BitcoinAddressDetails {
            address: value.address,
            network: BitcoinNetwork::from(value.network),
            source: value.source.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11Invoice {
    pub bolt11: String,
    pub source: PaymentRequestSource,
}

impl From<breez_sdk_common::input::Bolt11Invoice> for Bolt11Invoice {
    fn from(value: breez_sdk_common::input::Bolt11Invoice) -> Self {
        Bolt11Invoice {
            bolt11: value.bolt11,
            source: value.source.into(),
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11RouteHint {
    pub hops: Vec<Bolt11RouteHintHop>,
}

impl From<breez_sdk_common::input::Bolt11RouteHint> for Bolt11RouteHint {
    fn from(value: breez_sdk_common::input::Bolt11RouteHint) -> Self {
        Bolt11RouteHint {
            hops: value.hops.into_iter().map(From::from).collect(),
        }
    }
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

impl From<breez_sdk_common::input::Bolt11RouteHintHop> for Bolt11RouteHintHop {
    fn from(value: breez_sdk_common::input::Bolt11RouteHintHop) -> Self {
        Bolt11RouteHintHop {
            src_node_id: value.src_node_id,
            short_channel_id: value.short_channel_id,
            fees_base_msat: value.fees_base_msat,
            fees_proportional_millionths: value.fees_proportional_millionths,
            cltv_expiry_delta: value.cltv_expiry_delta,
            htlc_minimum_msat: value.htlc_minimum_msat,
            htlc_maximum_msat: value.htlc_maximum_msat,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12Invoice {
    pub invoice: String,
    pub source: PaymentRequestSource,
}

impl From<breez_sdk_common::input::Bolt12Invoice> for Bolt12Invoice {
    fn from(value: breez_sdk_common::input::Bolt12Invoice) -> Self {
        Bolt12Invoice {
            invoice: value.invoice,
            source: value.source.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12InvoiceRequestDetails {
    // TODO: Fill fields
}

impl From<breez_sdk_common::input::Bolt12InvoiceRequestDetails> for Bolt12InvoiceRequestDetails {
    fn from(_value: breez_sdk_common::input::Bolt12InvoiceRequestDetails) -> Self {
        Bolt12InvoiceRequestDetails {}
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12OfferBlindedPath {
    pub blinded_hops: Vec<String>,
}

impl From<breez_sdk_common::input::Bolt12OfferBlindedPath> for Bolt12OfferBlindedPath {
    fn from(value: breez_sdk_common::input::Bolt12OfferBlindedPath) -> Self {
        Bolt12OfferBlindedPath {
            blinded_hops: value.blinded_hops,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
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

impl From<breez_sdk_common::input::Bolt11InvoiceDetails> for Bolt11InvoiceDetails {
    fn from(value: breez_sdk_common::input::Bolt11InvoiceDetails) -> Self {
        Bolt11InvoiceDetails {
            amount_msat: value.amount_msat,
            description: value.description,
            description_hash: value.description_hash,
            expiry: value.expiry,
            invoice: value.invoice.into(),
            min_final_cltv_expiry_delta: value.min_final_cltv_expiry_delta,
            network: BitcoinNetwork::from(value.network),
            payee_pubkey: value.payee_pubkey,
            payment_hash: value.payment_hash,
            payment_secret: value.payment_secret,
            routing_hints: value.routing_hints.into_iter().map(From::from).collect(),
            timestamp: value.timestamp,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12InvoiceDetails {
    pub amount_msat: u64,
    pub invoice: Bolt12Invoice,
}

impl From<breez_sdk_common::input::Bolt12InvoiceDetails> for Bolt12InvoiceDetails {
    fn from(value: breez_sdk_common::input::Bolt12InvoiceDetails) -> Self {
        Bolt12InvoiceDetails {
            amount_msat: value.amount_msat,
            invoice: value.invoice.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12Offer {
    pub offer: String,
    pub source: PaymentRequestSource,
}

impl From<breez_sdk_common::input::Bolt12Offer> for Bolt12Offer {
    fn from(value: breez_sdk_common::input::Bolt12Offer) -> Self {
        Bolt12Offer {
            offer: value.offer,
            source: value.source.into(),
        }
    }
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

impl From<breez_sdk_common::input::Bolt12OfferDetails> for Bolt12OfferDetails {
    fn from(value: breez_sdk_common::input::Bolt12OfferDetails) -> Self {
        Bolt12OfferDetails {
            absolute_expiry: value.absolute_expiry,
            chains: value.chains,
            description: value.description,
            issuer: value.issuer,
            min_amount: value.min_amount.map(From::from),
            offer: value.offer.into(),
            paths: value.paths.into_iter().map(From::from).collect(),
            signing_pubkey: value.signing_pubkey,
        }
    }
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
    SparkInvoice(SparkInvoiceDetails),
}

impl From<breez_sdk_common::input::InputType> for InputType {
    fn from(value: breez_sdk_common::input::InputType) -> Self {
        match value {
            breez_sdk_common::input::InputType::BitcoinAddress(details) => {
                InputType::BitcoinAddress(details.into())
            }
            breez_sdk_common::input::InputType::Bolt11Invoice(details) => {
                InputType::Bolt11Invoice(details.into())
            }
            breez_sdk_common::input::InputType::Bolt12Invoice(details) => {
                InputType::Bolt12Invoice(details.into())
            }
            breez_sdk_common::input::InputType::Bolt12Offer(details) => {
                InputType::Bolt12Offer(details.into())
            }
            breez_sdk_common::input::InputType::LightningAddress(details) => {
                InputType::LightningAddress(details.into())
            }
            breez_sdk_common::input::InputType::LnurlPay(details) => {
                InputType::LnurlPay(details.into())
            }
            breez_sdk_common::input::InputType::SilentPaymentAddress(details) => {
                InputType::SilentPaymentAddress(details.into())
            }
            breez_sdk_common::input::InputType::LnurlAuth(details) => {
                InputType::LnurlAuth(details.into())
            }
            breez_sdk_common::input::InputType::Url(url) => InputType::Url(url),
            breez_sdk_common::input::InputType::Bip21(details) => InputType::Bip21(details.into()),
            breez_sdk_common::input::InputType::Bolt12InvoiceRequest(details) => {
                InputType::Bolt12InvoiceRequest(details.into())
            }
            breez_sdk_common::input::InputType::LnurlWithdraw(details) => {
                InputType::LnurlWithdraw(details.into())
            }
            breez_sdk_common::input::InputType::SparkAddress(details) => {
                InputType::SparkAddress(details.into())
            }
            breez_sdk_common::input::InputType::SparkInvoice(details) => {
                InputType::SparkInvoice(details.into())
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlPayRequestDetails {
    pub callback: String,
    /// The minimum amount, in millisats, that this LNURL-pay endpoint accepts
    pub min_sendable: u64,
    /// The maximum amount, in millisats, that this LNURL-pay endpoint accepts
    pub max_sendable: u64,
    /// As per LUD-06, `metadata` is a raw string (e.g. a json representation of the inner map).
    /// Use `metadata_vec()` to get the parsed items.
    #[serde(rename(deserialize = "metadata"))]
    pub metadata_str: String,
    /// The comment length accepted by this endpoint
    ///
    /// See <https://github.com/lnurl/luds/blob/luds/12.md>
    #[serde(default)]
    pub comment_allowed: u16,

    /// Indicates the domain of the LNURL-pay service, to be shown to the user when asking for
    /// payment input, as per LUD-06 spec.
    ///
    /// Note: this is not the domain of the callback, but the domain of the LNURL-pay endpoint.
    #[serde(skip)]
    pub domain: String,

    #[serde(skip)]
    pub url: String,

    /// Optional lightning address if that was used to resolve the lnurl.
    #[serde(skip)]
    pub address: Option<String>,

    /// Value indicating whether the recipient supports Nostr Zaps through NIP-57.
    ///
    /// See <https://github.com/nostr-protocol/nips/blob/master/57.md>
    pub allows_nostr: Option<bool>,
    /// Optional recipient's lnurl provider's Nostr pubkey for NIP-57. If it exists it should be a
    /// valid BIP 340 public key in hex.
    ///
    /// See <https://github.com/nostr-protocol/nips/blob/master/57.md>
    /// See <https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki>
    pub nostr_pubkey: Option<String>,
}

impl From<breez_sdk_common::lnurl::pay::LnurlPayRequestDetails> for LnurlPayRequestDetails {
    fn from(value: breez_sdk_common::lnurl::pay::LnurlPayRequestDetails) -> Self {
        LnurlPayRequestDetails {
            callback: value.callback,
            min_sendable: value.min_sendable,
            max_sendable: value.max_sendable,
            metadata_str: value.metadata_str,
            comment_allowed: value.comment_allowed,
            domain: value.domain,
            url: value.url,
            address: value.address,
            allows_nostr: value.allows_nostr,
            nostr_pubkey: value.nostr_pubkey,
        }
    }
}

impl From<LnurlPayRequestDetails> for breez_sdk_common::lnurl::pay::LnurlPayRequestDetails {
    fn from(value: LnurlPayRequestDetails) -> Self {
        breez_sdk_common::lnurl::pay::LnurlPayRequestDetails {
            callback: value.callback,
            min_sendable: value.min_sendable,
            max_sendable: value.max_sendable,
            metadata_str: value.metadata_str,
            comment_allowed: value.comment_allowed,
            domain: value.domain,
            url: value.url,
            address: value.address,
            allows_nostr: value.allows_nostr,
            nostr_pubkey: value.nostr_pubkey,
        }
    }
}

/// Wrapped in a [`LnurlAuth`], this is the result of [`parse`] when given a LNURL-auth endpoint.
///
/// It represents the endpoint's parameters for the LNURL workflow.
///
/// See <https://github.com/lnurl/luds/blob/luds/04.md>
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlAuthRequestDetails {
    /// Hex encoded 32 bytes of challenge
    pub k1: String,

    /// When available, one of: register, login, link, auth
    pub action: Option<String>,

    /// Indicates the domain of the LNURL-auth service, to be shown to the user when asking for
    /// auth confirmation, as per LUD-04 spec.
    #[serde(skip_serializing, skip_deserializing)]
    pub domain: String,

    /// Indicates the URL of the LNURL-auth service, including the query arguments. This will be
    /// extended with the signed challenge and the linking key, then called in the second step of the workflow.
    #[serde(skip_serializing, skip_deserializing)]
    pub url: String,
}

impl From<breez_sdk_common::lnurl::auth::LnurlAuthRequestDetails> for LnurlAuthRequestDetails {
    fn from(value: breez_sdk_common::lnurl::auth::LnurlAuthRequestDetails) -> Self {
        LnurlAuthRequestDetails {
            k1: value.k1,
            action: value.action,
            domain: value.domain,
            url: value.url,
        }
    }
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

impl From<breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails>
    for LnurlWithdrawRequestDetails
{
    fn from(value: breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails) -> Self {
        LnurlWithdrawRequestDetails {
            callback: value.callback,
            k1: value.k1,
            default_description: value.default_description,
            min_withdrawable: value.min_withdrawable,
            max_withdrawable: value.max_withdrawable,
        }
    }
}

impl From<LnurlWithdrawRequestDetails>
    for breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails
{
    fn from(value: LnurlWithdrawRequestDetails) -> Self {
        breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails {
            callback: value.callback,
            k1: value.k1,
            default_description: value.default_description,
            min_withdrawable: value.min_withdrawable,
            max_withdrawable: value.max_withdrawable,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkAddressDetails {
    /// The raw address string
    pub address: String,
    /// The identity public key of the address owner
    pub identity_public_key: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

impl From<breez_sdk_common::input::SparkAddressDetails> for SparkAddressDetails {
    fn from(value: breez_sdk_common::input::SparkAddressDetails) -> Self {
        SparkAddressDetails {
            address: value.address,
            identity_public_key: value.identity_public_key,
            network: value.network.into(),
            source: value.source.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkInvoiceDetails {
    /// The raw invoice string
    pub invoice: String,
    /// The identity public key of the invoice issuer
    pub identity_public_key: String,
    pub network: BitcoinNetwork,
    /// Optional amount denominated in sats if `token_identifier` is absent, otherwise in the token base units
    pub amount: Option<u128>,
    /// The token identifier of the token payment. Absence indicates a Bitcoin payment.
    pub token_identifier: Option<String>,
    /// Optional expiry time. If not provided, the invoice will never expire.
    pub expiry_time: Option<u64>,
    /// Optional description.
    pub description: Option<String>,
    /// If set, the invoice may only be fulfilled by a payer with this public key.
    pub sender_public_key: Option<String>,
}

impl From<breez_sdk_common::input::SparkInvoiceDetails> for SparkInvoiceDetails {
    fn from(value: breez_sdk_common::input::SparkInvoiceDetails) -> Self {
        SparkInvoiceDetails {
            invoice: value.invoice,
            identity_public_key: value.identity_public_key,
            network: value.network.into(),
            amount: value.amount,
            token_identifier: value.token_identifier,
            expiry_time: value.expiry_time,
            description: value.description,
            sender_public_key: value.sender_public_key,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LightningAddressDetails {
    pub address: String,
    pub pay_request: LnurlPayRequestDetails,
}

impl From<breez_sdk_common::input::LightningAddressDetails> for LightningAddressDetails {
    fn from(value: breez_sdk_common::input::LightningAddressDetails) -> Self {
        LightningAddressDetails {
            address: value.address,
            pay_request: value.pay_request.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentRequestSource {
    pub bip_21_uri: Option<String>,
    pub bip_353_address: Option<String>,
}

impl From<breez_sdk_common::input::PaymentRequestSource> for PaymentRequestSource {
    fn from(value: breez_sdk_common::input::PaymentRequestSource) -> Self {
        PaymentRequestSource {
            bip_21_uri: value.bip_21_uri,
            bip_353_address: value.bip_353_address,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SilentPaymentAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

impl From<breez_sdk_common::input::SilentPaymentAddressDetails> for SilentPaymentAddressDetails {
    fn from(value: breez_sdk_common::input::SilentPaymentAddressDetails) -> Self {
        SilentPaymentAddressDetails {
            address: value.address,
            network: value.network.into(),
            source: value.source.into(),
        }
    }
}

/// Configuration for an external input parser
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalInputParser {
    /// An arbitrary parser provider id
    pub provider_id: String,
    /// The external parser will be used when an input conforms to this regex
    pub input_regex: String,
    /// The URL of the parser containing a placeholder `<input>` that will be replaced with the
    /// input to be parsed. The input is sanitized using percent encoding.
    pub parser_url: String,
}

impl From<ExternalInputParser> for breez_sdk_common::input::ExternalInputParser {
    fn from(value: ExternalInputParser) -> Self {
        breez_sdk_common::input::ExternalInputParser {
            provider_id: value.provider_id,
            input_regex: value.input_regex,
            parser_url: value.parser_url,
        }
    }
}

/// Supported success action types
///
/// Receiving any other (unsupported) success action type will result in a failed parsing,
/// which will abort the LNURL-pay workflow, as per LUD-09.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "tag")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SuccessAction {
    /// AES type, described in LUD-10
    Aes {
        #[serde(flatten)]
        data: AesSuccessActionData,
    },

    /// Message type, described in LUD-09
    Message {
        #[serde(flatten)]
        data: MessageSuccessActionData,
    },

    /// URL type, described in LUD-09
    Url {
        #[serde(flatten)]
        data: UrlSuccessActionData,
    },
}

impl From<breez_sdk_common::lnurl::pay::SuccessAction> for SuccessAction {
    fn from(value: breez_sdk_common::lnurl::pay::SuccessAction) -> Self {
        match value {
            breez_sdk_common::lnurl::pay::SuccessAction::Aes { data } => {
                SuccessAction::Aes { data: data.into() }
            }
            breez_sdk_common::lnurl::pay::SuccessAction::Message { data } => {
                SuccessAction::Message { data: data.into() }
            }
            breez_sdk_common::lnurl::pay::SuccessAction::Url { data } => {
                SuccessAction::Url { data: data.into() }
            }
        }
    }
}

impl From<SuccessAction> for breez_sdk_common::lnurl::pay::SuccessAction {
    fn from(value: SuccessAction) -> Self {
        match value {
            SuccessAction::Aes { data } => {
                breez_sdk_common::lnurl::pay::SuccessAction::Aes { data: data.into() }
            }
            SuccessAction::Message { data } => {
                breez_sdk_common::lnurl::pay::SuccessAction::Message { data: data.into() }
            }
            SuccessAction::Url { data } => {
                breez_sdk_common::lnurl::pay::SuccessAction::Url { data: data.into() }
            }
        }
    }
}

/// [`SuccessAction`] where contents are ready to be consumed by the caller
///
/// Contents are identical to [`SuccessAction`], except for AES where the ciphertext is decrypted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SuccessActionProcessed {
    /// See [`SuccessAction::Aes`] for received payload
    ///
    /// See [`AesSuccessActionDataDecrypted`] for decrypted payload
    Aes { result: AesSuccessActionDataResult },

    /// See [`SuccessAction::Message`]
    Message { data: MessageSuccessActionData },

    /// See [`SuccessAction::Url`]
    Url { data: UrlSuccessActionData },
}

impl From<breez_sdk_common::lnurl::pay::SuccessActionProcessed> for SuccessActionProcessed {
    fn from(value: breez_sdk_common::lnurl::pay::SuccessActionProcessed) -> Self {
        match value {
            breez_sdk_common::lnurl::pay::SuccessActionProcessed::Aes { result } => {
                SuccessActionProcessed::Aes {
                    result: result.into(),
                }
            }
            breez_sdk_common::lnurl::pay::SuccessActionProcessed::Message { data } => {
                SuccessActionProcessed::Message { data: data.into() }
            }
            breez_sdk_common::lnurl::pay::SuccessActionProcessed::Url { data } => {
                SuccessActionProcessed::Url { data: data.into() }
            }
        }
    }
}

/// Payload of the AES success action, as received from the LNURL endpoint
///
/// See [`AesSuccessActionDataDecrypted`] for a similar wrapper containing the decrypted payload
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AesSuccessActionData {
    /// Contents description, up to 144 characters
    pub description: String,

    /// Base64, AES-encrypted data where encryption key is payment preimage, up to 4kb of characters
    pub ciphertext: String,

    /// Base64, initialization vector, exactly 24 characters
    pub iv: String,
}

impl From<breez_sdk_common::lnurl::pay::AesSuccessActionData> for AesSuccessActionData {
    fn from(value: breez_sdk_common::lnurl::pay::AesSuccessActionData) -> Self {
        AesSuccessActionData {
            description: value.description,
            ciphertext: value.ciphertext,
            iv: value.iv,
        }
    }
}

impl From<AesSuccessActionData> for breez_sdk_common::lnurl::pay::AesSuccessActionData {
    fn from(value: AesSuccessActionData) -> Self {
        breez_sdk_common::lnurl::pay::AesSuccessActionData {
            description: value.description,
            ciphertext: value.ciphertext,
            iv: value.iv,
        }
    }
}

/// Result of decryption of [`AesSuccessActionData`] payload
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AesSuccessActionDataResult {
    Decrypted { data: AesSuccessActionDataDecrypted },
    ErrorStatus { reason: String },
}

impl From<breez_sdk_common::lnurl::pay::AesSuccessActionDataResult> for AesSuccessActionDataResult {
    fn from(value: breez_sdk_common::lnurl::pay::AesSuccessActionDataResult) -> Self {
        match value {
            breez_sdk_common::lnurl::pay::AesSuccessActionDataResult::Decrypted { data } => {
                AesSuccessActionDataResult::Decrypted { data: data.into() }
            }
            breez_sdk_common::lnurl::pay::AesSuccessActionDataResult::ErrorStatus { reason } => {
                AesSuccessActionDataResult::ErrorStatus { reason }
            }
        }
    }
}

impl From<AesSuccessActionDataResult> for breez_sdk_common::lnurl::pay::AesSuccessActionDataResult {
    fn from(value: AesSuccessActionDataResult) -> Self {
        match value {
            AesSuccessActionDataResult::Decrypted { data } => {
                breez_sdk_common::lnurl::pay::AesSuccessActionDataResult::Decrypted {
                    data: data.into(),
                }
            }
            AesSuccessActionDataResult::ErrorStatus { reason } => {
                breez_sdk_common::lnurl::pay::AesSuccessActionDataResult::ErrorStatus { reason }
            }
        }
    }
}

/// Wrapper for the decrypted [`AesSuccessActionData`] payload
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AesSuccessActionDataDecrypted {
    /// Contents description, up to 144 characters
    pub description: String,

    /// Decrypted content
    pub plaintext: String,
}

impl From<breez_sdk_common::lnurl::pay::AesSuccessActionDataDecrypted>
    for AesSuccessActionDataDecrypted
{
    fn from(value: breez_sdk_common::lnurl::pay::AesSuccessActionDataDecrypted) -> Self {
        AesSuccessActionDataDecrypted {
            description: value.description,
            plaintext: value.plaintext,
        }
    }
}

impl From<AesSuccessActionDataDecrypted>
    for breez_sdk_common::lnurl::pay::AesSuccessActionDataDecrypted
{
    fn from(value: AesSuccessActionDataDecrypted) -> Self {
        breez_sdk_common::lnurl::pay::AesSuccessActionDataDecrypted {
            description: value.description,
            plaintext: value.plaintext,
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MessageSuccessActionData {
    pub message: String,
}

impl From<breez_sdk_common::lnurl::pay::MessageSuccessActionData> for MessageSuccessActionData {
    fn from(value: breez_sdk_common::lnurl::pay::MessageSuccessActionData) -> Self {
        MessageSuccessActionData {
            message: value.message,
        }
    }
}

impl From<MessageSuccessActionData> for breez_sdk_common::lnurl::pay::MessageSuccessActionData {
    fn from(value: MessageSuccessActionData) -> Self {
        breez_sdk_common::lnurl::pay::MessageSuccessActionData {
            message: value.message,
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UrlSuccessActionData {
    /// Contents description, up to 144 characters
    pub description: String,

    /// URL of the success action
    pub url: String,

    /// Indicates the success URL domain matches the LNURL callback domain.
    ///
    /// See <https://github.com/lnurl/luds/blob/luds/09.md>
    pub matches_callback_domain: bool,
}

impl From<breez_sdk_common::lnurl::pay::UrlSuccessActionData> for UrlSuccessActionData {
    fn from(value: breez_sdk_common::lnurl::pay::UrlSuccessActionData) -> Self {
        UrlSuccessActionData {
            description: value.description,
            url: value.url,
            matches_callback_domain: value.matches_callback_domain,
        }
    }
}

impl From<UrlSuccessActionData> for breez_sdk_common::lnurl::pay::UrlSuccessActionData {
    fn from(value: UrlSuccessActionData) -> Self {
        breez_sdk_common::lnurl::pay::UrlSuccessActionData {
            description: value.description,
            url: value.url,
            matches_callback_domain: value.matches_callback_domain,
        }
    }
}
