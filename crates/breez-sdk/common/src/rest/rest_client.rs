// REST client trait and implementations.

use crate::error::ServiceConnectivityError;
use std::collections::HashMap;

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

// Bitreq implementation (native only)
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub struct BitreqRestClient;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl BitreqRestClient {
    pub fn new() -> Result<Self, ServiceConnectivityError> {
        Ok(BitreqRestClient)
    }
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl Default for BitreqRestClient {
    fn default() -> Self {
        Self::new().expect("Failed to create BitreqRestClient")
    }
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[macros::async_trait]
impl RestClient for BitreqRestClient {
    async fn get_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        tracing::debug!("Making GET request to: {url}");
        let mut req = bitreq::get(&url).with_timeout(30);
        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.with_header(key, value);
            }
        }
        let response = req.send_async().await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = response.status_code as u16;
        let body = response.as_str()?.to_string();
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(RestResponse { status, body })
    }

    async fn post_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        tracing::debug!("Making POST request to: {url}");
        let mut req = bitreq::post(&url).with_timeout(30);
        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.with_header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.with_body(body);
        }
        let response = req.send_async().await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = response.status_code as u16;
        let body = response.as_str()?.to_string();
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(RestResponse { status, body })
    }

    async fn delete_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        tracing::debug!("Making DELETE request to: {url}");
        let mut req = bitreq::delete(&url).with_timeout(30);
        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.with_header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.with_body(body);
        }
        let response = req.send_async().await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = response.status_code as u16;
        let body = response.as_str()?.to_string();
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(RestResponse { status, body })
    }
}

// Reqwest implementation (WASM only)
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub struct ReqwestRestClient {
    client: reqwest::Client,
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
impl ReqwestRestClient {
    pub fn new() -> Result<Self, ServiceConnectivityError> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(Into::<ServiceConnectivityError>::into)?;
        Ok(ReqwestRestClient { client })
    }
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
impl Default for ReqwestRestClient {
    fn default() -> Self {
        Self::new().expect("Failed to create ReqwestRestClient")
    }
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
#[macros::async_trait]
impl RestClient for ReqwestRestClient {
    async fn get_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        tracing::debug!("Making GET request to: {url}");
        let mut req = self.client.get(&url);
        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.header(key, value);
            }
        }
        let response = req.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(RestResponse { status, body })
    }

    async fn post_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        tracing::debug!("Making POST request to: {url}");
        let mut req = self.client.post(&url);
        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.body(body);
        }
        let response = req.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(RestResponse { status, body })
    }

    async fn delete_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        tracing::debug!("Making DELETE request to: {url}");
        let mut req = self.client.delete(&url);
        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.body(body);
        }
        let response = req.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(RestResponse { status, body })
    }
}

/// Default REST client - bitreq on native, reqwest on WASM
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub type DefaultRestClient = BitreqRestClient;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub type DefaultRestClient = ReqwestRestClient;
