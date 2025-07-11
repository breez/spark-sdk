use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use std::sync::Mutex;

use crate::ssp::graphql::error::GraphQLError;

/// Auth provider for GraphQL API authentication
pub struct AuthProvider {
    session_token: Mutex<Option<String>>,
    valid_until: Mutex<Option<DateTime<Utc>>>,
}

impl AuthProvider {
    /// Create a new AuthProvider
    pub fn new() -> Self {
        Self {
            session_token: Mutex::new(None),
            valid_until: Mutex::new(None),
        }
    }

    /// Add authentication headers to a request
    pub fn add_auth_headers(&self, headers: &mut HeaderMap) -> Result<(), GraphQLError> {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(token) = self.session_token.lock().unwrap().as_ref() {
            let auth_value = format!("Bearer {token}");
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&auth_value)
                    .map_err(|_| GraphQLError::authentication("Invalid header"))?,
            );
        }

        Ok(())
    }

    /// Check if the provider is authorized with a valid token
    pub fn is_authorized(&self) -> Result<bool, GraphQLError> {
        let token_exists = self.session_token.lock().unwrap().is_some();
        let valid_until = self.valid_until.lock().unwrap();

        // TODO: WASM handling of now()
        Ok(token_exists && valid_until.as_ref().is_some_and(|date| *date > Utc::now()))
    }

    /// Set authentication token and expiry
    pub fn set_auth(
        &self,
        session_token: String,
        valid_until: DateTime<Utc>,
    ) -> Result<(), GraphQLError> {
        *self.session_token.lock().unwrap() = Some(session_token);
        *self.valid_until.lock().unwrap() = Some(valid_until);

        Ok(())
    }

    /// Remove authentication
    pub fn remove_auth(&self) -> Result<(), GraphQLError> {
        *self.session_token.lock().unwrap() = None;
        *self.valid_until.lock().unwrap() = None;

        Ok(())
    }
}
