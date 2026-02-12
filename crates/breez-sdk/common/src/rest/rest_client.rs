use crate::error::ServiceConnectivityError;
use reqwest::Client;
use std::{collections::HashMap, time::Duration};
use tracing::{debug, trace};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct RestResponse {
    pub status: u16,
    pub body: String,
}

impl RestResponse {
    pub fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

#[macros::async_trait]
pub trait RestClient: Send + Sync {
    /// Makes a GET request and logs on DEBUG.
    /// ### Arguments
    /// - `url`: the URL on which GET will be called
    /// - `headers`: optional headers that will be set on the request
    async fn get_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError>;

    /// Makes a POST request, and logs on DEBUG.
    /// ### Arguments
    /// - `url`: the URL on which POST will be called
    /// - `headers`: the optional POST headers
    /// - `body`: the optional POST body
    async fn post_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError>;

    /// Makes a DELETE request, and logs on DEBUG.
    /// ### Arguments
    /// - `url`: the URL on which DELETE will be called
    /// - `headers`: the optional DELETE headers
    /// - `body`: the optional DELETE body
    async fn delete_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError>;
}

pub struct ReqwestRestClient {
    client: Client,
}
impl ReqwestRestClient {
    pub fn new() -> Result<Self, ServiceConnectivityError> {
        let client = Client::builder()
            .build()
            .map_err(Into::<ServiceConnectivityError>::into)?;
        Ok(ReqwestRestClient { client })
    }
}

#[macros::async_trait]
impl RestClient for ReqwestRestClient {
    async fn get_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        debug!("Making GET request to: {url}");
        let mut req = self.client.get(url).timeout(REQUEST_TIMEOUT);
        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.header(key, value);
            }
        }
        let response = req.send().await?;
        let status = response.status().into();
        let body = response.text().await?;
        debug!("Received response, status: {status}");
        trace!("raw response body: {body}");

        Ok(RestResponse { status, body })
    }

    async fn post_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        debug!("Making POST request to: {url}");
        let mut req = self.client.post(url).timeout(REQUEST_TIMEOUT);
        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.body(body);
        }
        let response = req.send().await?;
        let status = response.status();
        let body = response.text().await?;
        debug!("Received response, status: {status}");
        trace!("raw response body: {body}");

        Ok(RestResponse {
            status: status.into(),
            body,
        })
    }

    async fn delete_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        debug!("Making DELETE request to: {url}");
        let mut req = self.client.delete(url).timeout(REQUEST_TIMEOUT);
        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.body(body);
        }
        let response = req.send().await?;
        let status = response.status();
        let body = response.text().await?;
        debug!("Received response, status: {status}");
        trace!("raw response body: {body}");

        Ok(RestResponse {
            status: status.into(),
            body,
        })
    }
}

/// A REST client that proxies GET requests through a CORS proxy endpoint.
///
/// In WASM/browser environments, cross-origin requests to third-party domains
/// (e.g. LNURL-auth callbacks) are blocked by CORS policy. This client rewrites
/// GET URLs to go through the Breez LNURL server's `/v1/proxy` endpoint.
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub struct ProxyRestClient {
    inner: ReqwestRestClient,
    proxy_base_url: String,
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
impl ProxyRestClient {
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(domain: String) -> Result<Self, ServiceConnectivityError> {
        let proxy_base_url = if domain.contains("://") {
            format!("{domain}/v1/proxy")
        } else {
            format!("https://{domain}/v1/proxy")
        };
        Ok(Self {
            inner: ReqwestRestClient::new()?,
            proxy_base_url,
        })
    }

    fn proxy_url(&self, url: &str) -> String {
        format!(
            "{}?url={}",
            self.proxy_base_url,
            percent_encode(url)
        )
    }
}

/// Percent-encode a string per RFC 3986.
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
fn percent_encode(input: &str) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push('%');
                write!(result, "{byte:02X}").expect("writing to String cannot fail");
            }
        }
    }
    result
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
#[macros::async_trait]
impl RestClient for ProxyRestClient {
    async fn get_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        debug!("ProxyRestClient: proxying GET {url}");
        self.inner
            .get_request(self.proxy_url(&url), headers)
            .await
    }

    async fn post_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        self.inner.post_request(url, headers, body).await
    }

    async fn delete_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        self.inner.delete_request(url, headers, body).await
    }
}

pub fn parse_json<T>(json: &str) -> Result<T, ServiceConnectivityError>
where
    for<'a> T: serde::de::Deserialize<'a>,
{
    serde_json::from_str::<T>(json).map_err(|e| ServiceConnectivityError::Json(e.to_string()))
}
