use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum QuoteError {
    /// The manifest or envelope inputs cannot be hashed into a signable digest.
    #[error("quote not hashable: {0}")]
    NotHashable(String),
    /// The quoted manifest does not describe the payment it was requested for.
    #[error("invalid quoted manifest: {0}")]
    InvalidManifest(String),
    #[error("invalid quote signature: {0}")]
    InvalidSignature(String),
}

impl QuoteError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidManifest(message.into())
    }
}

impl From<spark_primitives::SparkPrimitivesError> for QuoteError {
    fn from(err: spark_primitives::SparkPrimitivesError) -> Self {
        let spark_primitives::SparkPrimitivesError::Spark(message) = err;
        Self::NotHashable(message)
    }
}
