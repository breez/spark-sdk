use std::fmt;
use std::str::FromStr;

use flashnet::{BTC_ASSET_ADDRESS, Pool};
use serde::{Deserialize, Serialize};

use crate::SdkError;

use crate::utils::serde_helpers::{serde_option_u128_as_string, serde_u128_as_string};

/// Default maximum slippage for conversions in basis points (0.1%)
pub const DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS: u32 = 10;
/// Default timeout for conversion operations in seconds
pub const DEFAULT_CONVERSION_TIMEOUT_SECS: u32 = 30;
/// Default integrator pubkey used when executing conversions
pub const DEFAULT_INTEGRATOR_PUBKEY: &str =
    "037e26d9d62e0b3df2d3e66805f61de2a33914465297abf76817296a92ac3f2379";
/// Default integrator fee BPS used when simulating/executing conversions
pub const DEFAULT_INTEGRATOR_FEE_BPS: u32 = 5;

/// Fee attribution for a conversion, indicating which side of the conversion
/// (sent or received) the pool fee is denominated in. The two variants are
/// mutually exclusive — a pool fee is always denominated in one asset.
pub(crate) enum FeeSplit {
    /// Fee is on the sent (outbound/`asset_in`) payment, denominated in `asset_in`.
    Sent(u128),
    /// Fee is on the received (inbound/`asset_out`) payment, denominated in `asset_out`.
    Received(u128),
}

/// Response from estimating a conversion, used when preparing a payment that requires conversion
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize)]
pub struct ConversionEstimate {
    /// The conversion options used for the estimate
    pub options: ConversionOptions,
    /// The input amount for the conversion.
    /// For `FromBitcoin`: the satoshis required to produce the desired token output.
    /// For `ToBitcoin`: the token amount being converted.
    pub amount_in: u128,
    /// The estimated output amount from the conversion.
    /// For `FromBitcoin`: the estimated token amount received.
    /// For `ToBitcoin`: the estimated satoshis received.
    pub amount_out: u128,
    /// The fee estimated for the conversion.
    /// Denominated in satoshis if converting from Bitcoin, otherwise in the token base units.
    pub fee: u128,
    /// The reason the conversion amount was adjusted, if applicable.
    pub amount_adjustment: Option<AmountAdjustmentReason>,
}

/// The purpose of the conversion, which is used to provide context for the conversion
/// if its related to an ongoing payment or a self-transfer.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConversionPurpose {
    /// Conversion is associated with an ongoing payment
    OngoingPayment {
        /// The payment request of the ongoing payment
        payment_request: String,
    },
    /// Conversion is for self-transfer
    SelfTransfer,
    /// Conversion triggered automatically
    AutoConversion,
}

/// Specifies how to determine the conversion amount.
#[derive(Debug, Clone)]
pub(crate) enum ConversionAmount {
    /// Specify the minimum output amount - the input will be calculated.
    /// Used for payment conversions where we know the required output.
    MinAmountOut(u128),
    /// Specify the exact input amount - used for auto-conversion where we know the sats balance.
    AmountIn(u128),
}

/// The reason why a conversion amount was adjusted from the originally requested value.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AmountAdjustmentReason {
    /// The amount was increased to meet the minimum conversion limit.
    FlooredToMinLimit,
    /// The amount was increased to convert the full token balance,
    /// avoiding a remaining balance below the minimum conversion limit (token dust).
    IncreasedToAvoidDust,
}

/// The status of the conversion
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConversionStatus {
    /// Conversion is in-flight (queued or started, not yet completed)
    Pending,
    /// The conversion was successful
    Completed,
    /// The conversion failed (e.g., the initial send payment failed)
    Failed,
    /// The conversion failed and no refund was made yet, which requires action by the SDK to
    /// perform the refund. This can happen if there was a failure during the conversion process.
    RefundNeeded,
    /// The conversion failed and a refund was made
    Refunded,
}

impl fmt::Display for ConversionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConversionStatus::Pending => write!(f, "pending"),
            ConversionStatus::Completed => write!(f, "completed"),
            ConversionStatus::Failed => write!(f, "failed"),
            ConversionStatus::RefundNeeded => write!(f, "refund_needed"),
            ConversionStatus::Refunded => write!(f, "refunded"),
        }
    }
}

impl FromStr for ConversionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(ConversionStatus::Pending),
            "completed" => Ok(ConversionStatus::Completed),
            "failed" => Ok(ConversionStatus::Failed),
            "refund_needed" => Ok(ConversionStatus::RefundNeeded),
            "refunded" => Ok(ConversionStatus::Refunded),
            _ => Err(format!("Invalid conversion status '{s}'")),
        }
    }
}

/// Conversion metadata stored on a payment's metadata row. Discriminated by a
/// `"type"` tag in JSON. Old data (AMM-only, no tag) is handled by the custom
/// `Deserialize` impl which defaults to `"amm"` when the tag is absent.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ConversionInfo {
    /// AMM (Flashnet pool-based) conversion — Spark ↔ Spark token swaps.
    #[serde(rename = "amm")]
    Amm {
        /// The pool id associated with the conversion
        pool_id: String,
        /// The conversion id shared by both sides of the conversion
        conversion_id: String,
        /// The status of the conversion
        status: ConversionStatus,
        /// The fee paid for the conversion.
        /// Denominated in satoshis if converting from Bitcoin, otherwise in the token base units.
        #[serde(default, with = "serde_option_u128_as_string")]
        fee: Option<u128>,
        /// The purpose of the conversion
        purpose: Option<ConversionPurpose>,
        /// The reason the conversion amount was adjusted, if applicable.
        #[serde(default)]
        amount_adjustment: Option<AmountAdjustmentReason>,
    },
    /// Orchestra cross-chain conversion via the Flashnet orchestration API.
    #[serde(rename = "orchestra")]
    Orchestra {
        /// The Orchestra order id returned by `/v1/orchestration/submit`.
        order_id: String,
        /// The Orchestra quote id used to create this order.
        quote_id: String,
        /// Opaque token required for querying order status.
        #[serde(default)]
        read_token: Option<String>,

        /// Chain name (e.g. `"base"`, `"solana"`, `"tron"`).
        chain: String,
        /// Stable chain identifier (e.g. EVM `chainId` decimal string `"8453"`
        /// for Base, SLIP-44 or similar for other chains). `None` if the
        /// provider doesn't expose one for this route.
        #[serde(default)]
        chain_id: Option<String>,
        /// Asset ticker (e.g. `"USDC"`, `"USDT"`).
        #[serde(default)]
        asset: String,
        /// Recipient address on the target chain.
        recipient_address: String,
        /// Estimated amount in the asset's base units, frozen at prepare time.
        #[serde(with = "serde_u128_as_string")]
        estimated_out: u128,
        /// Actual amount settled. `None` until the order reaches a terminal state.
        #[serde(default, with = "serde_option_u128_as_string")]
        delivered_amount: Option<u128>,
        /// Current status of the cross-chain order.
        status: ConversionStatus,
        /// Fee in the asset's base units.
        #[serde(default, with = "serde_option_u128_as_string")]
        fee: Option<u128>,
        /// Number of decimals for the asset (e.g. 6 for USDC).
        asset_decimals: u32,
        /// Token contract / mint address on the destination chain.
        /// `None` for native-asset destinations.
        #[serde(default)]
        asset_contract: Option<String>,
    },
    /// Boltz reverse swap — cross-chain conversion via Lightning hold invoice.
    ///
    /// `instance_id` and `claim_key_index` are intentionally not stored on
    /// the payment row in v1: they would only be needed for cross-device
    /// re-derivation of the preimage, which v1 does not support.
    #[serde(rename = "boltz")]
    Boltz {
        /// The Boltz swap id returned by `POST /swap/reverse`.
        swap_id: String,
        /// The BOLT11 hold invoice paid on the Spark/Lightning side.
        invoice: String,
        /// Amount of the hold invoice in sats.
        invoice_amount_sats: u64,
        /// `LayerZero` message GUID for bridged swaps.
        #[serde(default)]
        lz_guid: Option<String>,
        /// DEX slippage tolerance (basis points) committed at prepare time.
        max_slippage_bps: u32,
        /// Whether the claim-time DEX quote drifted beyond `max_slippage_bps`.
        #[serde(default)]
        quote_degraded: bool,

        /// Chain name (e.g. `"arbitrum"`, `"solana"`, `"tron"`).
        chain: String,
        /// Stable chain identifier (e.g. EVM `chainId` decimal string `"42161"`
        /// for Arbitrum). `None` if the provider doesn't expose one for this
        /// route.
        #[serde(default)]
        chain_id: Option<String>,
        /// Asset ticker (e.g. `"USDT"`, `"USDT0"`).
        #[serde(default)]
        asset: String,
        /// Recipient address on the target chain.
        recipient_address: String,
        /// Estimated amount in the asset's base units, frozen at prepare time.
        #[serde(with = "serde_u128_as_string")]
        estimated_out: u128,
        /// Actual amount delivered. `None` until the claim receipt is processed.
        #[serde(default, with = "serde_option_u128_as_string")]
        delivered_amount: Option<u128>,
        /// Current status of the reverse swap.
        status: ConversionStatus,
        /// Fee in sats (source-side for sends): Boltz spread + LN routing budget.
        #[serde(default, with = "serde_option_u128_as_string")]
        fee: Option<u128>,
        /// Number of decimals for the asset (e.g. 6 for USDT).
        asset_decimals: u32,
        /// Token contract / mint address on the destination chain.
        /// `None` for native-asset destinations.
        #[serde(default)]
        asset_contract: Option<String>,
    },
}

impl ConversionInfo {
    /// The current status, regardless of conversion type.
    pub fn status(&self) -> &ConversionStatus {
        match self {
            ConversionInfo::Amm { status, .. }
            | ConversionInfo::Orchestra { status, .. }
            | ConversionInfo::Boltz { status, .. } => status,
        }
    }

    /// A mutable reference to the status, for in-place updates.
    pub fn status_mut(&mut self) -> &mut ConversionStatus {
        match self {
            ConversionInfo::Amm { status, .. }
            | ConversionInfo::Orchestra { status, .. }
            | ConversionInfo::Boltz { status, .. } => status,
        }
    }

    /// The fee paid, regardless of conversion type.
    pub fn fee(&self) -> Option<u128> {
        match self {
            ConversionInfo::Amm { fee, .. }
            | ConversionInfo::Orchestra { fee, .. }
            | ConversionInfo::Boltz { fee, .. } => *fee,
        }
    }

    /// Whether this is an AMM (Flashnet pool) conversion.
    pub fn is_amm(&self) -> bool {
        matches!(self, ConversionInfo::Amm { .. })
    }

    /// Whether this is an Orchestra (cross-chain) conversion.
    pub fn is_orchestra(&self) -> bool {
        matches!(self, ConversionInfo::Orchestra { .. })
    }

    /// Whether this is a Boltz reverse swap.
    pub fn is_boltz(&self) -> bool {
        matches!(self, ConversionInfo::Boltz { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boltz_conversion_info_roundtrip() {
        let original = ConversionInfo::Boltz {
            swap_id: "boltz_swap_abc".to_string(),
            chain: "solana".to_string(),
            chain_id: None,
            asset: "USDT0".to_string(),
            recipient_address: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            invoice: "lnbc1000n1pexample".to_string(),
            invoice_amount_sats: 150_000,
            estimated_out: 99_000_000,
            delivered_amount: Some(98_750_000),
            lz_guid: Some("0xdeadbeef".to_string()),
            status: ConversionStatus::Pending,
            fee: Some(2_500),
            max_slippage_bps: 100,
            quote_degraded: false,
            asset_decimals: 6,
            asset_contract: Some("0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string()),
        };

        let json = serde_json::to_string(&original).unwrap();
        let decoded: ConversionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
        assert!(decoded.is_boltz());
        assert!(!decoded.is_orchestra());
        assert!(!decoded.is_amm());
        assert_eq!(decoded.status(), &ConversionStatus::Pending);
        assert_eq!(decoded.fee(), Some(2_500));

        // The `"type"` tag discriminator must match the rename attribute.
        assert!(json.contains(r#""type":"boltz""#));
        // u128 fields serialize as strings, not JSON numbers.
        assert!(json.contains(r#""estimated_out":"99000000""#));
    }

    #[test]
    fn boltz_status_mut_updates_status_in_place() {
        let mut info = ConversionInfo::Boltz {
            swap_id: "s1".to_string(),
            chain: "arbitrum".to_string(),
            chain_id: Some("42161".to_string()),
            asset: "USDT".to_string(),
            recipient_address: "0xdest".to_string(),
            invoice: "lnbc".to_string(),
            invoice_amount_sats: 100,
            estimated_out: 1,
            delivered_amount: None,
            lz_guid: None,
            status: ConversionStatus::Pending,
            fee: None,
            max_slippage_bps: 100,
            quote_degraded: false,
            asset_decimals: 6,
            asset_contract: None,
        };
        *info.status_mut() = ConversionStatus::Completed;
        assert_eq!(info.status(), &ConversionStatus::Completed);
    }
}

pub(crate) struct TokenConversionPool {
    pub(crate) asset_in_address: String,
    pub(crate) asset_out_address: String,
    pub(crate) pool: Pool,
}

pub(crate) struct TokenConversionResponse {
    /// The sent payment id for the conversion
    pub(crate) sent_payment_id: String,
    /// The received payment id for the conversion
    pub(crate) received_payment_id: String,
}

/// Options for conversion when fulfilling a payment. When set, the SDK will
/// perform a conversion before fulfilling the payment. If not set, the payment
/// will only be fulfilled if the wallet has sufficient balance of the required asset.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConversionOptions {
    /// The type of conversion to perform when fulfilling the payment
    pub conversion_type: ConversionType,
    /// The optional maximum slippage in basis points (1/100 of a percent) allowed when
    /// a conversion is needed to fulfill the payment. Defaults to 10 bps (0.1%) if not set.
    /// The conversion will fail if the actual amount received is less than
    /// `estimated_amount * (1 - max_slippage_bps / 10_000)`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub max_slippage_bps: Option<u32>,
    /// The optional timeout in seconds to wait for the conversion to complete
    /// when fulfilling the payment. This timeout only concerns waiting for the received
    /// payment of the conversion. If the timeout is reached before the conversion
    /// is complete, the payment will fail. Defaults to 30 seconds if not set.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub completion_timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ConversionType {
    /// Converting from Bitcoin to a token
    FromBitcoin,
    /// Converting from a token to Bitcoin
    ToBitcoin { from_token_identifier: String },
}

impl ConversionType {
    /// Returns the asset addresses for the conversion type
    ///
    /// # Arguments
    ///
    /// * `token_identifier` - The token identifier when converting from Bitcoin to a token
    ///
    /// # Returns
    ///
    /// Result containing:
    /// * (String, String): A tuple containing the asset in address and asset out address
    /// * `SdkError`: If the token identifier is required but not provided
    pub(crate) fn as_asset_addresses(
        &self,
        token_identifier: Option<&String>,
    ) -> Result<(String, String), SdkError> {
        Ok(match self {
            ConversionType::FromBitcoin => (
                BTC_ASSET_ADDRESS.to_string(),
                token_identifier
                    .ok_or(SdkError::InvalidInput(
                        "Token identifier is required for from Bitcoin conversion".to_string(),
                    ))?
                    .clone(),
            ),
            ConversionType::ToBitcoin {
                from_token_identifier,
            } => (from_token_identifier.clone(), BTC_ASSET_ADDRESS.to_string()),
        })
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FetchConversionLimitsRequest {
    /// The type of conversion, either from or to Bitcoin.
    pub conversion_type: ConversionType,
    /// The token identifier when converting to a token.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub token_identifier: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FetchConversionLimitsResponse {
    /// The minimum amount to be converted.
    /// Denominated in satoshis if converting from Bitcoin, otherwise in the token base units.
    pub min_from_amount: Option<u128>,
    /// The minimum amount to be received from the conversion.
    /// Denominated in satoshis if converting to Bitcoin, otherwise in the token base units.
    pub min_to_amount: Option<u128>,
}
