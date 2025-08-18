#[derive(Debug, thiserror::Error, Clone)]
pub enum AddressError {
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
    #[error("Invalid payment intent: {0}")]
    InvalidPaymentIntent(String),
    #[error("Invalid signature: {0}")]
    InvalidSignature(String),
    #[error("Other error: {0}")]
    Other(String),
}
