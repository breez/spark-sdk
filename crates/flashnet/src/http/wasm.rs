// WASM HTTP client using reqwest

use crate::FlashnetError;
use std::collections::HashMap;
use tracing::trace;

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub async fn get(
    url: &str,
    headers: Option<HashMap<String, String>>,
) -> Result<HttpResponse, FlashnetError> {
    let client = reqwest::Client::new();
    let mut req = client.get(url);
    if let Some(headers) = headers {
        for (key, value) in &headers {
            req = req.header(key, value);
        }
    }

    let response = req.send().await?;
    let status = response.status().as_u16();
    let body = response.text().await?;

    trace!("HTTP GET {url} -> status: {status}");
    Ok(HttpResponse { status, body })
}

pub async fn post(
    url: &str,
    headers: Option<HashMap<String, String>>,
    body: Option<String>,
) -> Result<HttpResponse, FlashnetError> {
    let client = reqwest::Client::new();
    let mut req = client.post(url);
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

    trace!("HTTP POST {url} -> status: {status}");
    Ok(HttpResponse { status, body })
}
