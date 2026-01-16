mod error;
mod flashnet;
mod models;

pub use error::ConversionError;
pub(crate) use flashnet::FlashnetTokenConverter;
pub use models::*;

/// Trait for conversion implementations.
///
/// This trait abstracts the conversion logic, allowing different
/// implementations (e.g., Flashnet) to be used interchangeably.
#[macros::async_trait]
pub(crate) trait TokenConverter: Send + Sync {
    /// Execute a conversion swap.
    ///
    /// Returns the sent and received payment IDs after updating payment metadata.
    ///
    /// # Arguments
    /// * `options` - The conversion options including type and slippage
    /// * `purpose` - The purpose of the conversion
    /// * `token_identifier` - Optional token identifier for `FromBitcoin` conversions
    /// * `min_amount_out` - The minimum amount to receive from the conversion
    async fn convert(
        &self,
        options: &ConversionOptions,
        purpose: &ConversionPurpose,
        token_identifier: Option<&String>,
        min_amount_out: u128,
    ) -> Result<TokenConversionResponse, ConversionError>;

    /// Validate a conversion and return the estimated conversion.
    ///
    /// Called during `prepare_send_payment` to calculate the conversion fee.
    ///
    /// # Arguments
    /// * `options` - Optional conversion options (returns None if not provided)
    /// * `token_identifier` - Optional token identifier for `FromBitcoin` conversions
    /// * `amount_out` - The amount to receive from the conversion
    ///
    /// # Returns
    /// The estimated conversion including amount and fee, or None if no conversion options provided.
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
