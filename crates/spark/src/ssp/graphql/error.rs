use thiserror::Error;

use crate::{session_store::SessionStoreError, signer::SignerError};

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

    /// The provider refused a static deposit whose credit after fees would be
    /// below its dust limit.
    #[error("graphql error: {0}")]
    DepositBelowDustLimit(String),

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
        let message = error_messages.join(", ");
        if errors.iter().any(is_deposit_below_dust_limit) {
            return Self::DepositBelowDustLimit(message);
        }
        Self::GraphQL(message)
    }
}

/// The provider names this case only in its message: the error name is the
/// generic `InvalidInputException` it also returns for a malformed request.
fn is_deposit_below_dust_limit(error: &graphql_client::Error) -> bool {
    const MESSAGE_PREFIX: &str = "Credit amount after fees is below dust limit";
    let error_name = error
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("error_name"))
        .and_then(serde_json::Value::as_str);
    error_name == Some("InvalidInputException") && error.message.starts_with(MESSAGE_PREFIX)
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

impl From<SessionStoreError> for GraphQLError {
    fn from(err: SessionStoreError) -> Self {
        Self::Authentication(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use macros::test_all;

    use super::*;

    fn graphql_error(json: &str) -> graphql_client::Error {
        serde_json::from_str(json).unwrap()
    }

    #[test_all]
    fn test_deposit_below_dust_limit_is_recognized() {
        let error = graphql_error(
            r#"{"message": "Credit amount after fees is below dust limit: 241",
                "path": ["static_deposit_quote"],
                "extensions": {"error_name": "InvalidInputException"}}"#,
        );
        let result = GraphQLError::from_graphql_errors(&[error]);
        assert!(matches!(
            &result,
            GraphQLError::DepositBelowDustLimit(message)
                if message == "Credit amount after fees is below dust limit: 241"
        ));
        assert_eq!(
            result.to_string(),
            "graphql error: Credit amount after fees is below dust limit: 241"
        );
    }

    #[test_all]
    fn test_other_invalid_input_stays_generic() {
        let error = graphql_error(
            r#"{"message": "Transaction not found",
                "extensions": {"error_name": "InvalidInputException"}}"#,
        );
        assert!(matches!(
            GraphQLError::from_graphql_errors(&[error]),
            GraphQLError::GraphQL(_)
        ));
    }

    #[test_all]
    fn test_dust_message_needs_the_invalid_input_error_name() {
        let error =
            graphql_error(r#"{"message": "Credit amount after fees is below dust limit: 241"}"#);
        assert!(matches!(
            GraphQLError::from_graphql_errors(&[error]),
            GraphQLError::GraphQL(_)
        ));
    }
}
