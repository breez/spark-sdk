mod error;
mod flashnet;
mod middleware;
mod models;

pub use error::ConversionError;
pub(crate) use flashnet::FlashnetTokenConverter;
pub(crate) use middleware::TokenConversionMiddleware;
pub use models::*;

use std::sync::Arc;

use spark_wallet::{PublicKey, TransferId};
use tokio::sync::broadcast;

use crate::{EventEmitter, RefundPendingConversionsResponse};

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

    /// The legs of the conversion whose sent leg is the transfer
    /// `transfer_id`, recorded as completed, when the pool reports its swap
    /// ran. Sends nothing. Errors when the pool's swaps could not be listed,
    /// which is distinct from a swap that did not run.
    async fn find_completed_conversion(
        &self,
        transfer_id: &TransferId,
        purpose: &ConversionPurpose,
    ) -> Result<Option<TokenConversionResponse>, ConversionError>;

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

    /// Runs a local and a remote pass to refund any pending conversions.
    async fn refund_pending(&self) -> Result<RefundPendingConversionsResponse, ConversionError>;

    /// Runs a local pass to refund any pending conversions.
    async fn refund_local_pending(
        &self,
    ) -> Result<RefundPendingConversionsResponse, ConversionError>;

    /// Records the swap if it ran after all, and otherwise claws the input back.
    async fn settle_stranded_input(&self, input: StrandedInput);

    /// Optional requests that wake the client-mode periodic refunder.
    fn subscribe_refund_requests(&self) -> Option<broadcast::Receiver<RefundRequest>> {
        None
    }
}

/// How long the refunder waits before settling a stranded input, so the pool's
/// swap listing can catch up with a swap that ran despite the failed call.
pub(crate) const STRANDED_INPUT_SETTLE_SECS: u64 = 10;

/// A swap input a failed conversion left at the pool.
#[derive(Clone, Debug)]
pub(crate) struct StrandedInput {
    pub(crate) clawback_id: String,
    pub(crate) pool_id: PublicKey,
    pub(crate) payment_id: Option<String>,
    pub(crate) prior_info: Option<ConversionInfo>,
}

/// Work for the client-mode conversion refunder.
#[derive(Clone, Debug)]
pub(crate) enum RefundRequest {
    /// Run a pass over the payments marked `RefundNeeded`.
    Pass,
    /// Settle one stranded input, after the settle wait.
    Settle(Box<StrandedInput>),
}
