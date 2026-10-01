use thiserror::Error;
use tonic::Status;

use crate::session_store::SessionStoreError;

#[derive(Error, Debug, Clone)]
pub enum OperatorRpcError {
    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Invalid URI: {0}")]
    InvalidUri(String),

    #[error("Authentication error: {0}")]
    Authentication(String),

    #[error("Connection error: {0}")]
    Connection(Box<Status>),

    #[error("Operator not found: {0}")]
    OperatorNotFound(String),

    #[error("Unexpected error: {0}")]
    Unexpected(String),

    #[error("Signer error: {0}")]
    SignerError(#[from] crate::signer::SignerError),

    #[error("Generic: {0}")]
    Generic(String),
}

impl OperatorRpcError {
    pub fn is_unavailable(&self) -> bool {
        match self {
            OperatorRpcError::Transport(_) => true,
            OperatorRpcError::Connection(status) => is_temporary_code(status.code()),
            _ => false,
        }
    }
}

/// `Cancelled` is the client's own request timeout. `Unknown` is a transport
/// failure on the web: the operators mask an `Unknown` of their own as
/// `Internal`.
pub fn is_temporary_code(code: tonic::Code) -> bool {
    matches!(
        code,
        tonic::Code::Unavailable
            | tonic::Code::DeadlineExceeded
            | tonic::Code::ResourceExhausted
            | tonic::Code::Aborted
            | tonic::Code::Cancelled
            | tonic::Code::Unknown
    )
}

impl From<Status> for OperatorRpcError {
    fn from(status: Status) -> Self {
        OperatorRpcError::Connection(Box::new(status))
    }
}

impl From<SessionStoreError> for OperatorRpcError {
    fn from(err: SessionStoreError) -> Self {
        OperatorRpcError::Authentication(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, OperatorRpcError>;

#[cfg(test)]
mod tests {
    use tonic::{Code, Status};

    use super::*;

    #[test]
    fn only_an_unreachable_or_busy_operator_is_unavailable() {
        assert!(OperatorRpcError::Transport("refused".to_string()).is_unavailable());
        for code in [
            Code::Unavailable,
            Code::DeadlineExceeded,
            Code::Cancelled,
            Code::Unknown,
        ] {
            assert!(OperatorRpcError::from(Status::new(code, "")).is_unavailable());
        }
        assert!(!OperatorRpcError::from(Status::internal("")).is_unavailable());
        assert!(!OperatorRpcError::from(Status::permission_denied("")).is_unavailable());
        assert!(!OperatorRpcError::Authentication("bad signature".to_string()).is_unavailable());
    }
}
