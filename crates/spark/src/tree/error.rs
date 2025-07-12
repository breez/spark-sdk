use thiserror::Error;

use crate::{operator::rpc::OperatorRpcError, signer::SignerError};

#[derive(Debug, Error)]
pub enum TreeServiceError {
    #[error("insufficient funds")]
    InsufficientFunds,

    #[error("rpc error: {0}")]
    RpcError(#[from] OperatorRpcError),

    #[error("unselectable amount")]
    UnselectableAmount,

    #[error("invalid amount")]
    InvalidAmount,

    #[error("Signer error: {0}")]
    SignerError(#[from] SignerError),

    #[error("generic error: {0}")]
    Generic(String),
}
