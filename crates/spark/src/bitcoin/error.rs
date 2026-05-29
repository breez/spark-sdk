use bitcoin::sighash::TaprootError;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum BitcoinError {
    #[error("failed to combine key: {0}")]
    KeyCombinationError(String),
    #[error("taproot error: {0}")]
    Taproot(#[from] TaprootError),
    #[error("invalid transaction: {0}")]
    InvalidTransaction(String),
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
}
