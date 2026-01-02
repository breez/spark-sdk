use std::{collections::HashMap, sync::Arc};

use crate::ServiceConnectivityError;

/// REST client trait for making HTTP requests.
///
/// This trait provides a way for users to supply their own HTTP client implementation
/// for use with the SDK. The SDK will use this client for all HTTP operations including
/// LNURL flows and chain service requests.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
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

#[macros::derive_from(platform_utils::HttpResponse)]
#[macros::derive_into(platform_utils::HttpResponse)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RestResponse {
    pub status: u16,
    pub body: String,
}

/// Wrapper that adapts an external `RestClient` to `platform_utils::HttpClient`
pub(crate) struct RestClientWrapper {
    inner: Arc<dyn RestClient>,
}

impl RestClientWrapper {
    pub fn new(inner: Arc<dyn RestClient>) -> Self {
        RestClientWrapper { inner }
    }
}

#[macros::async_trait]
impl platform_utils::HttpClient for RestClientWrapper {
    async fn get(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
        Ok(self.inner.get_request(url, headers).await?.into())
    }

    async fn post(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
        Ok(self.inner.post_request(url, headers, body).await?.into())
    }

    async fn delete(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
        Ok(self.inner.delete_request(url, headers, body).await?.into())
    }
}
