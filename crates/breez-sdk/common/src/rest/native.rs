// Native REST client using bitreq

use crate::error::ServiceConnectivityError;
use std::collections::HashMap;

use super::{REQUEST_TIMEOUT, RestClient, RestResponse};

pub struct BitreqRestClient;

impl BitreqRestClient {
    pub fn new() -> Result<Self, ServiceConnectivityError> {
        Ok(BitreqRestClient)
    }
}

#[macros::async_trait]
impl RestClient for BitreqRestClient {
    async fn get_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        tracing::debug!("Making GET request to: {url}");
        let mut req = bitreq::get(&url).with_timeout(REQUEST_TIMEOUT);
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
        let mut req = bitreq::post(&url).with_timeout(REQUEST_TIMEOUT);
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
        let mut req = bitreq::delete(&url).with_timeout(REQUEST_TIMEOUT);
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
