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

    #[error("{0}")]
    Generic(String),
}

impl From<platform_utils::HttpError> for BoltzError {
    fn from(err: platform_utils::HttpError) -> Self {
        Self::Api {
            code: err.status(),
            reason: err.to_string(),
        }
    }
}
