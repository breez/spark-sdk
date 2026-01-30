use flashnet::{BTC_ASSET_ADDRESS, Pool};
use serde::{Deserialize, Serialize};

use crate::SdkError;

/// Default maximum slippage for conversions in basis points (0.1%)
pub const DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS: u32 = 10;
/// Default timeout for conversion operations in seconds
pub const DEFAULT_CONVERSION_TIMEOUT_SECS: u32 = 30;
/// Default integrator pubkey used when executing conversions
pub const DEFAULT_INTEGRATOR_PUBKEY: &str =
    "037e26d9d62e0b3df2d3e66805f61de2a33914465297abf76817296a92ac3f2379";
/// Default integrator fee BPS used when simulating/executing conversions
pub const DEFAULT_INTEGRATOR_FEE_BPS: u32 = 5;

/// Response from estimating a conversion, used when preparing a payment that requires conversion
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize)]
pub struct ConversionEstimate {
    /// The conversion options used for the estimate
    pub options: ConversionOptions,
    /// The estimated amount to be received from the conversion
    /// Denominated in satoshis if converting from Bitcoin, otherwise in the token base units.
    pub amount: u128,
    /// The fee estimated for the conversion
    /// Denominated in satoshis if converting from Bitcoin, otherwise in the token base units.
    pub fee: u128,
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

/// The status of the conversion
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConversionStatus {
    /// The conversion was successful
    Completed,
    /// The conversion failed and no refund was made yet, which requires action by the SDK to
    /// perform the refund. This can happen if there was a failure during the conversion process.
    RefundNeeded,
    /// The conversion failed and a refund was made
    Refunded,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversionInfo {
    /// The pool id associated with the conversion
    pub pool_id: String,
    /// The conversion id shared by both sides of the conversion
    pub conversion_id: String,
    /// The status of the conversion
    pub status: ConversionStatus,
    /// The fee paid for the conversion
    /// Denominated in satoshis if converting from Bitcoin, otherwise in the token base units.
    pub fee: Option<u128>,
    /// The purpose of the conversion
    pub purpose: Option<ConversionPurpose>,
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
