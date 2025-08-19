use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum SignerError {
    #[error("frost error: {0}")]
    FrostError(String),
    #[error("failed to derive identifier")]
    IdentifierError,
    #[error("invalid hash")]
    InvalidHash,
    #[error("key derivation error: {0}")]
    KeyDerivationError(String),
    #[error("failed to create nonce: {0}")]
    NonceCreationError(String),
    #[error("secret sharing error: {0}")]
    SecretSharingError(String),
    #[error("serialization error: {0}")]
    SerializationError(String),
    #[error("unknown key")]
    UnknownKey,
    #[error("unknown nonce commitment")]
    UnknownNonceCommitment,

    #[error("generic error: {0}")]
    Generic(String),
}
