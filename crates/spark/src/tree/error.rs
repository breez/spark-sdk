use thiserror::Error;

#[derive(Debug, Error)]
pub enum TreeServiceError {
    #[error("insufficient funds")]
    InsufficientFunds,

    #[error("unselectable amount")]
    UnselectableAmount,

    #[error("illegal amount")]
    IllegalAmount,
}
