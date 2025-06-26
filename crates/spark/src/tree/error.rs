use thiserror::Error;

#[derive(Debug, Error)]
pub enum TreeServiceError {
    #[error("insufficient funds")]
    InsufficientFunds,

    #[error("unselectable amount")]
    UnselectableAmount,

    #[error("transfer service error: {0}")]
    TransferServiceError(#[from] crate::services::TransferServiceError),

    #[error("illegal amount")]
    IllegalAmount,
}
