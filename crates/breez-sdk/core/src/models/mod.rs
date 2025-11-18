pub(crate) mod adaptors;
pub mod payment_observer;
pub use payment_observer::*;

use core::fmt;
use lnurl_models::RecoverLnurlPayResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fmt::Display, str::FromStr};

use crate::{
    BitcoinAddressDetails, BitcoinNetwork, Bolt11InvoiceDetails, ExternalInputParser, FiatCurrency,
    LnurlPayRequestDetails, LnurlWithdrawRequestDetails, Rate, SparkInvoiceDetails, SuccessAction,
    SuccessActionProcessed, error::DepositClaimError,
};

/// A list of external input parsers that are used by default.
/// To opt-out, set `use_default_external_input_parsers` in [Config] to false.
pub const DEFAULT_EXTERNAL_INPUT_PARSERS: &[(&str, &str, &str)] = &[
    (
        "picknpay",
        "(.*)(za.co.electrum.picknpay)(.*)",
        "https://cryptoqr.net/.well-known/lnurlp/<input>",
    ),
    (
        "bootleggers",
        r"(.*)(wigroup\.co|yoyogroup\.co)(.*)",
        "https://cryptoqr.net/.well-known/lnurlw/<input>",
    ),
];

/// Represents the seed for wallet generation, either as a mnemonic phrase with an optional
/// passphrase or as raw entropy bytes.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Seed {
    /// A BIP-39 mnemonic phrase with an optional passphrase.
    Mnemonic {
        /// The mnemonic phrase. 12 or 24 words.
        mnemonic: String,
        /// An optional passphrase for the mnemonic.
        passphrase: Option<String>,
    },
    /// Raw entropy bytes.
    Entropy(Vec<u8>),
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConnectRequest {
    pub config: Config,
    pub seed: Seed,
    pub storage_dir: String,
}

/// The type of payment
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentType {
    /// Payment sent from this wallet
    Send,
    /// Payment received to this wallet
    Receive,
}

impl fmt::Display for PaymentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaymentType::Send => write!(f, "send"),
            PaymentType::Receive => write!(f, "receive"),
        }
    }
}

impl FromStr for PaymentType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "receive" => PaymentType::Receive,
            "send" => PaymentType::Send,
            _ => return Err(format!("invalid payment type '{s}'")),
        })
    }
}

/// The status of a payment
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentStatus {
    /// Payment is completed successfully
    Completed,
    /// Payment is in progress
    Pending,
    /// Payment has failed
    Failed,
}

impl PaymentStatus {
    /// Returns true if the payment status is final (either Completed or Failed)
    pub fn is_final(&self) -> bool {
        matches!(self, PaymentStatus::Completed | PaymentStatus::Failed)
    }
}

impl fmt::Display for PaymentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaymentStatus::Completed => write!(f, "completed"),
            PaymentStatus::Pending => write!(f, "pending"),
            PaymentStatus::Failed => write!(f, "failed"),
        }
    }
}

impl FromStr for PaymentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "completed" => PaymentStatus::Completed,
            "pending" => PaymentStatus::Pending,
            "failed" => PaymentStatus::Failed,
            _ => return Err(format!("Invalid payment status '{s}'")),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentMethod {
    Lightning,
    Spark,
    Token,
    Deposit,
    Withdraw,
    Unknown,
}

impl Display for PaymentMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaymentMethod::Lightning => write!(f, "lightning"),
            PaymentMethod::Spark => write!(f, "spark"),
            PaymentMethod::Token => write!(f, "token"),
            PaymentMethod::Deposit => write!(f, "deposit"),
            PaymentMethod::Withdraw => write!(f, "withdraw"),
            PaymentMethod::Unknown => write!(f, "unknown"),
        }
    }
}

impl FromStr for PaymentMethod {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "lightning" => Ok(PaymentMethod::Lightning),
            "spark" => Ok(PaymentMethod::Spark),
            "token" => Ok(PaymentMethod::Token),
            "deposit" => Ok(PaymentMethod::Deposit),
            "withdraw" => Ok(PaymentMethod::Withdraw),
            "unknown" => Ok(PaymentMethod::Unknown),
            _ => Err(()),
        }
    }
}

/// Represents a payment (sent or received)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Payment {
    /// Unique identifier for the payment
    pub id: String,
    /// Type of payment (send or receive)
    pub payment_type: PaymentType,
    /// Status of the payment
    pub status: PaymentStatus,
    /// Amount in satoshis or token base units
    pub amount: u128,
    /// Fee paid in satoshis or token base units
    pub fees: u128,
    /// Timestamp of when the payment was created
    pub timestamp: u64,
    /// Method of payment. Sometimes the payment details is empty so this field
    /// is used to determine the payment method.
    pub method: PaymentMethod,
    /// Details of the payment
    pub details: Option<PaymentDetails>,
}

#[cfg(feature = "uniffi")]
uniffi::custom_type!(u128, String);

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

// TODO: fix large enum variant lint - may be done by boxing lnurl_pay_info but that requires
//  some changes to the wasm bindgen macro
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentDetails {
    Spark {
        /// The invoice details if the payment fulfilled a spark invoice
        invoice_details: Option<SparkInvoicePaymentDetails>,
    },
    Token {
        metadata: TokenMetadata,
        tx_hash: String,
        /// The invoice details if the payment fulfilled a spark invoice
        invoice_details: Option<SparkInvoicePaymentDetails>,
    },
    Lightning {
        /// Represents the invoice description
        description: Option<String>,
        /// The preimage of the paid invoice (proof of payment).
        preimage: Option<String>,
        /// Represents the Bolt11/Bolt12 invoice associated with a payment
        /// In the case of a Send payment, this is the invoice paid by the user
        /// In the case of a Receive payment, this is the invoice paid to the user
        invoice: String,

        /// The payment hash of the invoice
        payment_hash: String,

        /// The invoice destination/payee pubkey
        destination_pubkey: String,

        /// Lnurl payment information if this was an lnurl payment.
        lnurl_pay_info: Option<LnurlPayInfo>,

        /// Lnurl withdrawal information if this was an lnurl payment.
        lnurl_withdraw_info: Option<LnurlWithdrawInfo>,
    },
    Withdraw {
        tx_id: String,
    },
    Deposit {
        tx_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkInvoicePaymentDetails {
    /// Represents the spark invoice description
    pub description: Option<String>,
    /// The raw spark invoice string
    pub invoice: String,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Network {
    Mainnet,
    Regtest,
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Network::Mainnet => write!(f, "Mainnet"),
            Network::Regtest => write!(f, "Regtest"),
        }
    }
}

impl From<Network> for BitcoinNetwork {
    fn from(network: Network) -> Self {
        match network {
            Network::Mainnet => BitcoinNetwork::Bitcoin,
            Network::Regtest => BitcoinNetwork::Regtest,
        }
    }
}

impl From<Network> for breez_sdk_common::network::BitcoinNetwork {
    fn from(network: Network) -> Self {
        match network {
            Network::Mainnet => breez_sdk_common::network::BitcoinNetwork::Bitcoin,
            Network::Regtest => breez_sdk_common::network::BitcoinNetwork::Regtest,
        }
    }
}

impl From<Network> for bitcoin::Network {
    fn from(network: Network) -> Self {
        match network {
            Network::Mainnet => bitcoin::Network::Bitcoin,
            Network::Regtest => bitcoin::Network::Regtest,
        }
    }
}

impl FromStr for Network {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mainnet" => Ok(Network::Mainnet),
            "regtest" => Ok(Network::Regtest),
            _ => Err("Invalid network".to_string()),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Config {
    pub api_key: Option<String>,
    pub network: Network,
    pub sync_interval_secs: u32,

    // The maximum fee that can be paid for a static deposit claim
    // If not set then any fee is allowed
    pub max_deposit_claim_fee: Option<Fee>,

    /// The domain used for receiving through lnurl-pay and lightning address.
    pub lnurl_domain: Option<String>,

    /// When this is set to `true` we will prefer to use spark payments over
    /// lightning when sending and receiving. This has the benefit of lower fees
    /// but is at the cost of privacy.
    pub prefer_spark_over_lightning: bool,

    /// A set of external input parsers that are used by [`BreezSdk::parse`](crate::sdk::BreezSdk::parse) when the input
    /// is not recognized. See [`ExternalInputParser`] for more details on how to configure
    /// external parsing.
    pub external_input_parsers: Option<Vec<ExternalInputParser>>,
    /// The SDK includes some default external input parsers
    /// ([`DEFAULT_EXTERNAL_INPUT_PARSERS`]).
    /// Set this to false in order to prevent their use.
    pub use_default_external_input_parsers: bool,

    /// Url to use for the real-time sync server. Defaults to the Breez real-time sync server.
    pub real_time_sync_server_url: Option<String>,

    /// Whether the Spark private mode is enabled by default.
    ///
    /// If set to true, the Spark private mode will be enabled on the first initialization of the SDK.
    /// If set to false, no changes will be made to the Spark private mode.
    pub private_enabled_default: bool,
}

impl Config {
    pub(crate) fn get_all_external_input_parsers(&self) -> Vec<ExternalInputParser> {
        let mut external_input_parsers = Vec::new();
        if self.use_default_external_input_parsers {
            let default_parsers = DEFAULT_EXTERNAL_INPUT_PARSERS
                .iter()
                .map(|(id, regex, url)| ExternalInputParser {
                    provider_id: (*id).to_string(),
                    input_regex: (*regex).to_string(),
                    parser_url: (*url).to_string(),
                })
                .collect::<Vec<_>>();
            external_input_parsers.extend(default_parsers);
        }
        external_input_parsers.extend(self.external_input_parsers.clone().unwrap_or_default());

        external_input_parsers
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Fee {
    // Fixed fee amount in sats
    Fixed { amount: u64 },
    // Relative fee rate in satoshis per vbyte
    Rate { sat_per_vbyte: u64 },
}

impl Fee {
    pub fn to_sats(&self, vbytes: u64) -> u64 {
        match self {
            Fee::Fixed { amount } => *amount,
            Fee::Rate { sat_per_vbyte } => sat_per_vbyte.saturating_mul(vbytes),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DepositInfo {
    pub txid: String,
    pub vout: u32,
    pub amount_sats: u64,
    pub refund_tx: Option<String>,
    pub refund_tx_id: Option<String>,
    pub claim_error: Option<DepositClaimError>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClaimDepositRequest {
    pub txid: String,
    pub vout: u32,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub max_fee: Option<Fee>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClaimDepositResponse {
    pub payment: Payment,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RefundDepositRequest {
    pub txid: String,
    pub vout: u32,
    pub destination_address: String,
    pub fee: Fee,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RefundDepositResponse {
    pub tx_id: String,
    pub tx_hex: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListUnclaimedDepositsRequest {}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListUnclaimedDepositsResponse {
    pub deposits: Vec<DepositInfo>,
}

impl std::fmt::Display for Fee {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fee::Fixed { amount } => write!(f, "Fixed: {amount}"),
            Fee::Rate { sat_per_vbyte } => write!(f, "Rate: {sat_per_vbyte}"),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Request to get the balance of the wallet
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetInfoRequest {
    pub ensure_synced: Option<bool>,
}

/// Response containing the balance of the wallet
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetInfoResponse {
    /// The balance in satoshis
    pub balance_sats: u64,
    /// The balances of the tokens in the wallet keyed by the token identifier
    pub token_balances: HashMap<String, TokenBalance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TokenBalance {
    pub balance: u128,
    pub token_metadata: TokenMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TokenMetadata {
    pub identifier: String,
    /// Hex representation of the issuer public key
    pub issuer_public_key: String,
    pub name: String,
    pub ticker: String,
    /// Number of decimals the token uses
    pub decimals: u32,
    pub max_supply: u128,
    pub is_freezable: bool,
}

/// Request to sync the wallet with the Spark network
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SyncWalletRequest {}

/// Response from synchronizing the wallet
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SyncWalletResponse {}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ReceivePaymentMethod {
    SparkAddress,
    SparkInvoice {
        /// Amount to receive. Denominated in sats if token identifier is empty, otherwise in the token base units
        amount: Option<u128>,
        /// The presence of this field indicates that the payment is for a token
        /// If empty, it is a Bitcoin payment
        token_identifier: Option<String>,
        /// The expiry time of the invoice in seconds since the Unix epoch
        expiry_time: Option<u64>,
        /// A description to embed in the invoice.
        description: Option<String>,
        /// If set, the invoice may only be fulfilled by a payer with this public key
        sender_public_key: Option<String>,
    },
    BitcoinAddress,
    Bolt11Invoice {
        description: String,
        amount_sats: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SendPaymentMethod {
    BitcoinAddress {
        address: BitcoinAddressDetails,
        fee_quote: SendOnchainFeeQuote,
    },
    Bolt11Invoice {
        invoice_details: Bolt11InvoiceDetails,
        spark_transfer_fee_sats: Option<u64>,
        lightning_fee_sats: u64,
    }, // should be replaced with the parsed invoice
    SparkAddress {
        address: String,
        /// Fee to pay for the transaction
        /// Denominated in sats if token identifier is empty, otherwise in the token base units
        fee: u128,
        /// The presence of this field indicates that the payment is for a token
        /// If empty, it is a Bitcoin payment
        token_identifier: Option<String>,
    },
    SparkInvoice {
        spark_invoice_details: SparkInvoiceDetails,
        /// Fee to pay for the transaction
        /// Denominated in sats if token identifier is empty, otherwise in the token base units
        fee: u128,
        /// The presence of this field indicates that the payment is for a token
        /// If empty, it is a Bitcoin payment
        token_identifier: Option<String>,
    },
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize)]
pub struct SendOnchainFeeQuote {
    pub id: String,
    pub expires_at: u64,
    pub speed_fast: SendOnchainSpeedFeeQuote,
    pub speed_medium: SendOnchainSpeedFeeQuote,
    pub speed_slow: SendOnchainSpeedFeeQuote,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize)]
pub struct SendOnchainSpeedFeeQuote {
    pub user_fee_sat: u64,
    pub l1_broadcast_fee_sat: u64,
}

impl SendOnchainSpeedFeeQuote {
    pub fn total_fee_sat(&self) -> u64 {
        self.user_fee_sat.saturating_add(self.l1_broadcast_fee_sat)
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ReceivePaymentRequest {
    pub payment_method: ReceivePaymentMethod,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ReceivePaymentResponse {
    pub payment_request: String,
    /// Fee to pay to receive the payment
    /// Denominated in sats or token base units
    pub fee: u128,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareLnurlPayRequest {
    pub amount_sats: u64,
    pub pay_request: LnurlPayRequestDetails,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub comment: Option<String>,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub validate_success_action_url: Option<bool>,
}

#[derive(Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareLnurlPayResponse {
    pub amount_sats: u64,
    pub comment: Option<String>,
    pub pay_request: LnurlPayRequestDetails,
    pub fee_sats: u64,
    pub invoice_details: Bolt11InvoiceDetails,
    pub success_action: Option<SuccessAction>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlPayRequest {
    pub prepare_response: PrepareLnurlPayResponse,
    /// If set, providing the same idempotency key for multiple requests will ensure that only one
    /// payment is made. If an idempotency key is re-used, the same payment will be returned.
    /// The idempotency key must be a valid UUID.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlPayResponse {
    pub payment: Payment,
    pub success_action: Option<SuccessActionProcessed>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlWithdrawRequest {
    /// The amount to withdraw in satoshis
    /// Must be within the min and max withdrawable limits
    pub amount_sats: u64,
    pub withdraw_request: LnurlWithdrawRequestDetails,
    /// If set, the function will return the payment if it is still pending after this
    /// number of seconds. If unset, the function will return immediately after
    /// initiating the LNURL withdraw.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub completion_timeout_secs: Option<u32>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlWithdrawResponse {
    /// The Lightning invoice generated for the LNURL withdraw
    pub payment_request: String,
    pub payment: Option<Payment>,
}

/// Represents the payment LNURL info
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlPayInfo {
    pub ln_address: Option<String>,
    pub comment: Option<String>,
    pub domain: Option<String>,
    pub metadata: Option<String>,
    pub processed_success_action: Option<SuccessActionProcessed>,
    pub raw_success_action: Option<SuccessAction>,
}

/// Represents the withdraw LNURL info
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlWithdrawInfo {
    pub withdraw_url: String,
}

impl LnurlPayInfo {
    pub fn extract_description(&self) -> Option<String> {
        let Some(metadata) = &self.metadata else {
            return None;
        };

        let Ok(metadata) = serde_json::from_str::<Vec<Vec<Value>>>(metadata) else {
            return None;
        };

        for arr in metadata {
            if arr.len() != 2 {
                continue;
            }
            if let (Some(key), Some(value)) = (arr[0].as_str(), arr[1].as_str())
                && key == "text/plain"
            {
                return Some(value.to_string());
            }
        }

        None
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Serialize)]
pub enum OnchainConfirmationSpeed {
    Fast,
    Medium,
    Slow,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareSendPaymentRequest {
    pub payment_request: String,
    /// Amount to send. By default is denominated in sats.
    /// If a token identifier is provided, the amount will be denominated in the token base units.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub amount: Option<u128>,
    /// If provided, the payment will be for a token
    /// May only be provided if the payment request is a spark address
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub token_identifier: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareSendPaymentResponse {
    pub payment_method: SendPaymentMethod,
    /// Amount to send. By default is denominated in sats.
    /// If a token identifier is provided, the amount will be denominated in the token base units.
    pub amount: u128,
    /// The presence of this field indicates that the payment is for a token
    /// If empty, it is a Bitcoin payment
    pub token_identifier: Option<String>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SendPaymentOptions {
    BitcoinAddress {
        confirmation_speed: OnchainConfirmationSpeed,
    },
    Bolt11Invoice {
        prefer_spark: bool,

        /// If set, the function will return the payment if it is still pending after this
        /// number of seconds. If unset, the function will return immediately after initiating the payment.
        completion_timeout_secs: Option<u32>,
    },
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SendPaymentRequest {
    pub prepare_response: PrepareSendPaymentResponse,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub options: Option<SendPaymentOptions>,
    /// The optional idempotency key for all Spark based transfers (excludes token payments).
    /// If set, providing the same idempotency key for multiple requests will ensure that only one
    /// payment is made. If an idempotency key is re-used, the same payment will be returned.
    /// The idempotency key must be a valid UUID.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SendPaymentResponse {
    pub payment: Payment,
}

/// Request to list payments with optional filters and pagination
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListPaymentsRequest {
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub type_filter: Option<Vec<PaymentType>>,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub status_filter: Option<Vec<PaymentStatus>>,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub asset_filter: Option<AssetFilter>,
    /// Only include payments created after this timestamp (inclusive)
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub from_timestamp: Option<u64>,
    /// Only include payments created before this timestamp (exclusive)
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub to_timestamp: Option<u64>,
    /// Number of records to skip
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub offset: Option<u32>,
    /// Maximum number of records to return
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub limit: Option<u32>,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub sort_ascending: Option<bool>,
}

/// A field of [`ListPaymentsRequest`] when listing payments filtered by asset
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AssetFilter {
    Bitcoin,
    Token {
        /// Optional token identifier to filter by
        token_identifier: Option<String>,
    },
}

impl FromStr for AssetFilter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "bitcoin" => AssetFilter::Bitcoin,
            "token" => AssetFilter::Token {
                token_identifier: None,
            },
            str if str.starts_with("token:") => AssetFilter::Token {
                token_identifier: Some(
                    str.split_once(':')
                        .ok_or(format!("Invalid asset filter '{s}'"))?
                        .1
                        .to_string(),
                ),
            },
            _ => return Err(format!("Invalid asset filter '{s}'")),
        })
    }
}

/// Response from listing payments
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListPaymentsResponse {
    /// The list of payments
    pub payments: Vec<Payment>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetPaymentRequest {
    pub payment_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetPaymentResponse {
    pub payment: Payment,
}

#[cfg_attr(feature = "uniffi", uniffi::export(callback_interface))]
pub trait Logger: Send + Sync {
    fn log(&self, l: LogEntry);
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LogEntry {
    pub line: String,
    pub level: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckLightningAddressRequest {
    pub username: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterLightningAddressRequest {
    pub username: String,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub description: Option<String>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Deserialize, Serialize)]
pub struct LightningAddressInfo {
    pub description: String,
    pub lightning_address: String,
    pub lnurl: String,
    pub username: String,
}

impl From<RecoverLnurlPayResponse> for LightningAddressInfo {
    fn from(resp: RecoverLnurlPayResponse) -> Self {
        Self {
            description: resp.description,
            lightning_address: resp.lightning_address,
            lnurl: resp.lnurl,
            username: resp.username,
        }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum KeySetType {
    #[default]
    Default,
    Taproot,
    NativeSegwit,
    WrappedSegwit,
    Legacy,
}

impl From<spark_wallet::KeySetType> for KeySetType {
    fn from(value: spark_wallet::KeySetType) -> Self {
        match value {
            spark_wallet::KeySetType::Default => KeySetType::Default,
            spark_wallet::KeySetType::Taproot => KeySetType::Taproot,
            spark_wallet::KeySetType::NativeSegwit => KeySetType::NativeSegwit,
            spark_wallet::KeySetType::WrappedSegwit => KeySetType::WrappedSegwit,
            spark_wallet::KeySetType::Legacy => KeySetType::Legacy,
        }
    }
}

impl From<KeySetType> for spark_wallet::KeySetType {
    fn from(value: KeySetType) -> Self {
        match value {
            KeySetType::Default => spark_wallet::KeySetType::Default,
            KeySetType::Taproot => spark_wallet::KeySetType::Taproot,
            KeySetType::NativeSegwit => spark_wallet::KeySetType::NativeSegwit,
            KeySetType::WrappedSegwit => spark_wallet::KeySetType::WrappedSegwit,
            KeySetType::Legacy => spark_wallet::KeySetType::Legacy,
        }
    }
}

/// Response from listing fiat currencies
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListFiatCurrenciesResponse {
    /// The list of fiat currencies
    pub currencies: Vec<FiatCurrency>,
}

/// Response from listing fiat rates
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListFiatRatesResponse {
    /// The list of fiat rates
    pub rates: Vec<Rate>,
}

pub(crate) enum WaitForPaymentIdentifier {
    PaymentId(String),
    PaymentRequest(String),
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetTokensMetadataRequest {
    pub token_identifiers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetTokensMetadataResponse {
    pub tokens_metadata: Vec<TokenMetadata>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SignMessageRequest {
    pub message: String,
    /// If true, the signature will be encoded in compact format instead of DER format
    pub compact: bool,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SignMessageResponse {
    pub pubkey: String,
    /// The DER or compact hex encoded signature
    pub signature: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CheckMessageRequest {
    /// The message that was signed
    pub message: String,
    /// The public key that signed the message
    pub pubkey: String,
    /// The DER or compact hex encoded signature
    pub signature: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CheckMessageResponse {
    pub is_valid: bool,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize)]
pub struct UserSettings {
    pub spark_private_mode_enabled: bool,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UpdateUserSettingsRequest {
    pub spark_private_mode_enabled: Option<bool>,
}
