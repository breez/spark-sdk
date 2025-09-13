use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use tracing::debug;

use crate::{
    error::ServiceConnectivityError,
    rest::{RestClient, RestResponse},
};

#[derive(Debug)]
pub struct MockResponse {
    pub(crate) status_code: u16,
    pub(crate) text: String,
}

impl MockResponse {
    pub fn new(status_code: u16, text: String) -> Self {
        MockResponse { status_code, text }
    }
}

#[derive(Default)]
pub struct MockRestClient {
    responses: Mutex<VecDeque<MockResponse>>,
}

impl MockRestClient {
    pub fn new() -> Self {
        MockRestClient::default()
    }

    pub fn add_response(&self, response: MockResponse) -> &Self {
        debug!("Push response: {response:?}");
        let mut responses = self.responses.lock().unwrap();
        responses.push_back(response);
        self
    }
}

#[macros::async_trait]
impl RestClient for MockRestClient {
    async fn get(
        &self,
        _url: String,
        _headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        let mut responses = self.responses.lock().unwrap();
        let response = responses.pop_front().ok_or_else(|| {
            ServiceConnectivityError::Other(String::from("No response available for GET request"))
        })?;
        debug!("Pop GET response: {response:?}");
        let status = response.status_code;
        let body = response.text;

        Ok(RestResponse { status, body })
    }

    async fn post(
        &self,
        _url: String,
        _headers: Option<HashMap<String, String>>,
        _body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        let mut responses = self.responses.lock().unwrap();
        let response = responses.pop_front().ok_or_else(|| {
            ServiceConnectivityError::Other(String::from("No response available for POST request"))
        })?;
        debug!("Pop POST response: {response:?}");
        let status = response.status_code;
        let body = response.text;

        Ok(RestResponse { status, body })
    }

    async fn delete(
        &self,
        _url: String,
        _headers: Option<HashMap<String, String>>,
        _body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError> {
        let mut responses = self.responses.lock().unwrap();
        let response = responses.pop_front().ok_or_else(|| {
            ServiceConnectivityError::Other(String::from(
                "No response available for DELETE request",
            ))
        })?;
        debug!("Pop DELETE response: {response:?}");
        let status = response.status_code;
        let body = response.text;

        Ok(RestResponse { status, body })
    }
}
