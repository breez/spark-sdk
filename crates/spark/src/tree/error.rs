use thiserror::Error;

#[derive(Debug, Error)]
pub enum TreeServiceError {
    #[error("insufficient funds")]
    InsufficientFunds,
}
