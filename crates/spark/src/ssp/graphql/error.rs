use thiserror::Error;

use crate::{session_manager::SessionManagerError, signer::SignerError};

/// Alias for Result with GraphQLError as the error type
pub type GraphQLResult<T> = std::result::Result<T, GraphQLError>;

/// GraphQLError represents all the possible errors that can occur when using the GraphQL client
#[derive(Clone, Error, Debug)]
pub(crate) enum GraphQLError {
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
}

impl GraphQLError {
    /// Creates a new serialization error
    pub fn serialization<S: Into<String>>(reason: S) -> Self {
        Self::Serialization(reason.into())
    }

    /// Creates a new GraphQL error from GraphQL error objects
    pub fn from_graphql_errors(errors: &[graphql_client::Error]) -> Self {
        let error_messages: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
        Self::GraphQL(error_messages.join(", "))
    }
}

impl From<platform_utils::HttpError> for GraphQLError {
    fn from(err: platform_utils::HttpError) -> Self {
        Self::Network {
            code: err.status(),
            reason: err.to_string(),
        }
    }
}

impl From<SignerError> for GraphQLError {
    fn from(err: SignerError) -> Self {
        Self::Signer(err.to_string())
    }
}

impl From<SessionManagerError> for GraphQLError {
    fn from(err: SessionManagerError) -> Self {
        Self::Authentication(err.to_string())
    }
}
