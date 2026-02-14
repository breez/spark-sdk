//! Native HTTP client using bitreq.

use std::collections::HashMap;

use crate::HttpError;

use super::{HttpClient, HttpResponse, REQUEST_TIMEOUT};

/// Default connection pool capacity for the HTTP client.
const DEFAULT_POOL_CAPACITY: usize = 10;

/// HTTP client implementation using bitreq for native platforms.
///
/// This client uses bitreq's connection pool to reuse connections
/// across requests, avoiding repeated TCP handshakes and TLS negotiations.
pub struct BitreqHttpClient {
    client: bitreq::Client,
    user_agent: Option<String>,
}

impl BitreqHttpClient {
    /// Create a new `BitreqHttpClient` with an optional user agent.
    pub fn new(user_agent: Option<String>) -> Self {
        Self {
            client: bitreq::Client::new(DEFAULT_POOL_CAPACITY),
            user_agent,
        }
    }

    /// Create a new `BitreqHttpClient` with a custom connection pool capacity.
    pub fn with_capacity(user_agent: Option<String>, capacity: usize) -> Self {
        Self {
            client: bitreq::Client::new(capacity),
            user_agent,
        }
    }

    fn add_common_headers(&self, req: bitreq::Request) -> bitreq::Request {
        let mut req = req.with_timeout(REQUEST_TIMEOUT);
        if let Some(ua) = &self.user_agent {
            req = req.with_header("User-Agent", ua);
        }
        req
    }
}

impl Default for BitreqHttpClient {
    fn default() -> Self {
        Self::new(None)
    }
}

#[macros::async_trait]
impl HttpClient for BitreqHttpClient {
    async fn get(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making GET request to: {url}");
        let mut req = self.add_common_headers(bitreq::get(&url));

        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.with_header(key, value);
            }
        }

        let response = self.client.send_async(req).await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = response.status_code as u16;
        let body = response.as_str()?.to_string();
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse { status, body })
    }

    async fn post(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making POST request to: {url}");
        let mut req = self.add_common_headers(bitreq::post(&url));

        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.with_header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.with_body(body);
        }

        let response = self.client.send_async(req).await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = response.status_code as u16;
        let body = response.as_str()?.to_string();
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse { status, body })
    }

    async fn delete(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making DELETE request to: {url}");
        let mut req = self.add_common_headers(bitreq::delete(&url));

        if let Some(headers) = headers {
            for (key, value) in &headers {
                req = req.with_header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.with_body(body);
        }

        let response = self.client.send_async(req).await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = response.status_code as u16;
        let body = response.as_str()?.to_string();
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse { status, body })
    }
}
