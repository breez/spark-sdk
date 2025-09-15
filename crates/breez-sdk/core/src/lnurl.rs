use bitcoin::hex::DisplayHex;
use lnurl_models::{
    CheckUsernameAvailableResponse, RecoverLnurlPayRequest, RecoverLnurlPayResponse,
    RegisterLnurlPayRequest, RegisterLnurlPayResponse, UnregisterLnurlPayRequest,
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
    SigningError(String),
}

// Public-facing request types that don't expose signature details
#[derive(Debug, Clone)]
pub struct RegisterLightningAddressRequest {
    pub username: String,
    pub description: String,
}

// No signature parameter needed for unregister
#[derive(Debug, Clone)]
pub struct UnregisterLightningAddressRequest {
    pub username: String,
}

#[macros::async_trait]
pub trait LnurlServerClient: Send + Sync {
    async fn check_username_available(&self, username: &str) -> Result<bool, LnurlServerError>;
    async fn recover_lightning_address(
        &self,
        pubkey: &PublicKey,
    ) -> Result<Option<RecoverLnurlPayResponse>, LnurlServerError>;
    async fn register_lightning_address(
        &self,
        pubkey: &PublicKey,
        request: &RegisterLightningAddressRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError>;
    async fn unregister_lightning_address(
        &self,
        pubkey: &PublicKey,
        request: &UnregisterLightningAddressRequest,
    ) -> Result<(), LnurlServerError>;
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ReqwestLnurlServerClientError {
    #[error("Invalid API key")]
    InvalidApiKey,

    #[error("Failed to initialize reqwest client: {0}")]
    Initialization(String),

    #[error("Failed to generate signature: {0}")]
    SigningError(String),
}

impl From<InvalidHeaderValue> for ReqwestLnurlServerClientError {
    fn from(_value: InvalidHeaderValue) -> Self {
        Self::InvalidApiKey
    }
}

pub struct ReqwestLnurlServerClient {
    client: reqwest::Client,
    domain: String,
    wallet: std::sync::Arc<spark_wallet::SparkWallet>,
}

impl ReqwestLnurlServerClient {
    pub fn new(
        domain: String,
        api_key: Option<String>,
        wallet: std::sync::Arc<spark_wallet::SparkWallet>,
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
        Ok(Self {
            client,
            domain,
            wallet,
        })
    }
}

#[macros::async_trait]
impl LnurlServerClient for ReqwestLnurlServerClient {
    async fn check_username_available(&self, username: &str) -> Result<bool, LnurlServerError> {
        let url = format!("https://{}/lnurlpay/available/{}", self.domain, username);
        let result = self.client.get(url).send().await;
        let response = match result {
            Ok(response) => response,
            Err(e) => {
                return Err(LnurlServerError::RequestFailure(e.to_string()));
            }
        };

        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(LnurlServerError::InvalidApiKey),
            success if success.is_success() => {
                let body: CheckUsernameAvailableResponse = response.json().await.map_err(|e| {
                    LnurlServerError::RequestFailure(format!(
                        "failed to deserialize response json: {e}"
                    ))
                })?;
                return Ok(body.available);
            }
            other => {
                return Err(LnurlServerError::Network {
                    statuscode: other.as_u16(),
                    message: response.text().await.ok(),
                });
            }
        }
    }

    async fn recover_lightning_address(
        &self,
        pubkey: &PublicKey,
    ) -> Result<Option<RecoverLnurlPayResponse>, LnurlServerError> {
        // Sign the pubkey itself for recovery
        let signature = self
            .wallet
            .sign_message(&pubkey.to_string())
            .await
            .map_err(|e| LnurlServerError::SigningError(e.to_string()))?
            .serialize_der()
            .to_lower_hex_string();

        let request = RecoverLnurlPayRequest { signature };
        let url = format!("https://{}/lnurlpay/{}/recover", self.domain, pubkey);
        let result = self.client.post(url).json(&request).send().await;
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

    async fn register_lightning_address(
        &self,
        pubkey: &PublicKey,
        request: &RegisterLightningAddressRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError> {
        // Sign the username
        let signature = self
            .wallet
            .sign_message(&request.username)
            .await
            .map_err(|e| LnurlServerError::SigningError(e.to_string()))?
            .serialize_der()
            .to_lower_hex_string();

        let request = RegisterLnurlPayRequest {
            username: request.username.clone(),
            description: request.description.clone(),
            signature,
        };

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

    async fn unregister_lightning_address(
        &self,
        pubkey: &PublicKey,
        request: &UnregisterLightningAddressRequest,
    ) -> Result<(), LnurlServerError> {
        // Sign the username
        let signature = self
            .wallet
            .sign_message(&request.username)
            .await
            .map_err(|e| LnurlServerError::SigningError(e.to_string()))?
            .serialize_der()
            .to_lower_hex_string();

        let request = UnregisterLnurlPayRequest {
            username: request.username.clone(),
            signature,
        };

        let url = format!("https://{}/lnurlpay/{}", self.domain, pubkey);
        let result = self.client.delete(url).json(&request).send().await;
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
