pub(crate) mod adaptors;
pub mod payment_observer;
pub mod prepared_payment;
pub use payment_observer::*;
pub use prepared_payment::*;

// Re-export public conversion types from the conversion module
pub use crate::token_conversion::{
    ConversionEstimate, ConversionInfo, ConversionOptions, ConversionPurpose, ConversionStatus,
    ConversionType, FetchConversionLimitsRequest, FetchConversionLimitsResponse,
};

// Re-export internal types for crate use only
pub(crate) use crate::token_conversion::TokenConversionResponse;

use core::fmt;
use lnurl_models::RecoverLnurlPayResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fmt::Display, str::FromStr};

use crate::{
    BitcoinAddressDetails, BitcoinChainService, BitcoinNetwork, Bolt11InvoiceDetails,
    ExternalInputParser, FiatCurrency, LnurlPayRequestDetails, LnurlWithdrawRequestDetails, Rate,
    SdkError, SparkInvoiceDetails, SuccessAction, SuccessActionProcessed, error::DepositClaimError,
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

/// Configuration for connecting to the Breez SDK.
///
/// `api_key`, `network`, and `seed` are required. All other fields have sensible
/// defaults (identical to the legacy `default_config()` function).
///
/// # Examples
///
/// ```ignore
/// let client = Breez::connect(ClientConfig {
///     api_key: "brz_test_...".into(),
///     network: Network::Mainnet,
///     seed: Seed::Mnemonic { mnemonic: "...".into(), passphrase: None },
///     ..Default::default()
/// }).await?;
/// ```
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientConfig {
    /// Breez API key. Required for mainnet; may be empty for regtest.
    pub api_key: String,
    /// Network to connect to.
    pub network: Network,
    /// The wallet seed. Required.
    pub seed: Seed,
    /// Storage directory for this client's data. If `None`, auto-derived
    /// from the seed fingerprint under `storage_root`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub storage_dir: Option<String>,
    /// Root directory for wallet storage when `storage_dir` is `None`.
    /// Default: `"./.breez"`
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub storage_root: Option<String>,
    /// How often (in seconds) to sync wallet state.
    /// Default: 60
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub sync_interval_secs: Option<u32>,
    /// Maximum fee for automatic on-chain deposit claims.
    /// Default: `Rate { sat_per_vbyte: 1 }`
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub max_deposit_claim_fee: Option<MaxFee>,
    /// The domain for receiving via lnurl-pay and lightning address.
    /// Default: derived from network ("breez.tips" for mainnet, None for regtest).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub lnurl_domain: Option<String>,
    /// Whether to prefer Spark payments over Lightning.
    /// Default: false
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub prefer_spark_over_lightning: Option<bool>,
    /// Custom external input parsers.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub external_input_parsers: Option<Vec<ExternalInputParser>>,
    /// Whether to use the built-in default external input parsers.
    /// Default: true
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub use_default_external_input_parsers: Option<bool>,
    /// URL for the real-time sync server.
    /// Default: Breez real-time sync server.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub real_time_sync_server_url: Option<String>,
    /// Whether Spark private mode is enabled by default.
    /// Default: true
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub private_mode: Option<bool>,
    /// Leaf optimization configuration.
    /// Default: auto_enabled=true, multiplicity=1
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub leaf_optimization_config: Option<LeafOptimizationConfig>,
}

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

impl Default for Seed {
    fn default() -> Self {
        Self::Entropy(Vec::new())
    }
}

impl Seed {
    /// Create a `Seed` from a mnemonic phrase (no passphrase).
    ///
    /// # Example
    /// ```ignore
    /// let seed = Seed::mnemonic("word1 word2 word3 ...");
    /// ```
    pub fn mnemonic(mnemonic: impl Into<String>) -> Self {
        Self::Mnemonic {
            mnemonic: mnemonic.into(),
            passphrase: None,
        }
    }

    /// Create a `Seed` from a mnemonic phrase with passphrase.
    pub fn mnemonic_with_passphrase(
        mnemonic: impl Into<String>,
        passphrase: impl Into<String>,
    ) -> Self {
        Self::Mnemonic {
            mnemonic: mnemonic.into(),
            passphrase: Some(passphrase.into()),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, SdkError> {
        match self {
            Seed::Mnemonic {
                mnemonic,
                passphrase,
            } => {
                let mnemonic = bip39::Mnemonic::parse(mnemonic)
                    .map_err(|e| SdkError::Generic(e.to_string()))?;

                Ok(mnemonic
                    .to_seed(passphrase.as_deref().unwrap_or(""))
                    .to_vec())
            }
            Seed::Entropy(entropy) => Ok(entropy.clone()),
        }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConnectRequest {
    pub config: Config,
    pub seed: Seed,
    pub storage_dir: String,
}

/// Request object for connecting to the Spark network using an external signer.
///
/// This allows using a custom signer implementation instead of providing a seed directly.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConnectWithSignerRequest {
    pub config: Config,
    pub signer: std::sync::Arc<dyn crate::signer::ExternalSigner>,
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
    /// If set, this payment involved a conversion before the payment
    pub conversion_details: Option<ConversionDetails>,
}

/// Outlines the steps involved in a conversion
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConversionDetails {
    /// First step is converting from the available asset
    pub from: ConversionStep,
    /// Second step is converting to the requested asset
    pub to: ConversionStep,
}

/// Conversions have one send and one receive payment that are associated to the
/// ongoing payment via the `parent_payment_id` in the payment metadata. These payments
/// are queried from the storage by the SDK, then converted.
impl TryFrom<&Vec<Payment>> for ConversionDetails {
    type Error = SdkError;
    fn try_from(payments: &Vec<Payment>) -> Result<Self, Self::Error> {
        let from = payments
            .iter()
            .find(|p| p.payment_type == PaymentType::Send)
            .ok_or(SdkError::Generic(
                "From step of conversion not found".to_string(),
            ))?;
        let to = payments
            .iter()
            .find(|p| p.payment_type == PaymentType::Receive)
            .ok_or(SdkError::Generic(
                "To step of conversion not found".to_string(),
            ))?;
        Ok(ConversionDetails {
            from: from.try_into()?,
            to: to.try_into()?,
        })
    }
}

/// A single step in a conversion
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConversionStep {
    /// The underlying payment id of the conversion step
    pub payment_id: String,
    /// Payment amount in satoshis or token base units
    pub amount: u128,
    /// Fee paid in satoshis or token base units
    /// This represents the payment fee + the conversion fee
    pub fee: u128,
    /// Method of payment
    pub method: PaymentMethod,
    /// Token metadata if a token is used for payment
    pub token_metadata: Option<TokenMetadata>,
}

/// Converts a Spark or Token payment into a `ConversionStep`.
/// Fees are a sum of the payment fee and the conversion fee, if applicable,
/// from the payment details. Token metadata should only be set for a token payment.
impl TryFrom<&Payment> for ConversionStep {
    type Error = SdkError;
    fn try_from(payment: &Payment) -> Result<Self, Self::Error> {
        let (conversion_info, token_metadata) = match &payment.details {
            Some(PaymentDetails::Spark {
                conversion_info: Some(info),
                ..
            }) => (info, None),
            Some(PaymentDetails::Token {
                conversion_info: Some(info),
                metadata,
                ..
            }) => (info, Some(metadata.clone())),
            _ => {
                return Err(SdkError::Generic(format!(
                    "No conversion info available for payment {}",
                    payment.id
                )));
            }
        };
        Ok(ConversionStep {
            payment_id: payment.id.clone(),
            amount: payment.amount,
            fee: payment
                .fees
                .saturating_add(conversion_info.fee.unwrap_or(0)),
            method: payment.method,
            token_metadata,
        })
    }
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
        /// The HTLC transfer details if the payment fulfilled an HTLC transfer
        htlc_details: Option<SparkHtlcDetails>,
        /// The information for a conversion
        conversion_info: Option<ConversionInfo>,
    },
    Token {
        metadata: TokenMetadata,
        tx_hash: String,
        tx_type: TokenTransactionType,
        /// The invoice details if the payment fulfilled a spark invoice
        invoice_details: Option<SparkInvoicePaymentDetails>,
        /// The information for a conversion
        conversion_info: Option<ConversionInfo>,
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

        /// Lnurl receive information if this was a received lnurl payment.
        lnurl_receive_metadata: Option<LnurlReceiveMetadata>,
    },
    Withdraw {
        tx_id: String,
    },
    Deposit {
        tx_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TokenTransactionType {
    Transfer,
    Mint,
    Burn,
}

impl fmt::Display for TokenTransactionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenTransactionType::Transfer => write!(f, "transfer"),
            TokenTransactionType::Mint => write!(f, "mint"),
            TokenTransactionType::Burn => write!(f, "burn"),
        }
    }
}

impl FromStr for TokenTransactionType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "transfer" => Ok(TokenTransactionType::Transfer),
            "mint" => Ok(TokenTransactionType::Mint),
            "burn" => Ok(TokenTransactionType::Burn),
            _ => Err(format!("Invalid token transaction type '{s}'")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkInvoicePaymentDetails {
    /// Represents the spark invoice description
    pub description: Option<String>,
    /// The raw spark invoice string
    pub invoice: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkHtlcDetails {
    /// The payment hash of the HTLC
    pub payment_hash: String,
    /// The preimage of the HTLC. Empty until receiver has released it.
    pub preimage: Option<String>,
    /// The expiry time of the HTLC as a unix timestamp in seconds
    pub expiry_time: u64,
    /// The HTLC status
    pub status: SparkHtlcStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SparkHtlcStatus {
    /// The HTLC is waiting for the preimage to be shared by the receiver
    WaitingForPreimage,
    /// The HTLC preimage has been shared and the transfer can be or has been claimed by the receiver
    PreimageShared,
    /// The HTLC has been returned to the sender due to expiry
    Returned,
}

impl fmt::Display for SparkHtlcStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SparkHtlcStatus::WaitingForPreimage => write!(f, "WaitingForPreimage"),
            SparkHtlcStatus::PreimageShared => write!(f, "PreimageShared"),
            SparkHtlcStatus::Returned => write!(f, "Returned"),
        }
    }
}

impl FromStr for SparkHtlcStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "WaitingForPreimage" => Ok(SparkHtlcStatus::WaitingForPreimage),
            "PreimageShared" => Ok(SparkHtlcStatus::PreimageShared),
            "Returned" => Ok(SparkHtlcStatus::Returned),
            _ => Err("Invalid Spark HTLC status".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Network {
    #[default]
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
    pub max_deposit_claim_fee: Option<MaxFee>,

    /// The domain used for receiving through lnurl-pay and lightning address.
    pub lnurl_domain: Option<String>,

    /// When this is set to `true` we will prefer to use spark payments over
    /// lightning when sending and receiving. This has the benefit of lower fees
    /// but is at the cost of privacy.
    pub prefer_spark_over_lightning: bool,

    /// A set of external input parsers that are used by [`BreezClient::parse`](crate::sdk::BreezClient::parse) when the input
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

    /// Configuration for leaf optimization.
    ///
    /// Leaf optimization controls the denominations of leaves that are held in the wallet.
    /// Fewer, bigger leaves allow for more funds to be exited unilaterally.
    /// More leaves allow payments to be made without needing a swap, reducing payment latency.
    pub optimization_config: LeafOptimizationConfig,
}

/// Deprecated alias for [`LeafOptimizationConfig`].
#[deprecated(note = "Use `LeafOptimizationConfig` instead.")]
pub type OptimizationConfig = LeafOptimizationConfig;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LeafOptimizationConfig {
    /// Whether automatic leaf optimization is enabled.
    ///
    /// If set to true, the SDK will automatically optimize the leaf set when it changes.
    /// Otherwise, the manual optimization API must be used to optimize the leaf set.
    ///
    /// Default value is true.
    pub auto_enabled: bool,
    /// The desired multiplicity for the leaf set. Acceptable values are 0-5.
    ///
    /// Setting this to 0 will optimize for maximizing unilateral exit.
    /// Higher values will optimize for minimizing transfer swaps, with higher values
    /// being more aggressive.
    ///
    /// Default value is 1.
    pub multiplicity: u8,
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
pub enum MaxFee {
    // Fixed fee amount in sats
    Fixed { amount: u64 },
    // Relative fee rate in satoshis per vbyte
    Rate { sat_per_vbyte: u64 },
    // Fastest network recommended fee at the time of claim, with a leeway in satoshis per vbyte
    NetworkRecommended { leeway_sat_per_vbyte: u64 },
}

impl MaxFee {
    pub(crate) async fn to_fee(&self, client: &dyn BitcoinChainService) -> Result<Fee, SdkError> {
        match self {
            MaxFee::Fixed { amount } => Ok(Fee::Fixed { amount: *amount }),
            MaxFee::Rate { sat_per_vbyte } => Ok(Fee::Rate {
                sat_per_vbyte: *sat_per_vbyte,
            }),
            MaxFee::NetworkRecommended {
                leeway_sat_per_vbyte,
            } => {
                let recommended_fees = client.recommended_fees().await?;
                let max_fee_rate = recommended_fees
                    .fastest_fee
                    .saturating_add(*leeway_sat_per_vbyte);
                Ok(Fee::Rate {
                    sat_per_vbyte: max_fee_rate,
                })
            }
        }
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
    pub max_fee: Option<MaxFee>,
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

/// Request to buy Bitcoin using an external provider (`MoonPay`)
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BuyBitcoinRequest {
    /// Optional: Lock the purchase to a specific amount in satoshis.
    /// When provided, the user cannot change the amount in the purchase flow.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub locked_amount_sat: Option<u64>,
    /// Optional: Custom redirect URL after purchase completion
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub redirect_url: Option<String>,
}

/// Response containing a URL to complete the Bitcoin purchase
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BuyBitcoinResponse {
    /// The URL to open in a browser to complete the purchase
    pub url: String,
}

impl std::fmt::Display for MaxFee {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaxFee::Fixed { amount } => write!(f, "Fixed: {amount}"),
            MaxFee::Rate { sat_per_vbyte } => write!(f, "Rate: {sat_per_vbyte}"),
            MaxFee::NetworkRecommended {
                leeway_sat_per_vbyte,
            } => write!(f, "NetworkRecommended: {leeway_sat_per_vbyte}"),
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
    /// The identity public key of the wallet as a hex string
    pub identity_pubkey: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
        /// The expiry time of the invoice as a unix timestamp in seconds
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
        /// The expiry of the invoice as a duration in seconds
        expiry_secs: Option<u32>,
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
    /// The amount to send in satoshis.
    pub amount_sats: u64,
    pub pay_request: LnurlPayRequestDetails,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub comment: Option<String>,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub validate_success_action_url: Option<bool>,
    /// If provided, the payment will include a token conversion step before sending the payment
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub conversion_options: Option<ConversionOptions>,
    /// How fees should be handled. Defaults to `FeesExcluded` (fees added on top).
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub fee_policy: Option<FeePolicy>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareLnurlPayResponse {
    /// The amount to send in satoshis.
    pub amount_sats: u64,
    pub comment: Option<String>,
    pub pay_request: LnurlPayRequestDetails,
    /// The fee in satoshis. For `FeesIncluded` operations, this represents the total fee
    /// (including potential overpayment).
    pub fee_sats: u64,
    pub invoice_details: Bolt11InvoiceDetails,
    pub success_action: Option<SuccessAction>,
    /// When set, the payment will include a token conversion step before sending the payment
    pub conversion_estimate: Option<ConversionEstimate>,
    /// How fees are handled for this payment.
    pub fee_policy: FeePolicy,
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

/// Specifies how fees are handled in a payment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum FeePolicy {
    /// Fees are added on top of the specified amount (default behavior).
    /// The receiver gets the exact amount specified.
    #[default]
    FeesExcluded,
    /// Fees are deducted from the specified amount.
    /// The receiver gets the amount minus fees.
    FeesIncluded,
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
    /// The amount to send.
    /// Optional for payment requests with embedded amounts (e.g., Spark/Bolt11 invoices with amounts).
    /// Required for Spark addresses, Bitcoin addresses, and amountless invoices.
    /// Denominated in satoshis for Bitcoin payments, or token base units for token payments.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub amount: Option<u128>,
    /// Optional token identifier for token payments.
    /// Absence indicates that the payment is a Bitcoin payment.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub token_identifier: Option<String>,
    /// If provided, the payment will include a conversion step before sending the payment
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub conversion_options: Option<ConversionOptions>,
    /// How fees should be handled. Defaults to `FeesExcluded` (fees added on top).
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub fee_policy: Option<FeePolicy>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareSendPaymentResponse {
    pub payment_method: SendPaymentMethod,
    /// The amount for the payment.
    /// Denominated in satoshis for Bitcoin payments, or token base units for token payments.
    pub amount: u128,
    /// Optional token identifier for token payments.
    /// Absence indicates that the payment is a Bitcoin payment.
    pub token_identifier: Option<String>,
    /// When set, the payment will include a conversion step before sending the payment
    pub conversion_estimate: Option<ConversionEstimate>,
    /// How fees are handled for this payment.
    pub fee_policy: FeePolicy,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SendPaymentOptions {
    BitcoinAddress {
        /// Confirmation speed for the on-chain transaction.
        confirmation_speed: OnchainConfirmationSpeed,
    },
    Bolt11Invoice {
        prefer_spark: bool,

        /// If set, the function will return the payment if it is still pending after this
        /// number of seconds. If unset, the function will return immediately after initiating the payment.
        completion_timeout_secs: Option<u32>,
    },
    SparkAddress {
        /// Can only be provided for Bitcoin payments. If set, a Spark HTLC transfer will be created.
        /// The receiver will need to provide the preimage to claim it.
        htlc_options: Option<SparkHtlcOptions>,
    },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkHtlcOptions {
    /// The payment hash of the HTLC. The receiver will need to provide the associated preimage to claim it.
    pub payment_hash: String,
    /// The duration of the HTLC in seconds.
    /// After this time, the HTLC will be returned.
    pub expiry_duration_secs: u64,
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

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentDetailsFilter {
    Spark {
        /// Filter specific Spark HTLC statuses
        htlc_status: Option<Vec<SparkHtlcStatus>>,
        /// Filter conversion payments with refund information
        conversion_refund_needed: Option<bool>,
    },
    Token {
        /// Filter conversion payments with refund information
        conversion_refund_needed: Option<bool>,
        /// Filter by transaction hash
        tx_hash: Option<String>,
        /// Filter by transaction type
        tx_type: Option<TokenTransactionType>,
    },
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
    /// Only include payments matching at least one of these payment details filters
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub payment_details_filter: Option<Vec<PaymentDetailsFilter>>,
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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LnurlInfo {
    pub url: String,
    pub bech32: String,
}

impl LnurlInfo {
    pub fn new(url: String) -> Self {
        let bech32 =
            breez_sdk_common::lnurl::encode_lnurl_to_bech32(&url).unwrap_or_else(|_| url.clone());
        Self { url, bech32 }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Deserialize, Serialize)]
pub struct LightningAddressInfo {
    pub description: String,
    pub lightning_address: String,
    pub lnurl: LnurlInfo,
    pub username: String,
}

impl From<RecoverLnurlPayResponse> for LightningAddressInfo {
    fn from(resp: RecoverLnurlPayResponse) -> Self {
        Self {
            description: resp.description,
            lightning_address: resp.lightning_address,
            lnurl: LnurlInfo::new(resp.lnurl),
            username: resp.username,
        }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Configuration for key set derivation.
///
/// This struct encapsulates the parameters needed for BIP32 key derivation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct KeySetConfig {
    /// The key set type which determines the derivation path
    pub key_set_type: KeySetType,
    /// Controls the structure of the BIP derivation path
    pub use_address_index: bool,
    /// Optional account number for key derivation
    pub account_number: Option<u32>,
}

impl Default for KeySetConfig {
    fn default() -> Self {
        Self {
            key_set_type: KeySetType::Default,
            use_address_index: false,
            account_number: None,
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

/// The operational status of a Spark service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ServiceStatus {
    /// Service is fully operational.
    Operational,
    /// Service is experiencing degraded performance.
    Degraded,
    /// Service is partially unavailable.
    Partial,
    /// Service status is unknown.
    Unknown,
    /// Service is experiencing a major outage.
    Major,
}

/// The status of the Spark network services relevant to the SDK.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkStatus {
    /// The worst status across all relevant services.
    pub status: ServiceStatus,
    /// The last time the status was updated, as a unix timestamp in seconds.
    pub last_updated: u64,
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

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClaimHtlcPaymentRequest {
    pub preimage: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClaimHtlcPaymentResponse {
    pub payment: Payment,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlReceiveMetadata {
    pub nostr_zap_request: Option<String>,
    pub nostr_zap_receipt: Option<String>,
    pub sender_comment: Option<String>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OptimizationProgress {
    pub is_running: bool,
    pub current_round: u32,
    pub total_rounds: u32,
}

// ---------------------------------------------------------------------------
//  Unified Payment API types (Phase 1 of SDK Modernization)
// ---------------------------------------------------------------------------

/// The destination for a payment: either a raw string or an already-parsed [`InputType`].
///
/// For most destinations (Bolt11 invoices, Bitcoin addresses, Spark addresses),
/// a raw string is accepted and parsed internally by `prepare_payment()`.
///
/// **LNURL-Pay and Lightning Address** destinations **must** use the `Parsed`
/// variant with an [`InputType`](crate::InputType) from a prior `parse()` call.
/// This is required by the [LNURL spec (LUD-06)](https://github.com/lnurl/luds/blob/luds/06.md):
/// the wallet must discover and display the service metadata (min/max sendable,
/// description, comment constraints) to the user before selecting an amount.
/// Passing a raw LNURL/Lightning address string to `prepare_payment()` returns an error.
///
/// # Examples
///
/// ```ignore
/// // Bolt11 invoice: pass raw string directly
/// let payment = client.prepare_payment("lnbc1...".into(), None).await?;
///
/// // LNURL-Pay / Lightning Address: must parse first (LUD-06)
/// let input = client.parse("user@domain.com").await?;
/// // Show min/max sendable, description to user …
/// let payment = client.prepare_payment(input.into(), Some(options)).await?;
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentDestination {
    /// A raw destination string (invoice, address, etc.).
    /// Will be parsed internally by `prepare_payment()`.
    ///
    /// **Note:** LNURL-Pay and Lightning Address strings are rejected in this
    /// form — use `Parsed` with a pre-parsed [`InputType`](crate::InputType) instead.
    Raw { destination: String },
    /// An already-parsed input from a prior `parse()` / `parseInput()` call.
    /// Required for LNURL-Pay and Lightning Address destinations.
    Parsed { input: crate::InputType },
}

impl From<String> for PaymentDestination {
    fn from(s: String) -> Self {
        Self::Raw { destination: s }
    }
}

impl From<&str> for PaymentDestination {
    fn from(s: &str) -> Self {
        Self::Raw {
            destination: s.to_string(),
        }
    }
}

impl From<crate::InputType> for PaymentDestination {
    fn from(input: crate::InputType) -> Self {
        Self::Parsed { input }
    }
}

/// Options for preparing a payment via [`prepare_payment()`](crate::BreezClient::prepare_payment).
///
/// All fields are optional – callers only need to supply the fields relevant
/// to the destination they parsed with `parse()`.
///
/// For the amount, set **one** of `amount_sats` or `amount_token_units`:
/// - `amount_sats` — for Bitcoin (satoshi) payments.
/// - `amount_token_units` — for token payments (requires `token_identifier`).
///
/// Setting both is an error.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareOptions {
    /// Amount to send, in satoshis. Use this for Bitcoin payments (Lightning,
    /// on-chain, Spark address, Spark invoice without a token).
    /// Optional for payment requests with embedded amounts (e.g., invoices with amounts).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub amount_sats: Option<u64>,

    /// Amount to send, in token base units. Use this for token payments.
    /// Requires `token_identifier` to be set.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub amount_token_units: Option<u128>,

    /// Optional token identifier for token payments.
    /// Absence indicates a Bitcoin payment.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub token_identifier: Option<String>,

    /// If provided, the payment will include a conversion step before sending
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub conversion_options: Option<ConversionOptions>,

    /// How fees should be handled. Defaults to `FeesExcluded` (fees added on top).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub fee_policy: Option<FeePolicy>,

    /// Options specific to LNURL-Pay / Lightning Address destinations.
    /// Ignored for non-LNURL destinations.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub lnurl: Option<LnurlPayOptions>,
}

impl PrepareOptions {
    /// Resolve the amount into the single `Option<u128>` expected by the
    /// legacy `PrepareSendPaymentRequest`.
    ///
    /// Returns an error if both `amount_sats` and `amount_token_units` are set,
    /// or if `amount_token_units` is set without `token_identifier`.
    pub(crate) fn unified_amount(&self) -> Result<Option<u128>, SdkError> {
        match (self.amount_sats, self.amount_token_units) {
            (Some(_), Some(_)) => Err(SdkError::InvalidInput(
                "Cannot set both amount_sats and amount_token_units".to_string(),
            )),
            (Some(sats), None) => Ok(Some(u128::from(sats))),
            (None, Some(token_units)) => {
                if self.token_identifier.is_none() {
                    return Err(SdkError::InvalidInput(
                        "amount_token_units requires token_identifier to be set".to_string(),
                    ));
                }
                Ok(Some(token_units))
            }
            (None, None) => Ok(None),
        }
    }
}

/// Options specific to LNURL-Pay and Lightning Address payments.
///
/// Pass this in [`PrepareOptions::lnurl`] when the destination is an
/// LNURL-Pay URL or Lightning address.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlPayOptions {
    /// Comment to attach to the LNURL-Pay request.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub comment: Option<String>,

    /// Whether to validate the URL in an LNURL success action.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub validate_success_action_url: Option<bool>,
}

/// Options for confirming a prepared payment.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PayOptions {
    /// Idempotency key for all Spark-based transfers.
    /// Must be a valid UUID.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub idempotency_key: Option<String>,

    /// Method-specific send options (e.g., on-chain confirmation speed).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub send_options: Option<SendPaymentOptions>,
}

/// A unified, user-friendly fee representation extracted from a prepared payment.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PreparedPaymentFee {
    /// Fee for a Spark Bitcoin transfer, denominated in satoshis.
    SparkSats { fee_sats: u64 },
    /// Fee for a Spark token transfer, denominated in token base units.
    SparkToken { fee_token_units: u128 },
    /// Fee for a Lightning (Bolt11) payment.
    Bolt11Invoice { fee_sats: u64 },
    /// Fee for an on-chain (Bitcoin address) payment, with speed tiers.
    BitcoinAddress {
        speed_fast: OnchainSpeedFee,
        speed_medium: OnchainSpeedFee,
        speed_slow: OnchainSpeedFee,
    },
}

/// On-chain fee for a specific confirmation speed tier.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnchainSpeedFee {
    pub total_fee_sat: u64,
}

impl PreparedPaymentFee {
    /// Returns the fee in satoshis.
    ///
    /// For `SparkToken` payments this returns 0, since the fee is in token units.
    /// Use [`fee_token_units()`](Self::fee_token_units) for those.
    /// For on-chain payments, returns the medium-speed tier fee by default.
    /// Use the variant fields directly if you need a specific speed tier.
    pub fn fee_sats(&self) -> u64 {
        match self {
            Self::SparkSats { fee_sats } | Self::Bolt11Invoice { fee_sats } => *fee_sats,
            Self::SparkToken { .. } => 0,
            Self::BitcoinAddress { speed_medium, .. } => speed_medium.total_fee_sat,
        }
    }

    /// Returns the fee in token base units, if this is a token payment.
    ///
    /// Returns `None` for non-token payments (`SparkSats`, `Bolt11Invoice`, `BitcoinAddress`).
    pub fn fee_token_units(&self) -> Option<u128> {
        match self {
            Self::SparkToken { fee_token_units } => Some(*fee_token_units),
            _ => None,
        }
    }

    /// Extracts the fee from an existing `SendPaymentMethod`.
    pub(crate) fn from_send_payment_method(method: &SendPaymentMethod) -> Self {
        match method {
            SendPaymentMethod::SparkAddress {
                fee,
                token_identifier,
                ..
            }
            | SendPaymentMethod::SparkInvoice {
                fee,
                token_identifier,
                ..
            } => {
                if token_identifier.is_some() {
                    PreparedPaymentFee::SparkToken {
                        fee_token_units: *fee,
                    }
                } else {
                    PreparedPaymentFee::SparkSats {
                        fee_sats: *fee as u64,
                    }
                }
            }
            SendPaymentMethod::Bolt11Invoice {
                lightning_fee_sats, ..
            } => PreparedPaymentFee::Bolt11Invoice {
                fee_sats: *lightning_fee_sats,
            },
            SendPaymentMethod::BitcoinAddress { fee_quote, .. } => PreparedPaymentFee::BitcoinAddress {
                speed_fast: OnchainSpeedFee {
                    total_fee_sat: fee_quote.speed_fast.total_fee_sat(),
                },
                speed_medium: OnchainSpeedFee {
                    total_fee_sat: fee_quote.speed_medium.total_fee_sat(),
                },
                speed_slow: OnchainSpeedFee {
                    total_fee_sat: fee_quote.speed_slow.total_fee_sat(),
                },
            },
        }
    }

    /// Extracts the fee from a `PrepareLnurlPayResponse`.
    pub(crate) fn from_lnurl_prepare(response: &PrepareLnurlPayResponse) -> Self {
        PreparedPaymentFee::Bolt11Invoice {
            fee_sats: response.fee_sats,
        }
    }
}

/// The data backing a `PreparedPayment`.
///
/// Callers should treat this as opaque — it is public only so that binding
/// layers (WASM, Flutter) can decompose and re-wrap `PreparedPayment` with
/// a different smart-pointer type.
#[derive(Debug, Clone)]
pub enum PreparedPaymentData {
    /// A standard prepare → send flow (Spark address, Spark invoice, Bolt11, Bitcoin address).
    Standard(PrepareSendPaymentResponse),
    /// An LNURL-Pay / Lightning Address flow.
    Lnurl(PrepareLnurlPayResponse),
}

// ---------------------------------------------------------------------------
//  Unified Receive API types
// ---------------------------------------------------------------------------

/// The type of payment to receive.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ReceivePaymentType {
    /// Receive via Lightning (Bolt11 invoice).
    #[default]
    Bolt11Invoice,
    /// Receive via on-chain Bitcoin address.
    BitcoinAddress,
    /// Receive via Spark address (persistent).
    SparkAddress,
    /// Receive via Spark invoice.
    SparkInvoice,
}

/// Simplified options for receiving a payment via the unified `receive()` API.
///
/// Only supply the fields relevant to the chosen `payment_type`.
///
/// For the amount, set **one** of `amount_sats` or `amount_token_units`:
/// - `amount_sats` — for Bitcoin (satoshi) payments (Lightning, on-chain, Spark address/invoice).
/// - `amount_token_units` — for Spark invoice token payments (requires `token_identifier`).
///
/// Setting both is an error.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ReceiveOptions {
    /// The type of payment to receive. Defaults to `Bolt11Invoice`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub payment_type: Option<ReceivePaymentType>,

    /// Amount to receive, in satoshis. Use this for Lightning invoices,
    /// on-chain, Spark address, or Spark invoice (Bitcoin) payments.
    /// Optional for SparkAddress and BitcoinAddress (they don't embed amounts).
    /// Required for Lightning.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub amount_sats: Option<u64>,

    /// Amount to receive, in token base units. Use this for Spark invoice
    /// token payments. Requires `token_identifier` to be set.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub amount_token_units: Option<u128>,

    /// Description to embed in the invoice.
    /// Used by Lightning and SparkInvoice payment types.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub description: Option<String>,

    /// Expiry duration in seconds (for Lightning invoices) or
    /// expiry unix timestamp (for Spark invoices).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub expiry: Option<u64>,

    /// Token identifier for Spark invoice token payments.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub token_identifier: Option<String>,

    /// If set, the Spark invoice may only be fulfilled by a payer with this public key.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub sender_public_key: Option<String>,
}

impl ReceiveOptions {
    /// Resolve the amount into the single `Option<u128>` expected by the
    /// legacy `ReceivePaymentMethod`.
    ///
    /// Returns an error if both `amount_sats` and `amount_token_units` are set,
    /// or if `amount_token_units` is set without `token_identifier`.
    pub(crate) fn unified_amount(&self) -> Result<Option<u128>, SdkError> {
        match (self.amount_sats, self.amount_token_units) {
            (Some(_), Some(_)) => Err(SdkError::InvalidInput(
                "Cannot set both amount_sats and amount_token_units".to_string(),
            )),
            (Some(sats), None) => Ok(Some(u128::from(sats))),
            (None, Some(token_units)) => {
                if self.token_identifier.is_none() {
                    return Err(SdkError::InvalidInput(
                        "amount_token_units requires token_identifier to be set".to_string(),
                    ));
                }
                Ok(Some(token_units))
            }
            (None, None) => Ok(None),
        }
    }
}

/// The result of a `receive()` call.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ReceiveResult {
    /// The payment request string (invoice, address, etc.)
    pub destination: String,
    /// Fee to pay to receive the payment, in satoshis (usually 0).
    pub fee_sats: u64,
    /// Fee to pay to receive a token payment, in token base units.
    /// `None` for non-token payments.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub fee_token_units: Option<u128>,
}
