mod error;
mod flashnet;
mod middleware;
mod models;

pub use error::ConversionError;
pub(crate) use flashnet::FlashnetTokenConverter;
pub(crate) use middleware::TokenConversionMiddleware;
pub use models::*;

use std::sync::Arc;

use spark_wallet::TransferId;
use tokio::sync::broadcast;

use crate::EventEmitter;

/// Trait for conversion implementations.
///
/// This trait abstracts the conversion mechanics, allowing different
/// implementations (e.g., Flashnet) to be used interchangeably.
/// Business logic for when/how much to convert is handled by `StableBalance`.
///
/// Implementations are reachable from the `EventEmitter` (the stable balance
/// middleware holds the converter), so they must not store an emitter
/// reference; the caller passes one into [`convert`](Self::convert) instead.
#[macros::async_trait]
pub(crate) trait TokenConverter: Send + Sync {
    /// Execute a conversion swap.
    ///
    /// # Arguments
    /// * `event_emitter` - Emitter for the payment events of the swap legs
    /// * `options` - The conversion options including type and slippage
    /// * `purpose` - The purpose of the conversion
    /// * `token_identifier` - Optional token identifier for `FromBitcoin` conversions
    /// * `amount` - Either the minimum output amount or exact input amount
    /// * `transfer_id` - Optional transfer ID for idempotency
    async fn convert(
        &self,
        event_emitter: Arc<EventEmitter>,
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
    /// `estimate.amount_in` is the input amount, `estimate.amount_out` is the estimated output.
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

    /// Process any conversions whose pending refunds need to be issued.
    ///
    /// Iterates over payments marked as needing a refund and attempts to
    /// refund each one. Surfaced through `BreezSdk::refund_pending_conversions`
    /// so partners can drive this explicitly — required in server mode (where
    /// no periodic refunder runs) and available in client mode as a way to
    /// force an immediate refund pass instead of waiting for the next tick.
    async fn refund_pending(&self) -> Result<(), ConversionError>;

    /// Optional signal that wakes the client-mode periodic refunder.
    fn subscribe_refund_requests(&self) -> Option<broadcast::Receiver<()>> {
        None
    }
}
