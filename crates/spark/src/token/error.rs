use thiserror::Error;

use crate::{operator::rpc::OperatorRpcError, signer::SignerError};

#[derive(Debug, Error, Clone)]
pub enum TokenOutputServiceError {
    #[error("insufficient funds")]
    InsufficientFunds,

    #[error("rpc error: {0}")]
    RpcError(Box<OperatorRpcError>),

    #[error("Signer error: {0}")]
    SignerError(#[from] SignerError),

    #[error("Service error: {0}")]
    ServiceError(#[from] crate::services::ServiceError),

    #[error("generic error: {0}")]
    Generic(String),
}

impl From<OperatorRpcError> for TokenOutputServiceError {
    fn from(error: OperatorRpcError) -> Self {
        TokenOutputServiceError::RpcError(Box::new(error))
    }
}

impl From<TokenOutputServiceError> for crate::services::ServiceError {
    fn from(error: TokenOutputServiceError) -> Self {
        use crate::services::ServiceError;
        match error {
            TokenOutputServiceError::InsufficientFunds => ServiceError::InsufficientFunds,
            TokenOutputServiceError::RpcError(e) => ServiceError::ServiceConnectionError(e),
            TokenOutputServiceError::SignerError(e) => ServiceError::SignerError(e),
            TokenOutputServiceError::ServiceError(e) => e,
            TokenOutputServiceError::Generic(msg) => ServiceError::Generic(msg),
        }
    }
}
