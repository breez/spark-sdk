use thiserror::Error;
use tonic::Status;

#[derive(Error, Debug)]
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
}

impl From<Status> for OperatorRpcError {
    fn from(status: Status) -> Self {
        OperatorRpcError::Connection(Box::new(status))
    }
}

pub type Result<T> = std::result::Result<T, OperatorRpcError>;
