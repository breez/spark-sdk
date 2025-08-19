use thiserror::Error;

use crate::{operator::rpc::OperatorRpcError, signer::SignerError};

#[derive(Debug, Error, Clone)]
pub enum TreeServiceError {
    #[error("insufficient funds")]
    InsufficientFunds,

    #[error("rpc error: {0}")]
    RpcError(Box<OperatorRpcError>),

    #[error("unselectable amount")]
    UnselectableAmount,

    #[error("invalid amount")]
    InvalidAmount,

    #[error("Signer error: {0}")]
    SignerError(#[from] SignerError),

    #[error("non reservable leaves")]
    NonReservableLeaves,

    #[error("Service error: {0}")]
    ServiceError(#[from] crate::services::ServiceError),

    #[error("generic error: {0}")]
    Generic(String),
}

impl From<OperatorRpcError> for TreeServiceError {
    fn from(error: OperatorRpcError) -> Self {
        TreeServiceError::RpcError(Box::new(error))
    }
}
