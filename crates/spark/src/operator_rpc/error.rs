use thiserror::Error;
use tonic::transport::Error as TonicError;

#[derive(Error, Debug)]
pub enum OperatorRpcError {
    #[error("Transport error: {0}")]
    Transport(#[from] TonicError),

    #[error("Invalid URI: {0}")]
    InvalidUri(String),

    #[error("Authentication error: {0}")]
    Authentication(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Operator not found: {0}")]
    OperatorNotFound(String),

    #[error("Unexpected error: {0}")]
    Unexpected(String),
}

pub type Result<T> = std::result::Result<T, OperatorRpcError>;
