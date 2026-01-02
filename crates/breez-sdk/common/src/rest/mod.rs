// REST client trait and implementations.
// Uses bitreq on native, reqwest on WASM.

use crate::error::ServiceConnectivityError;
use std::collections::HashMap;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod native;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
mod wasm;

// Re-export platform-specific clients
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use native::BitreqRestClient;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use wasm::ReqwestRestClient;

/// Default REST client - bitreq on native, reqwest on WASM
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub type DefaultRestClient = BitreqRestClient;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub type DefaultRestClient = ReqwestRestClient;

pub(crate) const REQUEST_TIMEOUT: u64 = 30;

/// Response from a REST request
pub struct RestResponse {
    pub status: u16,
    pub body: String,
}

impl RestResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[macros::async_trait]
pub trait RestClient: Send + Sync {
    /// Makes a GET request and logs on DEBUG.
    async fn get_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError>;

    /// Makes a POST request, and logs on DEBUG.
    async fn post_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError>;

    /// Makes a DELETE request, and logs on DEBUG.
    async fn delete_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError>;
}

pub fn parse_json<T>(json: &str) -> Result<T, ServiceConnectivityError>
where
    for<'a> T: serde::de::Deserialize<'a>,
{
    serde_json::from_str::<T>(json).map_err(|e| ServiceConnectivityError::Json(e.to_string()))
}
