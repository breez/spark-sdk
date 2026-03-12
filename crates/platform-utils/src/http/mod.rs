//! HTTP client abstraction for cross-platform requests.
//!
//! Uses bitreq on native platforms and reqwest on WASM.

use std::collections::HashMap;

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

// Native: bitreq error conversion
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl From<bitreq::Error> for HttpError {
    fn from(err: bitreq::Error) -> Self {
        let err_str = format!("{err:?}");
        match &err {
            bitreq::Error::IoError(io_err) => {
                // Check if it's a timeout error
                if io_err.kind() == std::io::ErrorKind::TimedOut {
                    Self::Timeout(err_str)
                } else {
                    Self::Connect(err_str)
                }
            }
            bitreq::Error::InvalidUtf8InBody(_) | bitreq::Error::InvalidUtf8InResponse => {
                Self::Decode(err_str)
            }
            // Redirect-related errors
            bitreq::Error::TooManyRedirections
            | bitreq::Error::InfiniteRedirectionLoop
            | bitreq::Error::RedirectLocationMissing => Self::Redirect(err_str),
            // Connection errors
            bitreq::Error::AddressNotFound => Self::Connect(err_str),
            // Request/URL errors
            bitreq::Error::InvalidUrl(_) => Self::Request(err_str),
            // Body errors
            bitreq::Error::BodyOverflow => Self::Body(err_str),
            // Other errors
            bitreq::Error::Other(msg) => Self::Other((*msg).to_string()),
            _ => Self::Other(err_str),
        }
    }
}

// WASM: reqwest error conversion
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
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

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod native;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
mod wasm;

// Re-export platform-specific clients
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use native::BitreqHttpClient;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use wasm::ReqwestHttpClient;

/// Default HTTP client type for the current platform.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub type DefaultHttpClient = BitreqHttpClient;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub type DefaultHttpClient = ReqwestHttpClient;

/// Default request timeout in seconds.
pub const REQUEST_TIMEOUT: u64 = 60;

/// Response from an HTTP request.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

impl HttpResponse {
    /// Returns true if the status code indicates success (2xx).
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
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
/// This trait provides a platform-agnostic interface for HTTP operations.
/// Implementations use bitreq on native platforms and reqwest on WASM.
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
pub fn create_http_client(user_agent: Option<&str>) -> Box<dyn HttpClient> {
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    {
        Box::new(BitreqHttpClient::new(user_agent.map(String::from)))
    }
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    {
        Box::new(ReqwestHttpClient::new(user_agent.map(String::from)))
    }
}
