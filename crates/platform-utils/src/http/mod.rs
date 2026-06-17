//! HTTP client abstraction for cross-platform requests.
//!
//! Uses reqwest on both native and WASM targets.

use std::collections::HashMap;
use std::sync::Arc;

use thiserror::Error;

/// HTTP client error type with rich variants for different error conditions.
#[derive(Clone, Debug, Error)]
pub enum HttpError {
    #[error("Builder error: {0}")]
    Builder(String),
    #[error("Redirect error: {0}")]
    Redirect(String),
    #[error("Status error: {status} - {body}")]
    Status { status: u16, body: String },
    #[error("Timeout error: {0}")]
    Timeout(String),
    #[error("Request error: {0}")]
    Request(String),
    #[error("Connect error: {0}")]
    Connect(String),
    #[error("Body error: {0}")]
    Body(String),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Json error: {0}")]
    Json(String),
    #[error("Other error: {0}")]
    Other(String),
}

impl HttpError {
    /// Returns the HTTP status code if this error contains one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for HttpError {
    fn from(err: reqwest::Error) -> Self {
        let mut err_str = err.to_string();
        let mut walk: &dyn std::error::Error = &err;
        while let Some(src) = walk.source() {
            err_str.push_str(format!(" : {src}").as_str());
            walk = src;
        }
        if err.is_builder() {
            Self::Builder(err_str)
        } else if err.is_redirect() {
            Self::Redirect(err_str)
        } else if err.is_status() {
            Self::Status {
                status: err.status().unwrap_or_default().into(),
                body: err_str,
            }
        } else if err.is_timeout() {
            Self::Timeout(err_str)
        } else if err.is_request() {
            Self::Request(err_str)
        } else if err.is_body() {
            Self::Body(err_str)
        } else if err.is_decode() {
            Self::Decode(err_str)
        } else {
            Self::Other(err_str)
        }
    }
}

mod client;

pub use client::ReqwestHttpClient;

/// Default HTTP client type.
pub type DefaultHttpClient = ReqwestHttpClient;

/// Default request timeout in seconds.
pub const REQUEST_TIMEOUT: u64 = 60;

/// Response from an HTTP request.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    /// Response headers with lowercased names. A header appearing more than once
    /// collapses to its last value, which is sufficient for the single-valued
    /// headers the SDK reads (e.g. `Retry-After`).
    pub headers: HashMap<String, String>,
}

impl HttpResponse {
    /// Returns true if the status code indicates success (2xx).
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Case-insensitive lookup of a response header by name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Parse the response body as JSON.
    pub fn json<T>(&self) -> Result<T, HttpError>
    where
        for<'a> T: serde::de::Deserialize<'a>,
    {
        serde_json::from_str::<T>(&self.body).map_err(|e| HttpError::Json(e.to_string()))
    }
}

/// HTTP client trait for making requests.
///
/// This trait provides a platform-agnostic interface for HTTP operations
/// implemented on top of reqwest.
#[macros::async_trait]
pub trait HttpClient: Send + Sync {
    /// Makes a GET request.
    async fn get(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<HttpResponse, HttpError>;

    /// Makes a POST request with optional body.
    async fn post(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<HttpResponse, HttpError>;

    /// Makes a DELETE request with optional body.
    async fn delete(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<HttpResponse, HttpError>;
}

/// Create a new HTTP client with the given user agent.
pub fn create_http_client(user_agent: Option<&str>) -> Arc<dyn HttpClient> {
    Arc::new(ReqwestHttpClient::new(user_agent.map(String::from)))
}
