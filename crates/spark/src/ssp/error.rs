use thiserror::Error;

use crate::{signer::SignerError, ssp::graphql::GraphQLError};

/// Alias for Result with ServiceProviderError as the error type
pub type ServiceProviderResult<T> = std::result::Result<T, ServiceProviderError>;

/// GraphQLError represents all the possible errors that can occur when using the GraphQL client
#[derive(Clone, Error, Debug)]
pub enum ServiceProviderError {
    /// Error that occurs during authentication
    #[error("authentication error: {0}")]
    Authentication(String),

    /// Error that occurs when processing GraphQL responses
    #[error("graphql error: {0}")]
    GraphQL(String),

    /// Error that occurs during network requests
    #[error("network error: {reason} (code: {code:?})")]
    Network { reason: String, code: Option<u16> },

    /// Error that occues when using the signer
    #[error("signer error: {0}")]
    Signer(String),

    /// Error during serialization or deserialization
    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("parse error: {0}")]
    ParseError(String),

    #[error("error: {0}")]
    Generic(String),
}

impl From<GraphQLError> for ServiceProviderError {
    fn from(err: GraphQLError) -> Self {
        match err {
            GraphQLError::Authentication(reason) => Self::Authentication(reason),
            GraphQLError::GraphQL(reason) => Self::GraphQL(reason),
            GraphQLError::Network { reason, code } => Self::Network { reason, code },
            GraphQLError::Signer(reason) => Self::Signer(reason),
            GraphQLError::Serialization(reason) => Self::Serialization(reason),
            GraphQLError::Generic(reason) => Self::Generic(reason),
        }
    }
}

impl From<SignerError> for ServiceProviderError {
    fn from(err: SignerError) -> Self {
        Self::Signer(err.to_string())
    }
}
