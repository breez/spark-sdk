use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum BoltzError {
    #[error("API error: {reason} (code: {code:?})")]
    Api { reason: String, code: Option<u16> },

    #[error("EVM error: {reason} (tx: {tx_hash:?})")]
    Evm {
        reason: String,
        tx_hash: Option<String>,
    },

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Signing error: {0}")]
    Signing(String),

    #[error("Store error: {0}")]
    Store(String),

    #[error("Swap expired: {swap_id}")]
    SwapExpired { swap_id: String },

    #[error("Swap failed: {swap_id}: {reason}")]
    SwapFailed { swap_id: String, reason: String },

    #[error("Quote expired")]
    QuoteExpired,

    #[error("Amount out of range: {amount} sats (min: {min}, max: {max})")]
    AmountOutOfRange { amount: u64, min: u64, max: u64 },

    #[error("Invalid quote: {0}")]
    InvalidQuote(String),

    #[error(
        "DEX quote degraded beyond slippage tolerance: expected {expected_usdt}, got {quoted_usdt}"
    )]
    QuoteDegradedBeyondSlippage { expected_usdt: u64, quoted_usdt: u64 },

    #[error("{0}")]
    Generic(String),
}

impl BoltzError {
    /// Returns `true` if Boltz rejected swap creation because the preimage hash
    /// was already used by a previous swap.
    ///
    /// Current error: HTTP 400, `{"error":"a swap with this preimage hash exists already"}`
    pub fn is_duplicate_preimage(&self) -> bool {
        matches!(self, Self::Api { code: Some(400), reason }
            if reason.to_lowercase().contains("preimage hash"))
    }
}

impl From<platform_utils::HttpError> for BoltzError {
    fn from(err: platform_utils::HttpError) -> Self {
        Self::Api {
            code: err.status(),
            reason: err.to_string(),
        }
    }
}
