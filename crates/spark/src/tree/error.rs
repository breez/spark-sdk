use thiserror::Error;

use crate::operator::rpc::OperatorRpcError;

#[derive(Debug, Error)]
pub enum TreeServiceError {
    #[error("insufficient funds")]
    InsufficientFunds,

    #[error("rpc error: {0}")]
    RpcError(#[from] OperatorRpcError),

    #[error("unselectable amount")]
    UnselectableAmount,

    #[error("transfer service error: {0}")]
    TransferServiceError(#[from] crate::services::ServiceError),

    #[error("illegal amount")]
    IllegalAmount,
}
