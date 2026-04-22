pub(crate) mod adaptors;
pub mod payment_observer;
pub use payment_observer::*;

// Re-export public conversion types from the conversion module
pub use crate::token_conversion::{
    AmountAdjustmentReason, ConversionEstimate, ConversionInfo, ConversionOptions,
    ConversionPurpose, ConversionStatus, ConversionType, FetchConversionLimitsRequest,
    FetchConversionLimitsResponse,
};

use core::fmt;
use lnurl_models::RecoverLnurlPayResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    str::FromStr,
};

use crate::{
    BitcoinAddressDetails, BitcoinChainService, BitcoinNetwork, Bolt11InvoiceDetails,
    ExternalInputParser, FiatCurrency, LnurlPayRequestDetails, LnurlWithdrawRequestDetails, Rate,
    SdkError, SparkInvoiceDetails, SuccessAction, SuccessActionProcessed,
    cross_chain::{CrossChainProviderContext, CrossChainRoutePair},
    error::DepositClaimError,
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

impl Seed {
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

impl Payment {
    /// Returns `true` if this payment is a child of a conversion operation.
    ///
    /// Conversion operations (stable balance, ongoing sends) create internal child
    /// payments (send sats→Flashnet, receive tokens). These are identified by having
    /// `conversion_info` set in their payment details.
    pub fn is_conversion_child(&self) -> bool {
        matches!(
            &self.details,
            Some(
                PaymentDetails::Spark {
                    conversion_info: Some(_),
                    ..
                } | PaymentDetails::Token {
                    conversion_info: Some(_),
                    ..
                }
            )
        )
    }
}

/// Outlines the steps involved in one or more conversions on a payment.
///
/// Built progressively: `status` is available immediately from payment metadata,
/// while `conversions` are enriched later from child payments and conversion info.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConversionDetails {
    /// Overall status of the conversion (persisted in storage)
    pub status: ConversionStatus,
    /// Ordered list of conversion steps. For sends: [AMM, cross-chain].
    /// For receives: [cross-chain, AMM]. Rebuilt on retrieval, not persisted.
    #[serde(default)]
    pub conversions: Vec<Conversion>,
}

/// The provider that performed a conversion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ConversionProvider {
    /// AMM (Flashnet pool) conversion between token and BTC on Spark
    Amm,
    /// Orchestra cross-chain conversion
    Orchestra,
    /// Boltz reverse-swap cross-chain conversion
    Boltz,
}

/// One side (source or destination) of a conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConversionSide {
    /// The chain or network (e.g. "spark", "lightning", "solana", "base")
    pub chain: String,
    /// The asset ticker (e.g. "BTC", "USDB", "USDC", "USDT")
    pub asset: String,
    /// Amount in base units (satoshis or token base units)
    pub amount: u128,
    /// Fee in the same base units
    pub fee: u128,
    /// Number of decimals for the asset (e.g. 6 for USDC/USDT/USDB).
    /// None for BTC sides where sats formatting is used.
    pub decimals: Option<u32>,
}

/// A single conversion in a payment's conversion chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Conversion {
    /// The provider that performed this conversion
    pub provider: ConversionProvider,
    /// Status of this specific conversion step
    pub status: ConversionStatus,
    /// Source side of the conversion
    pub from: ConversionSide,
    /// Destination side of the conversion
    pub to: ConversionSide,
    /// Reason the conversion amount was adjusted, if applicable (AMM only)
    #[serde(default)]
    pub amount_adjustment: Option<AmountAdjustmentReason>,
}

/// Components extracted from a payment's details for building a conversion side:
/// (chain, asset, decimals, `conversion_info`)
type SideInfo<'a> = (String, String, Option<u32>, Option<&'a ConversionInfo>);

/// Extracts chain, asset, decimals, and conversion info from a payment's details.
fn extract_side_info(payment: &Payment) -> Result<SideInfo<'_>, SdkError> {
    match &payment.details {
        Some(PaymentDetails::Token {
            metadata,
            conversion_info,
            ..
        }) => Ok((
            "spark".to_string(),
            metadata.ticker.clone(),
            Some(metadata.decimals),
            conversion_info.as_ref(),
        )),
        Some(PaymentDetails::Spark {
            conversion_info, ..
        }) => Ok((
            "spark".to_string(),
            "BTC".to_string(),
            None,
            conversion_info.as_ref(),
        )),
        Some(PaymentDetails::Lightning {
            conversion_info, ..
        }) => Ok((
            "lightning".to_string(),
            "BTC".to_string(),
            None,
            conversion_info.as_ref(),
        )),
        _ => Err(SdkError::Generic(format!(
            "Unsupported payment details for conversion side on payment {}",
            payment.id
        ))),
    }
}

/// Builds an AMM conversion from a send/receive child payment pair.
pub fn build_amm_conversion(send: &Payment, recv: &Payment) -> Result<Conversion, SdkError> {
    let (from_chain, from_asset, from_decimals, from_info) = extract_side_info(send)?;
    let (to_chain, to_asset, to_decimals, to_info) = extract_side_info(recv)?;

    // Each side gets its own conversion fee from its own ConversionInfo
    let from_conv_fee = from_info.and_then(ConversionInfo::fee).unwrap_or(0);
    let to_conv_fee = to_info.and_then(ConversionInfo::fee).unwrap_or(0);

    // Status and amount_adjustment from whichever side has AMM info
    let amm_info = from_info
        .filter(|i| i.is_amm())
        .or_else(|| to_info.filter(|i| i.is_amm()));
    let (status, amount_adjustment) = match amm_info {
        Some(ConversionInfo::Amm {
            status,
            amount_adjustment,
            ..
        }) => (status.clone(), amount_adjustment.clone()),
        _ => (ConversionStatus::Completed, None),
    };

    Ok(Conversion {
        provider: ConversionProvider::Amm,
        status,
        from: ConversionSide {
            chain: from_chain,
            asset: from_asset,
            amount: send.amount,
            fee: send.fees.saturating_add(from_conv_fee),
            decimals: from_decimals,
        },
        to: ConversionSide {
            chain: to_chain,
            asset: to_asset,
            amount: recv.amount,
            fee: recv.fees.saturating_add(to_conv_fee),
            decimals: to_decimals,
        },
        amount_adjustment,
    })
}

/// Builds a cross-chain conversion from an Orchestra or Boltz `ConversionInfo`.
/// Returns None for AMM conversion info (handled separately via child payments).
pub fn build_crosschain_conversion(
    info: &ConversionInfo,
    source_payment: &Payment,
) -> Option<Conversion> {
    /// Default decimals for cross-chain destination assets when not stored
    /// (pre-migration records). All current cross-chain assets (USDC, USDT) use 6 decimals.
    const DEFAULT_CROSS_CHAIN_DECIMALS: u32 = 6;

    let (from_chain, from_asset, from_decimals, _) = extract_side_info(source_payment).ok()?;

    match info {
        ConversionInfo::Orchestra {
            chain,
            asset,
            estimated_out,
            delivered_amount,
            status,
            fee,
            asset_decimals,
            ..
        } => Some(Conversion {
            provider: ConversionProvider::Orchestra,
            status: status.clone(),
            from: ConversionSide {
                chain: from_chain,
                asset: from_asset,
                amount: source_payment.amount,
                fee: 0,
                decimals: from_decimals,
            },
            to: ConversionSide {
                chain: chain.clone(),
                asset: asset.clone(),
                amount: delivered_amount.unwrap_or(*estimated_out),
                fee: fee.unwrap_or(0),
                decimals: Some(asset_decimals.unwrap_or(DEFAULT_CROSS_CHAIN_DECIMALS)),
            },
            amount_adjustment: None,
        }),
        ConversionInfo::Boltz {
            chain,
            asset,
            invoice_amount_sats,
            estimated_out,
            delivered_amount,
            status,
            fee,
            asset_decimals,
            ..
        } => Some(Conversion {
            provider: ConversionProvider::Boltz,
            status: status.clone(),
            from: ConversionSide {
                chain: from_chain,
                asset: from_asset,
                amount: u128::from(*invoice_amount_sats),
                fee: fee.unwrap_or(0),
                decimals: from_decimals,
            },
            to: ConversionSide {
                chain: chain.clone(),
                asset: asset.clone(),
                amount: delivered_amount.unwrap_or(*estimated_out),
                fee: 0,
                decimals: Some(asset_decimals.unwrap_or(DEFAULT_CROSS_CHAIN_DECIMALS)),
            },
            amount_adjustment: None,
        }),
        ConversionInfo::Amm { .. } => None,
    }
}

#[cfg(feature = "uniffi")]
uniffi::custom_type!(u128, String, {
    remote,
    try_lift: |val| val.parse::<u128>().map_err(uniffi::deps::anyhow::Error::msg),
    lower: |obj| obj.to_string(),
});

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
        /// Represents the Bolt11/Bolt12 invoice associated with a payment
        /// In the case of a Send payment, this is the invoice paid by the user
        /// In the case of a Receive payment, this is the invoice paid to the user
        invoice: String,

        /// The invoice destination/payee pubkey
        destination_pubkey: String,

        /// The HTLC transfer details
        htlc_details: SparkHtlcDetails,

        /// Lnurl payment information if this was an lnurl payment.
        lnurl_pay_info: Option<LnurlPayInfo>,

        /// Lnurl withdrawal information if this was an lnurl payment.
        lnurl_withdraw_info: Option<LnurlWithdrawInfo>,

        /// Lnurl receive information if this was a received lnurl payment.
        lnurl_receive_metadata: Option<LnurlReceiveMetadata>,

        /// The information for a conversion — populated when this Lightning
        /// payment is the source leg of a cross-chain conversion (e.g. a
        /// Boltz reverse swap paying a hold invoice).
        conversion_info: Option<ConversionInfo>,
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
#[allow(clippy::struct_excessive_bools)]
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

    /// Configuration for leaf optimization.
    ///
    /// Leaf optimization controls the denominations of leaves that are held in the wallet.
    /// Fewer, bigger leaves allow for more funds to be exited unilaterally.
    /// More leaves allow payments to be made without needing a swap, reducing payment latency.
    pub optimization_config: OptimizationConfig,

    /// Configuration for automatic conversion of Bitcoin to stable tokens.
    ///
    /// When set, received sats will be automatically converted to the specified token
    /// once the balance exceeds the threshold.
    pub stable_balance_config: Option<StableBalanceConfig>,

    /// Maximum number of concurrent transfer claims.
    ///
    /// Default is 4. Increase for server environments with high incoming payment volume.
    pub max_concurrent_claims: u32,

    /// Optional custom Spark environment configuration.
    ///
    /// When set, overrides the default Spark operator pool, service provider,
    /// threshold, and token settings. Use this to connect to alternative Spark
    /// deployments (e.g. dev/staging environments).
    pub spark_config: Option<SparkConfig>,

    /// Whether cross-chain providers (Orchestra and Boltz) are enabled.
    ///
    /// When `true` (default on mainnet) the SDK enables cross-chain sends
    /// (sats → USDT on external chains) via Orchestra and Boltz. On regtest
    /// the flag has no effect since no provider is available.
    pub cross_chain_enabled: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OptimizationConfig {
    /// Whether automatic leaf optimization is enabled.
    ///
    /// If set to true, the SDK will automatically optimize the leaf set when it changes.
    /// Otherwise, the manual optimization API must be used to optimize the leaf set.
    ///
    /// Default value is true.
    pub auto_enabled: bool,
    /// The desired multiplicity for the leaf set.
    ///
    /// Setting this to 0 will optimize for maximizing unilateral exit.
    /// Higher values will optimize for minimizing transfer swaps, with higher values
    /// being more aggressive and allowing better TPS rates.
    ///
    /// For end-user wallets, values of 1-5 are recommended. Values above 5 are
    /// intended for high-throughput server environments and are not recommended
    /// for end-user wallets due to significantly higher unilateral exit costs.
    ///
    /// Default value is 1.
    pub multiplicity: u8,
}

/// A stable token that can be used for automatic balance conversion.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StableBalanceToken {
    /// Integrator-defined display label for the token, e.g. "USD".
    ///
    /// This is a short, human-readable name set by the integrator for display purposes.
    /// It is **not** a canonical Spark token ticker — it has no protocol-level meaning.
    /// Labels must be unique within the [`StableBalanceConfig::tokens`] list.
    pub label: String,

    /// The full token identifier string used for conversions.
    pub token_identifier: String,
}

/// Configuration for automatic conversion of Bitcoin to stable tokens.
///
/// When configured, the SDK automatically monitors the Bitcoin balance after each
/// wallet sync. When the balance exceeds the configured threshold plus the reserved
/// amount, the SDK automatically converts the excess balance (above the reserve)
/// to the active stable token.
///
/// When the balance is held in a stable token, Bitcoin payments can still be sent.
/// The SDK automatically detects when there's not enough Bitcoin balance to cover a
/// payment and auto-populates the token-to-Bitcoin conversion options to facilitate
/// the payment.
///
/// The active token can be changed at runtime via [`UpdateUserSettingsRequest`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StableBalanceConfig {
    /// Available tokens that can be used for stable balance.
    pub tokens: Vec<StableBalanceToken>,

    /// The label of the token to activate by default.
    ///
    /// If `None`, stable balance starts deactivated. The user can activate it
    /// at runtime via [`UpdateUserSettingsRequest`]. If a user setting is cached
    /// locally, it takes precedence over this default.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub default_active_label: Option<String>,

    /// The minimum sats balance that triggers auto-conversion.
    ///
    /// If not provided, uses the minimum from conversion limits.
    /// If provided but less than the conversion limit minimum, the limit minimum is used.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub threshold_sats: Option<u64>,

    /// Maximum slippage in basis points (1/100 of a percent).
    ///
    /// Defaults to 10 bps (0.1%) if not set.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub max_slippage_bps: Option<u32>,
}

/// Specifies how to update the active stable balance token.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum StableBalanceActiveLabel {
    /// Activate stable balance with the given label.
    Set { label: String },
    /// Deactivate stable balance.
    Unset,
}

/// Configuration for a custom Spark environment.
///
/// When set on [`Config`], overrides the default Spark operator pool,
/// service provider, threshold, and token settings. This allows connecting
/// to alternative Spark deployments (e.g. dev/staging environments).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkConfig {
    /// Hex-encoded identifier of the coordinator operator.
    pub coordinator_identifier: String,
    /// The FROST signing threshold (e.g. 2 of 3).
    pub threshold: u32,
    /// The set of signing operators.
    pub signing_operators: Vec<SparkSigningOperator>,
    /// Service provider (SSP) configuration.
    pub ssp_config: SparkSspConfig,
    /// Expected bond amount in sats for token withdrawals.
    pub expected_withdraw_bond_sats: u64,
    /// Expected relative block locktime for token withdrawals.
    pub expected_withdraw_relative_block_locktime: u64,
}

/// A Spark signing operator.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkSigningOperator {
    /// Sequential operator ID (0-indexed).
    pub id: u32,
    /// Hex-encoded 32-byte FROST identifier.
    pub identifier: String,
    /// gRPC address of the operator (e.g. `https://0.spark.lightspark.com`).
    pub address: String,
    /// Hex-encoded compressed public key of the operator.
    pub identity_public_key: String,
}

/// Configuration for the Spark Service Provider (SSP).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SparkSspConfig {
    /// Base URL of the SSP GraphQL API.
    pub base_url: String,
    /// Hex-encoded compressed public key of the SSP.
    pub identity_public_key: String,
    /// Optional GraphQL schema endpoint path (e.g. "graphql/spark/rc").
    /// Defaults to the hardcoded schema endpoint if not set.
    pub schema_endpoint: Option<String>,
}

impl Config {
    /// Validates the configuration.
    ///
    /// Returns an error if any configuration values are invalid.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.max_concurrent_claims == 0 {
            return Err(SdkError::InvalidInput(
                "max_concurrent_claims must be greater than 0".to_string(),
            ));
        }

        if let Some(sb) = &self.stable_balance_config {
            if sb.tokens.is_empty() {
                return Err(SdkError::InvalidInput(
                    "tokens must not be empty".to_string(),
                ));
            }

            let mut seen_labels = HashSet::new();
            let mut seen_identifiers = HashSet::new();
            for token in &sb.tokens {
                if token.label.is_empty() {
                    return Err(SdkError::InvalidInput(
                        "token label must not be empty".to_string(),
                    ));
                }
                if token.token_identifier.is_empty() {
                    return Err(SdkError::InvalidInput(
                        "token_identifier must not be empty".to_string(),
                    ));
                }
                if !seen_labels.insert(&token.label) {
                    return Err(SdkError::InvalidInput(format!(
                        "tokens contains duplicate label: {}",
                        token.label
                    )));
                }
                if !seen_identifiers.insert(&token.token_identifier) {
                    return Err(SdkError::InvalidInput(format!(
                        "tokens contains duplicate token_identifier: {}",
                        token.token_identifier
                    )));
                }
            }

            if let Some(bps) = sb.max_slippage_bps
                && bps > 10000
            {
                return Err(SdkError::InvalidInput(
                    "max_slippage_bps must be <= 10000".to_string(),
                ));
            }

            if let Some(default_label) = &sb.default_active_label
                && !seen_labels.contains(default_label)
            {
                return Err(SdkError::InvalidInput(format!(
                    "default_active_label '{default_label}' not found in tokens list"
                )));
            }
        }

        Ok(())
    }

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
    pub is_mature: bool,
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

/// The available providers for buying Bitcoin
/// Request to buy Bitcoin using an external provider.
///
/// Each variant carries only the parameters relevant to that provider.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum BuyBitcoinRequest {
    /// `MoonPay`: Fiat-to-Bitcoin via credit card, Apple Pay, etc.
    /// Uses an on-chain deposit address.
    Moonpay {
        /// Lock the purchase to a specific amount in satoshis.
        locked_amount_sat: Option<u64>,
        /// Custom redirect URL after purchase completion.
        redirect_url: Option<String>,
    },
    /// `CashApp`: Pay via the Lightning Network.
    /// Generates a bolt11 invoice and returns a `cash.app` deep link.
    /// Only available on mainnet.
    CashApp {
        /// Amount in satoshis for the Lightning invoice.
        amount_sats: Option<u64>,
    },
}

impl Default for BuyBitcoinRequest {
    fn default() -> Self {
        Self::Moonpay {
            locked_amount_sat: None,
            redirect_url: None,
        }
    }
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
    BitcoinAddress {
        /// If true, rotate to a new deposit address. Previous ones remain valid.
        /// If false or absent, return the existing address (creating one if none
        /// exists yet).
        new_address: Option<bool>,
    },
    Bolt11Invoice {
        description: String,
        amount_sats: Option<u64>,
        /// The expiry of the invoice as a duration in seconds
        expiry_secs: Option<u32>,
        /// If set, creates a HODL invoice with this payment hash (hex-encoded).
        /// The payer's HTLC will be held until the preimage is provided via
        /// `claim_htlc_payment` or the HTLC expires.
        payment_hash: Option<String>,
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
    /// A cross-chain send via a bridge/swap provider.
    CrossChainAddress {
        /// The route selected for this cross-chain send (includes provider, chain, asset).
        route: CrossChainRoutePair,
        /// Raw destination address (e.g. `0xabc...`).
        recipient_address: String,
        /// Amount (in source base units) the user must transfer.
        amount_in: u128,
        /// Estimated amount the recipient will receive in the destination
        /// asset's base units. Already nets out any destination-chain costs
        /// (e.g. gas, bridge messaging fees): those are reflected in the gap
        /// between `amount_in` and `estimated_out` rather than in `fee_amount`.
        estimated_out: u128,
        /// Sender-side service fee charged by the provider, in `fee_asset`
        /// base units. Does **not** include destination-chain costs (gas,
        /// bridge messaging, etc.), which are already deducted from
        /// `estimated_out`.
        fee_amount: u128,
        /// The asset the fee is denominated in (e.g. "USDC", "USDB"). `None` means BTC (sats).
        fee_asset: Option<String>,
        /// ISO8601 timestamp after which this quote is no longer valid.
        expires_at: String,
        /// Provider-internal state produced by `prepareSendPayment` and
        /// required by `sendPayment`. Callers should round-trip this value
        /// as-is.
        provider_context: CrossChainProviderContext,
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
    /// The amount to send. Denominated in satoshis, or in token base units
    /// when `token_identifier` is set.
    pub amount: u128,
    pub pay_request: LnurlPayRequestDetails,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub comment: Option<String>,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub validate_success_action_url: Option<bool>,
    /// The token identifier when sending a token amount with conversion.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub token_identifier: Option<String>,
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
    /// The amount for the payment, always denominated in sats, even when a
    /// `token_identifier` and conversion are present.
    /// When a conversion is present, the token input amount is available in
    /// `conversion_estimate.amount_in`.
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

/// The payment destination. Either a raw string (bolt11, spark address, BIP-21,
/// cross-chain URI, etc.) that is parsed internally, or a structured
/// cross-chain destination with explicit chain + asset selection.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentRequest {
    /// Unparsed user input string (bolt11, spark address, BIP-21, cross-chain URI, etc.)
    Input { input: String },
    /// Cross-chain send with a selected route from `get_cross_chain_routes()`.
    /// Amount comes from `PrepareSendPaymentRequest.amount`, not here.
    CrossChain {
        address: String,
        route: CrossChainRoutePair,
    },
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareSendPaymentRequest {
    pub payment_request: PaymentRequest,
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
    /// The amount to be sent, denominated in satoshis for Bitcoin payments
    /// (including token-to-Bitcoin conversions), or token base units for token payments.
    /// When a conversion is present, the input amount is in
    /// `conversion_estimate.amount_in`.
    pub amount: u128,
    /// Optional token identifier for token payments.
    /// Absence indicates that the payment is a Bitcoin payment.
    pub token_identifier: Option<String>,
    /// When set, the payment will include a conversion step before sending the payment
    pub conversion_estimate: Option<ConversionEstimate>,
    /// How fees are handled for this payment.
    pub fee_policy: FeePolicy,
}

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
    Lightning {
        /// Filter specific Spark HTLC statuses
        htlc_status: Option<Vec<SparkHtlcStatus>>,
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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

    /// The label of the currently active stable balance token, or `None` if deactivated.
    pub stable_balance_active_label: Option<String>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UpdateUserSettingsRequest {
    pub spark_private_mode_enabled: Option<bool>,

    /// Update the active stable balance token. `None` means no change.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub stable_balance_active_label: Option<StableBalanceActiveLabel>,
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

/// A contact entry containing a name and payment identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Contact {
    pub id: String,
    pub name: String,
    /// A Lightning address (user@domain).
    pub payment_identifier: String,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Request to add a new contact.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AddContactRequest {
    pub name: String,
    /// A Lightning address (user@domain).
    pub payment_identifier: String,
}

/// Request to update an existing contact.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UpdateContactRequest {
    pub id: String,
    pub name: String,
    /// A Lightning address (user@domain).
    pub payment_identifier: String,
}

/// Request to list contacts with optional pagination.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListContactsRequest {
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub offset: Option<u32>,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub limit: Option<u32>,
}

/// The type of event that triggers a webhook notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(clippy::enum_variant_names)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum WebhookEventType {
    /// Triggered when a Lightning receive operation completes.
    LightningReceiveFinished,
    /// Triggered when a Lightning send operation completes.
    LightningSendFinished,
    /// Triggered when a cooperative exit completes.
    CoopExitFinished,
    /// Triggered when a static deposit completes.
    StaticDepositFinished,
    /// An event type not yet recognized by this version of the SDK.
    Unknown(String),
}

/// A registered webhook entry.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Webhook {
    /// Unique identifier for this webhook.
    pub id: String,
    /// The URL that receives webhook notifications.
    pub url: String,
    /// The event types this webhook is subscribed to.
    pub event_types: Vec<WebhookEventType>,
}

/// Request to register a new webhook.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisterWebhookRequest {
    /// The URL that will receive webhook notifications.
    pub url: String,
    /// A secret used for HMAC-SHA256 signature verification of webhook payloads.
    pub secret: String,
    /// The event types to subscribe to.
    pub event_types: Vec<WebhookEventType>,
}

/// Response from registering a webhook.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisterWebhookResponse {
    /// The unique identifier of the newly registered webhook.
    pub webhook_id: String,
}

/// Request to unregister an existing webhook.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UnregisterWebhookRequest {
    /// The unique identifier of the webhook to unregister.
    pub webhook_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SparkHtlcDetails, SparkHtlcStatus};

    fn test_token_metadata() -> TokenMetadata {
        TokenMetadata {
            identifier: "token123".to_string(),
            issuer_public_key: "02abcdef".to_string(),
            name: "USD Balance".to_string(),
            ticker: "USDB".to_string(),
            decimals: 6,
            max_supply: 21_000_000,
            is_freezable: false,
        }
    }

    fn amm_info(status: ConversionStatus, fee: u128) -> ConversionInfo {
        ConversionInfo::Amm {
            pool_id: "pool_1".to_string(),
            conversion_id: "conv_1".to_string(),
            status,
            fee: Some(fee),
            purpose: None,
            amount_adjustment: None,
        }
    }

    fn amm_info_with_adjustment(adjustment: AmountAdjustmentReason) -> ConversionInfo {
        ConversionInfo::Amm {
            pool_id: "pool_1".to_string(),
            conversion_id: "conv_1".to_string(),
            status: ConversionStatus::Completed,
            fee: Some(10),
            purpose: None,
            amount_adjustment: Some(adjustment),
        }
    }

    fn test_htlc_details() -> SparkHtlcDetails {
        SparkHtlcDetails {
            payment_hash: "hash123".to_string(),
            preimage: None,
            expiry_time: 0,
            status: SparkHtlcStatus::PreimageShared,
        }
    }

    fn token_payment(
        id: &str,
        ptype: PaymentType,
        amount: u128,
        fees: u128,
        info: ConversionInfo,
    ) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type: ptype,
            status: PaymentStatus::Completed,
            amount,
            fees,
            timestamp: 1000,
            method: PaymentMethod::Token,
            details: Some(PaymentDetails::Token {
                metadata: test_token_metadata(),
                tx_hash: "tx_1".to_string(),
                tx_type: TokenTransactionType::Transfer,
                invoice_details: None,
                conversion_info: Some(info),
            }),
            conversion_details: None,
        }
    }

    fn spark_payment(
        id: &str,
        ptype: PaymentType,
        amount: u128,
        fees: u128,
        info: ConversionInfo,
    ) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type: ptype,
            status: PaymentStatus::Completed,
            amount,
            fees,
            timestamp: 1000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
                htlc_details: None,
                conversion_info: Some(info),
            }),
            conversion_details: None,
        }
    }

    fn lightning_payment_with_info(
        id: &str,
        amount: u128,
        fees: u128,
        info: ConversionInfo,
    ) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount,
            fees,
            timestamp: 1000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                description: None,
                invoice: "lnbc1000n1p".to_string(),
                destination_pubkey: "02abc".to_string(),
                htlc_details: test_htlc_details(),
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
                lnurl_receive_metadata: None,
                conversion_info: Some(info),
            }),
            conversion_details: None,
        }
    }

    fn orchestra_info() -> ConversionInfo {
        ConversionInfo::Orchestra {
            order_id: "ord_1".to_string(),
            quote_id: "q_1".to_string(),
            chain: "base".to_string(),
            asset: "USDC".to_string(),
            recipient_address: "0x1234".to_string(),
            estimated_out: 99_500_000,
            delivered_amount: None,
            status: ConversionStatus::Pending,
            fee: Some(500),
            read_token: None,
            asset_decimals: Some(6),
        }
    }

    fn boltz_info(delivered: Option<u128>) -> ConversionInfo {
        ConversionInfo::Boltz {
            swap_id: "swap_1".to_string(),
            chain: "solana".to_string(),
            asset: "USDT".to_string(),
            recipient_address: "So1ana".to_string(),
            invoice: "lnbc1000n1p".to_string(),
            invoice_amount_sats: 100_000,
            estimated_out: 1_450_000,
            delivered_amount: delivered,
            lz_guid: None,
            status: ConversionStatus::Completed,
            fee: Some(1_500),
            max_slippage_bps: 100,
            quote_degraded: false,
            asset_decimals: Some(6),
        }
    }

    // --- build_amm_conversion tests ---

    #[test]
    fn amm_token_to_btc() {
        let send = token_payment(
            "s1",
            PaymentType::Send,
            1_500_000,
            10,
            amm_info(ConversionStatus::Completed, 21),
        );
        let recv = spark_payment(
            "r1",
            PaymentType::Receive,
            1_500,
            0,
            amm_info(ConversionStatus::Completed, 0),
        );

        let conv = build_amm_conversion(&send, &recv).unwrap();
        assert_eq!(conv.provider, ConversionProvider::Amm);
        assert_eq!(conv.from.chain, "spark");
        assert_eq!(conv.from.asset, "USDB");
        assert_eq!(conv.from.amount, 1_500_000);
        assert_eq!(conv.from.fee, 31); // 10 (payment) + 21 (conversion)
        assert!(conv.from.decimals.is_some());
        assert_eq!(conv.to.chain, "spark");
        assert_eq!(conv.to.asset, "BTC");
        assert_eq!(conv.to.amount, 1_500);
        assert_eq!(conv.to.fee, 0);
        assert!(conv.to.decimals.is_none());
        assert!(conv.amount_adjustment.is_none());
    }

    #[test]
    fn amm_btc_to_token() {
        let send = spark_payment(
            "s1",
            PaymentType::Send,
            1_500,
            5,
            amm_info(ConversionStatus::Completed, 0),
        );
        let recv = token_payment(
            "r1",
            PaymentType::Receive,
            1_500_000,
            0,
            amm_info(ConversionStatus::Completed, 21),
        );

        let conv = build_amm_conversion(&send, &recv).unwrap();
        assert_eq!(conv.from.chain, "spark");
        assert_eq!(conv.from.asset, "BTC");
        assert_eq!(conv.from.amount, 1_500);
        assert_eq!(conv.from.fee, 5);
        assert!(conv.from.decimals.is_none());
        assert_eq!(conv.to.chain, "spark");
        assert_eq!(conv.to.asset, "USDB");
        assert_eq!(conv.to.amount, 1_500_000);
        assert!(conv.to.decimals.is_some());
    }

    #[test]
    fn amm_with_amount_adjustment() {
        let send = token_payment(
            "s1",
            PaymentType::Send,
            1_500_000,
            0,
            amm_info_with_adjustment(AmountAdjustmentReason::FlooredToMinLimit),
        );
        let recv = spark_payment(
            "r1",
            PaymentType::Receive,
            1_500,
            0,
            amm_info(ConversionStatus::Completed, 0),
        );

        let conv = build_amm_conversion(&send, &recv).unwrap();
        assert_eq!(
            conv.amount_adjustment,
            Some(AmountAdjustmentReason::FlooredToMinLimit)
        );
    }

    #[test]
    fn amm_fees_combined() {
        let send = token_payment(
            "s1",
            PaymentType::Send,
            1_000_000,
            10,
            amm_info(ConversionStatus::Completed, 21),
        );
        let recv = spark_payment(
            "r1",
            PaymentType::Receive,
            1_000,
            5,
            amm_info(ConversionStatus::Completed, 0),
        );

        let conv = build_amm_conversion(&send, &recv).unwrap();
        assert_eq!(conv.from.fee, 31); // 10 + 21
        assert_eq!(conv.to.fee, 5);
    }

    // --- build_crosschain_conversion tests ---

    #[test]
    fn orchestra_from_spark() {
        let info = orchestra_info();
        let payment = spark_payment("p1", PaymentType::Send, 100_000, 0, info.clone());

        let conv = build_crosschain_conversion(&info, &payment).unwrap();
        assert_eq!(conv.provider, ConversionProvider::Orchestra);
        assert_eq!(conv.status, ConversionStatus::Pending);
        assert_eq!(conv.from.chain, "spark");
        assert_eq!(conv.from.asset, "BTC");
        assert_eq!(conv.from.amount, 100_000);
        assert_eq!(conv.from.fee, 0);
        assert_eq!(conv.to.chain, "base");
        assert_eq!(conv.to.asset, "USDC");
        assert_eq!(conv.to.amount, 99_500_000);
        assert_eq!(conv.to.fee, 500);
        assert_eq!(conv.to.decimals, Some(6));
    }

    #[test]
    fn boltz_from_lightning() {
        let info = boltz_info(None);
        let payment = lightning_payment_with_info("p1", 100_000, 3, info.clone());

        let conv = build_crosschain_conversion(&info, &payment).unwrap();
        assert_eq!(conv.provider, ConversionProvider::Boltz);
        assert_eq!(conv.status, ConversionStatus::Completed);
        assert_eq!(conv.from.chain, "lightning");
        assert_eq!(conv.from.asset, "BTC");
        assert_eq!(conv.from.amount, 100_000); // invoice_amount_sats
        assert_eq!(conv.from.fee, 1_500);
        assert_eq!(conv.to.chain, "solana");
        assert_eq!(conv.to.asset, "USDT");
        assert_eq!(conv.to.amount, 1_450_000); // estimated_out (no delivered)
    }

    #[test]
    fn boltz_with_delivered_amount() {
        let info = boltz_info(Some(1_440_000));
        let payment = lightning_payment_with_info("p1", 100_000, 3, info.clone());

        let conv = build_crosschain_conversion(&info, &payment).unwrap();
        assert_eq!(conv.to.amount, 1_440_000); // uses delivered_amount
    }

    #[test]
    fn amm_info_returns_none() {
        let info = amm_info(ConversionStatus::Completed, 0);
        let payment = spark_payment("p1", PaymentType::Send, 1_000, 0, info.clone());

        assert!(build_crosschain_conversion(&info, &payment).is_none());
    }
}
