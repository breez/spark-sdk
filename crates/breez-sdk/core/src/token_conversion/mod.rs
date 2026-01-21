mod error;
mod flashnet;
mod models;

pub use error::ConversionError;
pub(crate) use flashnet::FlashnetTokenConverter;
pub use models::*;

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
    async fn convert(
        &self,
        options: &ConversionOptions,
        purpose: &ConversionPurpose,
        token_identifier: Option<&String>,
        amount: ConversionAmount,
    ) -> Result<TokenConversionResponse, ConversionError>;

    /// Validate a conversion and return the estimated conversion.
    ///
    /// Called during `prepare_send_payment` to calculate the conversion fee.
    ///
    /// # Arguments
    /// * `options` - The conversion options to validate
    /// * `token_identifier` - Optional token identifier for `FromBitcoin` conversions
    /// * `amount_out` - The amount to receive from the conversion
    ///
    /// # Returns
    /// The estimated conversion including amount and fee, or None if options is None.
    async fn validate(
        &self,
        options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        amount_out: u128,
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
