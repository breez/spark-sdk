mod error;
mod flashnet;
mod middleware;
mod models;

pub use error::ConversionError;
pub(crate) use flashnet::FlashnetTokenConverter;
pub(crate) use middleware::TokenConversionMiddleware;
pub use models::*;

use spark_wallet::TransferId;

/// Trait for conversion implementations.
///
/// This trait abstracts the conversion mechanics, allowing different
/// implementations (e.g., Flashnet) to be used interchangeably.
/// Business logic for when/how much to convert is handled by `StableBalance`.
#[macros::async_trait]
pub(crate) trait TokenConverter: Send + Sync {
    /// Execute a conversion swap.
    ///
    /// # Arguments
    /// * `options` - The conversion options including type and slippage
    /// * `purpose` - The purpose of the conversion
    /// * `token_identifier` - Optional token identifier for `FromBitcoin` conversions
    /// * `amount` - Either the minimum output amount or exact input amount
    /// * `transfer_id` - Optional transfer ID for idempotency
    async fn convert(
        &self,
        options: &ConversionOptions,
        purpose: &ConversionPurpose,
        token_identifier: Option<&String>,
        amount: ConversionAmount,
        transfer_id: Option<TransferId>,
    ) -> Result<TokenConversionResponse, ConversionError>;

    /// Validate a conversion and return the estimated conversion.
    ///
    /// Called during `prepare_send_payment` to calculate the conversion fee,
    /// and during auto-conversion to estimate the token output.
    ///
    /// # Arguments
    /// * `options` - The conversion options to validate
    /// * `token_identifier` - Optional token identifier for `FromBitcoin` conversions
    /// * `amount` - Either the minimum output amount or exact input amount
    ///
    /// # Returns
    /// The estimated conversion including amount and fee, or None if options is None.
    /// For `MinAmountOut`: `estimate.amount` is the required input amount.
    /// For `AmountIn`: `estimate.amount` is the estimated output amount (slippage-adjusted).
    async fn validate(
        &self,
        options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        amount: ConversionAmount,
    ) -> Result<Option<ConversionEstimate>, ConversionError>;

    /// Fetch conversion limits for a given conversion type.
    ///
    /// # Arguments
    /// * `request` - The request containing conversion type and optional token identifier
    async fn fetch_limits(
        &self,
        request: &FetchConversionLimitsRequest,
    ) -> Result<FetchConversionLimitsResponse, ConversionError>;
}
