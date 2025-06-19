use bitcoin::sighash::TaprootError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BitcoinError {
    #[error("failed to combine key: {0}")]
    KeyCombinationError(String),
    #[error("taproot error: {0}")]
    Taproot(#[from] TaprootError),
}
