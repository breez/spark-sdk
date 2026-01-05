use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[macros::derive_from(breez_sdk_common::network::BitcoinNetwork)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum BitcoinNetwork {
    /// Mainnet
    Bitcoin,
    Testnet3,
    Testnet4,
    Signet,
    Regtest,
}

#[derive(Clone, Debug, Error)]
#[macros::derive_from(breez_sdk_common::input::ParseError)]
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

#[derive(Clone, Debug, Error)]
#[macros::derive_from(breez_sdk_common::input::Bip21Error)]
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

#[derive(Clone, Debug, Error)]
#[macros::derive_from(breez_sdk_common::lnurl::error::LnurlError)]
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

#[derive(Clone, Debug, Error)]
#[macros::derive_from(breez_sdk_common::error::ServiceConnectivityError)]
#[macros::derive_into(breez_sdk_common::error::ServiceConnectivityError)]
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[macros::derive_from(breez_sdk_common::input::Amount)]
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
#[macros::derive_from(breez_sdk_common::input::Bip21Details)]
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
#[macros::derive_from(breez_sdk_common::input::Bip21Extra)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bip21Extra {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::input::BitcoinAddressDetails)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BitcoinAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[macros::derive_from(breez_sdk_common::input::Bolt11Invoice)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11Invoice {
    pub bolt11: String,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[macros::derive_from(breez_sdk_common::input::Bolt11RouteHint)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11RouteHint {
    pub hops: Vec<Bolt11RouteHintHop>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[macros::derive_from(breez_sdk_common::input::Bolt11RouteHintHop)]
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
#[macros::derive_from(breez_sdk_common::input::Bolt12Invoice)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12Invoice {
    pub invoice: String,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::input::Bolt12InvoiceRequestDetails)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12InvoiceRequestDetails {
    // TODO: Fill fields
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[macros::derive_from(breez_sdk_common::input::Bolt12OfferBlindedPath)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12OfferBlindedPath {
    pub blinded_hops: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(breez_sdk_common::input::Bolt11InvoiceDetails)]
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
#[macros::derive_from(breez_sdk_common::input::Bolt12InvoiceDetails)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12InvoiceDetails {
    pub amount_msat: u64,
    pub invoice: Bolt12Invoice,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::input::Bolt12Offer)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12Offer {
    pub offer: String,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::input::Bolt12OfferDetails)]
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
#[macros::derive_from(breez_sdk_common::input::InputType)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::pay::LnurlPayRequestDetails)]
#[macros::derive_into(breez_sdk_common::lnurl::pay::LnurlPayRequestDetails)]
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

/// Wrapped in a [`InputType::LnurlAuth`], this is the result of [`parse`](breez_sdk_common::input::parse) when given a LNURL-auth endpoint.
///
/// It represents the endpoint's parameters for the LNURL workflow.
///
/// See <https://github.com/lnurl/luds/blob/luds/04.md>
#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::auth::LnurlAuthRequestDetails)]
#[macros::derive_into(breez_sdk_common::lnurl::auth::LnurlAuthRequestDetails)]
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

/// LNURL error details
#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::LnurlErrorDetails)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlErrorDetails {
    pub reason: String,
}

/// The response from a LNURL-auth callback, indicating success or failure.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::LnurlCallbackStatus)]
#[serde(rename_all = "UPPERCASE")]
#[serde(tag = "status")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum LnurlCallbackStatus {
    /// On-wire format is: `{"status": "OK"}`
    Ok,
    /// On-wire format is: `{"status": "ERROR", "reason": "error details..."}`
    #[serde(rename = "ERROR")]
    ErrorStatus {
        #[serde(flatten)]
        error_details: LnurlErrorDetails,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails)]
#[macros::derive_into(breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::input::SparkAddressDetails)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkAddressDetails {
    /// The raw address string
    pub address: String,
    /// The identity public key of the address owner
    pub identity_public_key: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(breez_sdk_common::input::SparkInvoiceDetails)]
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
    /// Optional expiry time as a unix timestamp in seconds. If not provided, the invoice will never expire.
    pub expiry_time: Option<u64>,
    /// Optional description.
    pub description: Option<String>,
    /// If set, the invoice may only be fulfilled by a payer with this public key.
    pub sender_public_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::input::LightningAddressDetails)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LightningAddressDetails {
    pub address: String,
    pub pay_request: LnurlPayRequestDetails,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(breez_sdk_common::input::PaymentRequestSource)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentRequestSource {
    pub bip_21_uri: Option<String>,
    pub bip_353_address: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::input::SilentPaymentAddressDetails)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SilentPaymentAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

/// Configuration for an external input parser
#[derive(Debug, Clone, Serialize)]
#[macros::derive_from(breez_sdk_common::input::ExternalInputParser)]
#[macros::derive_into(breez_sdk_common::input::ExternalInputParser)]
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

/// Supported success action types
///
/// Receiving any other (unsupported) success action type will result in a failed parsing,
/// which will abort the LNURL-pay workflow, as per LUD-09.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::pay::SuccessAction)]
#[macros::derive_into(breez_sdk_common::lnurl::pay::SuccessAction)]
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

/// [`SuccessAction`] where contents are ready to be consumed by the caller
///
/// Contents are identical to [`SuccessAction`], except for AES where the ciphertext is decrypted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::pay::SuccessActionProcessed)]
#[macros::derive_into(breez_sdk_common::lnurl::pay::SuccessActionProcessed)]
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

/// Payload of the AES success action, as received from the LNURL endpoint
///
/// See [`AesSuccessActionDataDecrypted`] for a similar wrapper containing the decrypted payload
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::pay::AesSuccessActionData)]
#[macros::derive_into(breez_sdk_common::lnurl::pay::AesSuccessActionData)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AesSuccessActionData {
    /// Contents description, up to 144 characters
    pub description: String,

    /// Base64, AES-encrypted data where encryption key is payment preimage, up to 4kb of characters
    pub ciphertext: String,

    /// Base64, initialization vector, exactly 24 characters
    pub iv: String,
}

/// Result of decryption of [`AesSuccessActionData`] payload
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::pay::AesSuccessActionDataResult)]
#[macros::derive_into(breez_sdk_common::lnurl::pay::AesSuccessActionDataResult)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AesSuccessActionDataResult {
    Decrypted { data: AesSuccessActionDataDecrypted },
    ErrorStatus { reason: String },
}

/// Wrapper for the decrypted [`AesSuccessActionData`] payload
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::pay::AesSuccessActionDataDecrypted)]
#[macros::derive_into(breez_sdk_common::lnurl::pay::AesSuccessActionDataDecrypted)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AesSuccessActionDataDecrypted {
    /// Contents description, up to 144 characters
    pub description: String,

    /// Decrypted content
    pub plaintext: String,
}

#[derive(PartialEq, Eq, Debug, Clone, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::pay::MessageSuccessActionData)]
#[macros::derive_into(breez_sdk_common::lnurl::pay::MessageSuccessActionData)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MessageSuccessActionData {
    pub message: String,
}

#[derive(PartialEq, Eq, Debug, Clone, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::lnurl::pay::UrlSuccessActionData)]
#[macros::derive_into(breez_sdk_common::lnurl::pay::UrlSuccessActionData)]
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
