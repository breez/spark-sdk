use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use tokio::sync::Mutex;
use web_time::SystemTime;

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
    pub async fn add_auth_headers(&self, headers: &mut HeaderMap) -> Result<(), GraphQLError> {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(token) = self.session_token.lock().await.as_ref() {
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
    pub async fn is_authorized(&self) -> Result<bool, GraphQLError> {
        let token_exists = self.session_token.lock().await.is_some();
        let valid_until = self.valid_until.lock().await;

        if !token_exists {
            return Ok(false);
        }

        if let Some(date) = valid_until.as_ref() {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_err(|e| GraphQLError::generic(format!("Failed to get system time: {}", e)))?;

            let current_time =
                chrono::DateTime::from_timestamp(now.as_secs() as i64, now.subsec_nanos())
                    .ok_or_else(|| {
                        GraphQLError::generic("Failed to convert system time to DateTime")
                    })?;

            Ok(date.naive_utc() > current_time.naive_utc())
        } else {
            Ok(false)
        }
    }

    /// Set authentication token and expiry
    pub async fn set_auth(
        &self,
        session_token: String,
        valid_until: DateTime<Utc>,
    ) -> Result<(), GraphQLError> {
        *self.session_token.lock().await = Some(session_token);
        *self.valid_until.lock().await = Some(valid_until);

        Ok(())
    }

    /// Remove authentication
    pub async fn remove_auth(&self) -> Result<(), GraphQLError> {
        *self.session_token.lock().await = None;
        *self.valid_until.lock().await = None;

        Ok(())
    }
}
