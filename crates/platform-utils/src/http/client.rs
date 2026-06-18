//! HTTP client using reqwest for both native and WASM targets.

use std::collections::HashMap;
use std::time::Duration;

use super::{HttpClient, HttpError, HttpResponse, REQUEST_TIMEOUT};

/// Collects response headers into a map with lowercased names, skipping any
/// header whose value is not valid UTF-8.
fn collect_headers(headers: &reqwest::header::HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

/// HTTP client implementation backed by reqwest.
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl ReqwestHttpClient {
    /// Create a new `ReqwestHttpClient` with an optional user agent.
    ///
    /// Native targets layer HTTP/2 and TCP keepalives on top of reqwest's
    /// defaults (uncapped idle pool, 90s idle timeout, `TCP_NODELAY`) so a
    /// long-lived shared client survives intermediaries that reap idle HTTP/2
    /// flows, and apply the caller's user agent.
    ///
    /// On WASM both are skipped. The browser owns connection management, and a
    /// script-set `User-Agent` is not CORS-safelisted, so it turns an otherwise
    /// simple request into a preflighted one: Chrome silently drops the header
    /// (crbug.com/571722) while Firefox/Safari send it and then fail the
    /// preflight against an endpoint that doesn't allow it. The browser sends
    /// its own `User-Agent` regardless, so omitting it loses nothing.
    // `user_agent` is intentionally unused on WASM (see above); the signature
    // stays uniform across targets.
    #[cfg_attr(
        all(target_family = "wasm", target_os = "unknown"),
        expect(clippy::needless_pass_by_value, unused_variables)
    )]
    pub fn new(user_agent: Option<String>) -> Self {
        let builder = reqwest::Client::builder();
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        let builder = {
            let mut builder = builder;
            if let Some(ua) = user_agent {
                builder = builder.user_agent(ua);
            }
            builder
                .tcp_keepalive(Some(Duration::from_mins(1)))
                .http2_keep_alive_interval(Duration::from_secs(30))
                .http2_keep_alive_timeout(Duration::from_secs(10))
                .http2_keep_alive_while_idle(true)
        };
        let client = match builder.build() {
            Ok(client) => client,
            Err(e) => {
                tracing::error!("Failed to create reqwest client: {e}");
                panic!("Failed to create reqwest client: {e}");
            }
        };
        Self { client }
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new(None)
    }
}

#[macros::async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn get(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making GET request to: {url}");
        let mut req = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(REQUEST_TIMEOUT));

        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.header(key, value);
            }
        }

        let response = req.send().await?;
        let status = response.status().as_u16();
        let headers = collect_headers(response.headers());
        let body = response.text().await?;
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse {
            status,
            body,
            headers,
        })
    }

    async fn post(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making POST request to: {url}");
        let mut req = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(REQUEST_TIMEOUT));

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
        let headers = collect_headers(response.headers());
        let body = response.text().await?;
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse {
            status,
            body,
            headers,
        })
    }

    async fn delete(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making DELETE request to: {url}");
        let mut req = self
            .client
            .delete(&url)
            .timeout(Duration::from_secs(REQUEST_TIMEOUT));

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
        let headers = collect_headers(response.headers());
        let body = response.text().await?;
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse {
            status,
            body,
            headers,
        })
    }
}
