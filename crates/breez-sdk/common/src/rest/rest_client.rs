use crate::error::ServiceConnectivityError;
use reqwest::Client;
use std::{collections::HashMap, time::Duration};
use tracing::{debug, trace};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RestResponse {
    pub status: u16,
    pub body: String,
}

impl RestResponse {
    pub fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait RestClient: Send + Sync {
    /// Makes a GET request and logs on DEBUG.
    /// ### Arguments
    /// - `url`: the URL on which GET will be called
    /// - `headers`: optional headers that will be set on the request
    async fn get(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError>;

    /// Makes a POST request, and logs on DEBUG.
    /// ### Arguments
    /// - `url`: the URL on which POST will be called
    /// - `headers`: the optional POST headers
    /// - `body`: the optional POST body
    async fn post(
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
    async fn get(
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

    async fn post(
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
}

pub fn parse_json<T>(json: &str) -> Result<T, ServiceConnectivityError>
where
    for<'a> T: serde::de::Deserialize<'a>,
{
    serde_json::from_str::<T>(json).map_err(|e| ServiceConnectivityError::Json(e.to_string()))
}
