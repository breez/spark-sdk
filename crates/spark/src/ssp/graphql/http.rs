//! HTTP client abstraction for GraphQL and REST requests.

use std::collections::HashMap;

/// Response from an HTTP request
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

impl HttpResponse {
    /// Returns true if the status code indicates success (2xx)
    #[allow(dead_code)]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// HTTP client error type
#[derive(Clone, Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum HttpError {
    #[error("Connect error: {0}")]
    Connect(String),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Other error: {0}")]
    Other(String),
}

/// HTTP client trait for making requests
#[macros::async_trait]
pub trait HttpClient: Send + Sync {
    /// Make a GET request
    #[allow(dead_code)]
    async fn get(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
    ) -> Result<HttpResponse, HttpError>;

    /// Make a POST request with optional body
    async fn post(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
        body: Option<&str>,
    ) -> Result<HttpResponse, HttpError>;

    /// Make a DELETE request with optional body
    #[allow(dead_code)]
    async fn delete(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
        body: Option<&str>,
    ) -> Result<HttpResponse, HttpError>;
}

/// Create a new HTTP client with the given user agent
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

// Bitreq implementation (native only)
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub struct BitreqHttpClient {
    user_agent: Option<String>,
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl BitreqHttpClient {
    pub fn new(user_agent: Option<String>) -> Self {
        Self { user_agent }
    }

    fn add_common_headers(&self, req: bitreq::Request) -> bitreq::Request {
        let mut req = req.with_timeout(30);
        if let Some(ua) = &self.user_agent {
            req = req.with_header("User-Agent", ua);
        }
        req
    }
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl From<bitreq::Error> for HttpError {
    fn from(err: bitreq::Error) -> Self {
        let err_str = format!("{err:?}");
        match err {
            bitreq::Error::IoError(_) => Self::Connect(err_str),
            bitreq::Error::InvalidUtf8InBody(_) => Self::Decode(err_str),
            bitreq::Error::Other(msg) => Self::Other(msg.to_string()),
            _ => Self::Other(err_str),
        }
    }
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[macros::async_trait]
impl HttpClient for BitreqHttpClient {
    async fn get(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making GET request to: {url}");
        let mut req = self.add_common_headers(bitreq::get(url));

        if let Some(headers) = headers {
            for (key, value) in headers {
                req = req.with_header(key, value);
            }
        }

        let response = req.send_async().await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = response.status_code as u16;
        let body = response.as_str()?.to_string();
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse { status, body })
    }

    async fn post(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
        body: Option<&str>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making POST request to: {url}");
        let mut req = self.add_common_headers(bitreq::post(url));

        if let Some(headers) = headers {
            for (key, value) in headers {
                req = req.with_header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.with_body(body.to_string());
        }

        let response = req.send_async().await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = response.status_code as u16;
        let body = response.as_str()?.to_string();
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse { status, body })
    }

    async fn delete(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
        body: Option<&str>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making DELETE request to: {url}");
        let mut req = self.add_common_headers(bitreq::delete(url));

        if let Some(headers) = headers {
            for (key, value) in headers {
                req = req.with_header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.with_body(body.to_string());
        }

        let response = req.send_async().await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = response.status_code as u16;
        let body = response.as_str()?.to_string();
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse { status, body })
    }
}

// Reqwest implementation (WASM only)
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
impl ReqwestHttpClient {
    pub fn new(user_agent: Option<String>) -> Self {
        let mut builder = reqwest::Client::builder();
        if let Some(ua) = user_agent {
            builder = builder.user_agent(ua);
        }
        Self {
            client: builder.build().expect("Failed to create reqwest client"),
        }
    }
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
impl From<reqwest::Error> for HttpError {
    fn from(err: reqwest::Error) -> Self {
        Self::Other(err.to_string())
    }
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
#[macros::async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn get(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making GET request to: {url}");
        let mut req = self.client.get(url);

        if let Some(headers) = headers {
            for (key, value) in headers {
                req = req.header(key, value);
            }
        }

        let response = req.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse { status, body })
    }

    async fn post(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
        body: Option<&str>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making POST request to: {url}");
        let mut req = self.client.post(url);

        if let Some(headers) = headers {
            for (key, value) in headers {
                req = req.header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.body(body.to_string());
        }

        let response = req.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse { status, body })
    }

    async fn delete(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
        body: Option<&str>,
    ) -> Result<HttpResponse, HttpError> {
        tracing::debug!("Making DELETE request to: {url}");
        let mut req = self.client.delete(url);

        if let Some(headers) = headers {
            for (key, value) in headers {
                req = req.header(key, value);
            }
        }
        if let Some(body) = body {
            req = req.body(body.to_string());
        }

        let response = req.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        tracing::debug!("Received response, status: {status}");
        tracing::trace!("raw response body: {body}");

        Ok(HttpResponse { status, body })
    }
}
