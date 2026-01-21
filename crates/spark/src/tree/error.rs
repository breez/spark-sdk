use std::time::Duration;

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

    #[error("store processor has shut down")]
    ProcessorShutdown,

    #[error(
        "too many concurrent reservations (max: {max_concurrent}), timed out after {timeout:?}"
    )]
    ResourceBusy {
        max_concurrent: usize,
        timeout: Duration,
    },

    #[error("generic error: {0}")]
    Generic(String),
}

impl From<OperatorRpcError> for TreeServiceError {
    fn from(error: OperatorRpcError) -> Self {
        TreeServiceError::RpcError(Box::new(error))
    }
}
