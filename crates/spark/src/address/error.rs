#[derive(Debug, thiserror::Error)]
pub enum AddressServiceError {
    #[error("Invalid bech32m address: {0}")]
    InvalidBech32mAddress(String),
    #[error("Unknown HRP (human-readable part): {0}")]
    UnknownHrp(String),
    #[error("Failed to encode bech32: {0}")]
    Bech32EncodeError(String),
    #[error("Failed to decode protobuf: {0}")]
    ProtobufDecodeError(String),
    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),
    #[error("Other error: {0}")]
    Other(String),
}
