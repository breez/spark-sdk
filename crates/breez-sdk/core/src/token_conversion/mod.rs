mod error;
mod flashnet;
mod models;

use tokio::sync::OwnedMutexGuard;

pub use error::ConversionError;
pub(crate) use flashnet::FlashnetTokenConverter;
pub use models::*;

/// Guard that holds the conversion lock. When dropped, releases the lock
/// allowing other conversions to proceed.
///
/// This allows the SDK to perform a conversion followed by a payment while
/// holding the lock, preventing race conditions with auto-conversion.
pub(crate) struct ConversionGuard {
    _guard: OwnedMutexGuard<()>,
    pub(crate) response: TokenConversionResponse,
}

impl ConversionGuard {
    pub(crate) fn new(guard: OwnedMutexGuard<()>, response: TokenConversionResponse) -> Self {
        Self {
            _guard: guard,
            response,
        }
    }
}

/// Trait for conversion implementations.
///
/// This trait abstracts the conversion logic, allowing different
/// implementations (e.g., Flashnet) to be used interchangeably.
#[macros::async_trait]
pub(crate) trait TokenConverter: Send + Sync {
    /// Attempt auto-conversion of sats to stable tokens if configured and threshold is met.
    ///
    /// On first call, fetches conversion limits and caches the effective threshold.
    /// Subsequent calls use the cached threshold without network calls.
    ///
    /// # Arguments
    /// * `balance_sats` - The current sats balance to potentially convert
    ///
    /// # Returns
    /// * `Ok(true)` - If conversion was performed
    /// * `Ok(false)` - If skipped (not configured, threshold not met, etc.)
    /// * `ConversionError` - if there is an error in the conversion
    async fn auto_convert(&self, balance_sats: u64) -> Result<bool, ConversionError>;

    /// Execute a conversion swap, returning a guard that holds the conversion lock.
    ///
    /// The guard prevents concurrent conversions (e.g., auto-conversion racing with
    /// an ongoing payment conversion). The lock is released when the guard is dropped.
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
    ) -> Result<ConversionGuard, ConversionError>;

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
