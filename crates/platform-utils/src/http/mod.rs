//! HTTP client abstraction for cross-platform requests.
//!
//! Uses bitreq on native platforms and reqwest on WASM.

use std::collections::HashMap;

use crate::HttpError;

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
pub const REQUEST_TIMEOUT: u64 = 30;

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
