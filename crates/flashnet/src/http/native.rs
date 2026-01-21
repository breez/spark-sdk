// Native HTTP client using bitreq

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
    let mut req = bitreq::get(url);
    if let Some(headers) = headers {
        for (key, value) in &headers {
            req = req.with_header(key, value);
        }
    }

    let response = req.send_async().await?;

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let status = response.status_code as u16;
    let body = response
        .as_str()
        .map_err(|e| FlashnetError::Generic(format!("Failed to read response body: {e:?}")))?
        .to_string();

    trace!("HTTP GET {url} -> status: {status}");
    Ok(HttpResponse { status, body })
}

pub async fn post(
    url: &str,
    headers: Option<HashMap<String, String>>,
    body: Option<String>,
) -> Result<HttpResponse, FlashnetError> {
    let mut req = bitreq::post(url);
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
    let body = response
        .as_str()
        .map_err(|e| FlashnetError::Generic(format!("Failed to read response body: {e:?}")))?
        .to_string();

    trace!("HTTP POST {url} -> status: {status}");
    Ok(HttpResponse { status, body })
}
