use thiserror::Error;
use tonic::Status;

use crate::session_manager::SessionManagerError;

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

impl From<Status> for OperatorRpcError {
    fn from(status: Status) -> Self {
        OperatorRpcError::Connection(Box::new(status))
    }
}

impl From<SessionManagerError> for OperatorRpcError {
    fn from(err: SessionManagerError) -> Self {
        OperatorRpcError::Authentication(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, OperatorRpcError>;
