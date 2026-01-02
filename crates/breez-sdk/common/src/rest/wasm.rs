// WASM REST client using reqwest

use crate::error::ServiceConnectivityError;
use std::collections::HashMap;

use super::{REQUEST_TIMEOUT, RestClient, RestResponse};

pub struct ReqwestRestClient {
    client: reqwest::Client,
}

impl ReqwestRestClient {
    pub fn new() -> Result<Self, ServiceConnectivityError> {
        let client = reqwest::Client::builder()
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
        tracing::debug!("Making GET request to: {url}");
        let mut req = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT));
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
        let mut req = self
            .client
            .post(&url)
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT));
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
        let mut req = self
            .client
            .delete(&url)
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT));
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
