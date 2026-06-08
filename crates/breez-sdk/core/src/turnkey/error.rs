use thiserror::Error;

/// Errors from the Turnkey API client.
#[derive(Debug, Error)]
pub enum TurnkeyError {
    #[error("invalid Turnkey API key: {0}")]
    InvalidApiKey(String),

    #[error("failed to serialize request: {0}")]
    Serialize(String),

    #[error("failed to deserialize response: {0}")]
    Deserialize(String),

    #[error("HTTP transport error: {0}")]
    Transport(String),

    #[error("Turnkey returned HTTP {status}: {body}")]
    Http { status: u16, body: String },

    #[error("Turnkey response did not contain an activity")]
    MissingActivity,

    #[error("Turnkey activity failed: {0}")]
    ActivityFailed(String),

    #[error("Turnkey activity requires consensus/approval (id {0})")]
    ConsensusNeeded(String),

    #[error("unexpected Turnkey activity status: {0}")]
    UnexpectedStatus(String),

    #[error("Turnkey activity still pending after {0} retries")]
    ExceededRetries(u32),

    #[error("unexpected Turnkey response shape: {0}")]
    UnexpectedResponse(String),
}
