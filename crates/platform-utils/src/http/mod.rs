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

pub use client::{ReqwestHttpClient, read_capped_bytes, read_capped_text};

/// Default HTTP client type.
pub type DefaultHttpClient = ReqwestHttpClient;

/// Default request timeout in seconds.
pub const REQUEST_TIMEOUT: u64 = 60;

/// Maximum response body the client will buffer.
///
/// A response is refused once it passes this, so a hostile peer cannot drive an
/// unbounded allocation by streaming quickly enough to stay inside
/// [`REQUEST_TIMEOUT`]. Sized for the largest response the SDK legitimately
/// asks for: esplora's `/tx/{txid}/hex` serves hex, so a max-size mined
/// transaction arrives as roughly 8 MB.
pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

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

/// Lets a shared `Arc<dyn HttpClient>` satisfy generic `C: HttpClient` bounds,
/// so callers can hand the SDK's one pooled client to components that own their
/// client by value.
#[macros::async_trait]
impl HttpClient for Arc<dyn HttpClient> {
    async fn get(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<HttpResponse, HttpError> {
        (**self).get(url, headers).await
    }

    async fn post(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<HttpResponse, HttpError> {
        (**self).post(url, headers, body).await
    }

    async fn delete(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<HttpResponse, HttpError> {
        (**self).delete(url, headers, body).await
    }
}

/// Create a new HTTP client with the given user agent.
///
/// Fails when reqwest cannot assemble a client, most reachably because
/// `user_agent` is not a valid header value.
pub fn create_http_client(user_agent: Option<&str>) -> Result<Arc<dyn HttpClient>, HttpError> {
    Ok(Arc::new(ReqwestHttpClient::new(
        user_agent.map(String::from),
    )?))
}

/// Create a new HTTP client that routes every request through `proxy`.
///
/// Fails when `proxy` does not form a usable proxy URL, rather than falling
/// back to a client that would connect directly.
pub fn create_http_client_with_proxy(
    user_agent: Option<&str>,
    proxy: Option<&crate::proxy::ProxyConfig>,
) -> Result<Arc<dyn HttpClient>, HttpError> {
    Ok(Arc::new(ReqwestHttpClient::with_proxy(
        user_agent.map(String::from),
        proxy,
    )?))
}

/// Decides whether a redirect hop may be followed. Receives the hop target
/// and the original request URL (so the decision can depend on where the
/// request started), and returns the refusal reason otherwise.
pub type RedirectFilter = Arc<dyn Fn(&url::Url, &url::Url) -> Result<(), String> + Send + Sync>;

/// Like [`create_http_client_with_proxy`], but every redirect hop must pass
/// `filter` before it is followed (no effect on WASM, where the platform
/// `fetch` always follows). For requests to hosts chosen by untrusted remote
/// parties, where an unvalidated redirect would bypass the checks done on the
/// original URL.
pub fn create_http_client_with_redirect_filter(
    user_agent: Option<&str>,
    proxy: Option<&crate::proxy::ProxyConfig>,
    filter: RedirectFilter,
) -> Result<Arc<dyn HttpClient>, HttpError> {
    Ok(Arc::new(ReqwestHttpClient::with_proxy_and_redirect_filter(
        user_agent.map(String::from),
        proxy,
        filter,
    )?))
}
