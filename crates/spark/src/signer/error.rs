use thiserror::Error;

#[derive(Error, Debug)]
pub enum SignerError {
    #[error("Invalid hash")]
    InvalidHash,
    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),
}
