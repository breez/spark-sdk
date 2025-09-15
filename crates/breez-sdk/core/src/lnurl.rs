use lnurl_models::{
    RecoverLnurlPayRequest, RecoverLnurlPayResponse, RegisterLnurlPayRequest,
    RegisterLnurlPayResponse, UnregisterLnurlPayRequest,
};
use reqwest::{
    StatusCode,
    header::{AUTHORIZATION, HeaderMap, InvalidHeaderValue},
};
use spark_wallet::PublicKey;

pub enum LnurlServerError {
    InvalidApiKey,
    Network {
        statuscode: u16,
        message: Option<String>,
    },
    RequestFailure(String),
}

#[macros::async_trait]
pub trait LnurlServerClient: Send + Sync {
    async fn recover_lnurl_pay(
        &self,
        pubkey: &PublicKey,
        request: &RecoverLnurlPayRequest,
    ) -> Result<Option<RecoverLnurlPayResponse>, LnurlServerError>;
    async fn register_lnurl_pay(
        &self,
        pubkey: &PublicKey,
        request: &RegisterLnurlPayRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError>;
    async fn unregister_lnurl_pay(
        &self,
        pubkey: &PublicKey,
        request: &UnregisterLnurlPayRequest,
    ) -> Result<(), LnurlServerError>;
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ReqwestLnurlServerClientError {
    #[error("Invalid API key")]
    InvalidApiKey,

    #[error("Failed to initialize reqwest client: {0}")]
    Initialization(String),
}

impl From<InvalidHeaderValue> for ReqwestLnurlServerClientError {
    fn from(_value: InvalidHeaderValue) -> Self {
        Self::InvalidApiKey
    }
}
pub struct ReqwestLnurlServerClient {
    client: reqwest::Client,
    domain: String,
}

impl ReqwestLnurlServerClient {
    pub fn new(
        domain: String,
        api_key: Option<String>,
    ) -> Result<Self, ReqwestLnurlServerClientError> {
        let mut builder = reqwest::Client::builder().user_agent("breez-sdk-spark");
        if let Some(api_key) = api_key {
            builder = builder.default_headers({
                let mut headers = HeaderMap::new();
                headers.insert(AUTHORIZATION, api_key.parse()?);
                headers
            });
        }
        let client = builder
            .build()
            .map_err(|e| ReqwestLnurlServerClientError::Initialization(e.to_string()))?;
        Ok(Self { client, domain })
    }
}

#[macros::async_trait]
impl LnurlServerClient for ReqwestLnurlServerClient {
    async fn recover_lnurl_pay(
        &self,
        pubkey: &PublicKey,
        request: &RecoverLnurlPayRequest,
    ) -> Result<Option<RecoverLnurlPayResponse>, LnurlServerError> {
        let url = format!("https://{}/lnurlpay/{}/recover", self.domain, pubkey);
        let result = self.client.post(url).json(request).send().await;
        let response = match result {
            Ok(response) => response,
            Err(e) => {
                return Err(LnurlServerError::RequestFailure(e.to_string()));
            }
        };

        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(LnurlServerError::InvalidApiKey),
            StatusCode::NOT_FOUND => return Ok(None),
            success if success.is_success() => {
                let body = response.json().await.map_err(|e| {
                    LnurlServerError::RequestFailure(format!(
                        "failed to deserialize response json: {e}"
                    ))
                })?;
                return Ok(Some(body));
            }
            other => {
                return Err(LnurlServerError::Network {
                    statuscode: other.as_u16(),
                    message: response.text().await.ok(),
                });
            }
        }
    }

    async fn register_lnurl_pay(
        &self,
        pubkey: &PublicKey,
        request: &RegisterLnurlPayRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError> {
        let url = format!("https://{}/lnurlpay/{}", self.domain, pubkey);
        let result = self.client.post(url).json(&request).send().await;
        let response = match result {
            Ok(response) => response,
            Err(e) => {
                return Err(LnurlServerError::RequestFailure(e.to_string()));
            }
        };

        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(LnurlServerError::InvalidApiKey),
            success if success.is_success() => {
                let body = response.json().await.map_err(|e| {
                    LnurlServerError::RequestFailure(format!(
                        "failed to deserialize response json: {e}"
                    ))
                })?;
                return Ok(body);
            }
            other => {
                return Err(LnurlServerError::Network {
                    statuscode: other.as_u16(),
                    message: response.text().await.ok(),
                });
            }
        }
    }

    async fn unregister_lnurl_pay(
        &self,
        pubkey: &PublicKey,
        request: &UnregisterLnurlPayRequest,
    ) -> Result<(), LnurlServerError> {
        let url = format!("https://{}/lnurlpay/{}", self.domain, pubkey);
        let result = self.client.delete(url).json(request).send().await;
        let response = match result {
            Ok(response) => response,
            Err(e) => {
                return Err(LnurlServerError::RequestFailure(e.to_string()));
            }
        };

        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(LnurlServerError::InvalidApiKey),
            StatusCode::NOT_FOUND => return Ok(()),
            success if success.is_success() => return Ok(()),
            other => {
                return Err(LnurlServerError::Network {
                    statuscode: other.as_u16(),
                    message: response.text().await.ok(),
                });
            }
        }
    }
}
