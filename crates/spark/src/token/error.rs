use thiserror::Error;

use crate::{operator::rpc::OperatorRpcError, signer::SignerError};

#[derive(Debug, Error, Clone)]
pub enum TokenOutputServiceError {
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
