use bitcoin::hex::DisplayHex;
use lnurl_models::{
    CheckUsernameAvailableResponse, ListMetadataResponse, RecoverLnurlPayRequest,
    RecoverLnurlPayResponse, RegisterLnurlPayRequest, RegisterLnurlPayResponse,
    UnregisterLnurlPayRequest,
};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use platform_utils::{ContentType, HttpClient, add_content_type_header};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

#[derive(Debug)]
pub enum LnurlServerError {
    InvalidApiKey,
    Network {
        statuscode: u16,
        message: Option<String>,
    },
    RequestFailure(String),
    SigningError(String),
}

impl std::fmt::Display for LnurlServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LnurlServerError::InvalidApiKey => write!(f, "Invalid API key"),
            LnurlServerError::Network {
                statuscode,
                message,
            } => {
                write!(f, "Network error (status {statuscode}): {message:?}")
            }
            LnurlServerError::RequestFailure(msg) => write!(f, "Request failure: {msg}"),
            LnurlServerError::SigningError(msg) => write!(f, "Signing error: {msg}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegisterLightningAddressRequest {
    pub username: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct UnregisterLightningAddressRequest {
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct ListMetadataRequest {
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub updated_after: Option<i64>,
}

#[macros::async_trait]
pub trait LnurlServerClient: Send + Sync {
    fn domain(&self) -> &str;
    async fn check_username_available(&self, username: &str) -> Result<bool, LnurlServerError>;
    async fn recover_lightning_address(
        &self,
    ) -> Result<Option<RecoverLnurlPayResponse>, LnurlServerError>;
    async fn register_lightning_address(
        &self,
        request: &RegisterLightningAddressRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError>;
    async fn unregister_lightning_address(
        &self,
        request: &UnregisterLightningAddressRequest,
    ) -> Result<(), LnurlServerError>;
    async fn list_metadata(
        &self,
        request: &ListMetadataRequest,
    ) -> Result<ListMetadataResponse, LnurlServerError>;
}

/// Default `LnurlServerClient` implementation using `HttpClient` abstraction.
pub struct DefaultLnurlServerClient {
    http_client: Arc<dyn HttpClient>,
    domain: String,
    api_key: Option<String>,
    wallet: Arc<spark_wallet::SparkWallet>,
}

impl DefaultLnurlServerClient {
    pub fn new(
        http_client: Arc<dyn HttpClient>,
        domain: String,
        api_key: Option<String>,
        wallet: Arc<spark_wallet::SparkWallet>,
    ) -> Self {
        Self {
            http_client,
            domain,
            api_key,
            wallet,
        }
    }

    /// Construct the base URL for the lnurl server.
    fn base_url(&self) -> String {
        if self.domain.contains("://") {
            self.domain.clone()
        } else {
            format!("https://{}", self.domain)
        }
    }

    /// Get common headers for all requests (User-Agent and Authorization).
    fn get_common_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("User-Agent".to_string(), "breez-sdk-spark".to_string());
        if let Some(api_key) = &self.api_key {
            headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));
        }
        headers
    }

    /// Get headers for POST/DELETE requests (includes Content-Type).
    fn get_post_headers(&self) -> HashMap<String, String> {
        let mut headers = self.get_common_headers();
        add_content_type_header(&mut headers, ContentType::Json);
        headers
    }

    async fn sign_message(&self, message: &str) -> Result<(String, u64), LnurlServerError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| LnurlServerError::SigningError("invalid systemtime".to_string()))?
            .as_secs();
        let signature = self
            .wallet
            .sign_message(&format!("{message}-{timestamp}"))
            .await
            .map_err(|e| LnurlServerError::SigningError(e.to_string()))?;
        Ok((signature.serialize_der().to_lower_hex_string(), timestamp))
    }

    /// Handle response status and parse JSON
    fn handle_response<T: serde::de::DeserializeOwned>(
        status: u16,
        body: &str,
    ) -> Result<T, LnurlServerError> {
        match status {
            401 => Err(LnurlServerError::InvalidApiKey),
            s if (200..300).contains(&s) => serde_json::from_str(body).map_err(|e| {
                LnurlServerError::RequestFailure(format!(
                    "failed to deserialize response json: {e}"
                ))
            }),
            other => Err(LnurlServerError::Network {
                statuscode: other,
                message: Some(body.to_string()),
            }),
        }
    }
}

#[macros::async_trait]
impl LnurlServerClient for DefaultLnurlServerClient {
    fn domain(&self) -> &str {
        &self.domain
    }

    async fn check_username_available(&self, username: &str) -> Result<bool, LnurlServerError> {
        let url = format!("{}/lnurlpay/available/{}", self.base_url(), username);
        let response = self
            .http_client
            .get(url, Some(self.get_common_headers()))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let result: CheckUsernameAvailableResponse =
            Self::handle_response(response.status, &response.body)?;
        Ok(result.available)
    }

    async fn recover_lightning_address(
        &self,
    ) -> Result<Option<RecoverLnurlPayResponse>, LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        let (signature, timestamp) = self.sign_message(&pubkey.to_string()).await?;

        let request = RecoverLnurlPayRequest {
            signature,
            timestamp: Some(timestamp),
        };
        let url = format!("{}/lnurlpay/{}/recover", self.base_url(), pubkey);
        let body = serde_json::to_string(&request)
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let response = self
            .http_client
            .post(url, Some(self.get_post_headers()), Some(body))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        match response.status {
            401 => Err(LnurlServerError::InvalidApiKey),
            404 => Ok(None),
            s if (200..300).contains(&s) => {
                let result = serde_json::from_str(&response.body).map_err(|e| {
                    LnurlServerError::RequestFailure(format!(
                        "failed to deserialize response json: {e}"
                    ))
                })?;
                Ok(Some(result))
            }
            other => Err(LnurlServerError::Network {
                statuscode: other,
                message: Some(response.body),
            }),
        }
    }

    async fn register_lightning_address(
        &self,
        request: &RegisterLightningAddressRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        let (signature, timestamp) = self.sign_message(&request.username).await?;

        let api_request = RegisterLnurlPayRequest {
            username: request.username.clone(),
            description: request.description.clone(),
            signature,
            timestamp: Some(timestamp),
        };

        let url = format!("{}/lnurlpay/{}", self.base_url(), pubkey);
        let body = serde_json::to_string(&api_request)
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let response = self
            .http_client
            .post(url, Some(self.get_post_headers()), Some(body))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        Self::handle_response(response.status, &response.body)
    }

    async fn unregister_lightning_address(
        &self,
        request: &UnregisterLightningAddressRequest,
    ) -> Result<(), LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        let (signature, timestamp) = self.sign_message(&request.username).await?;

        let api_request = UnregisterLnurlPayRequest {
            username: request.username.clone(),
            signature,
            timestamp: Some(timestamp),
        };

        let url = format!("{}/lnurlpay/{}", self.base_url(), pubkey);
        let body = serde_json::to_string(&api_request)
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let response = self
            .http_client
            .delete(url, Some(self.get_post_headers()), Some(body))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        match response.status {
            401 => Err(LnurlServerError::InvalidApiKey),
            404 => Ok(()),
            s if (200..300).contains(&s) => Ok(()),
            other => Err(LnurlServerError::Network {
                statuscode: other,
                message: Some(response.body),
            }),
        }
    }

    async fn list_metadata(
        &self,
        request: &ListMetadataRequest,
    ) -> Result<ListMetadataResponse, LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        let (signature, timestamp) = self.sign_message(&pubkey.to_string()).await?;

        let mut url = format!(
            "{}/lnurlpay/{pubkey}/metadata?signature={signature}&timestamp={timestamp}",
            self.base_url(),
        );
        if let Some(offset) = request.offset {
            let _ = write!(url, "&offset={offset}");
        }
        if let Some(limit) = request.limit {
            let _ = write!(url, "&limit={limit}");
        }
        if let Some(updated_after) = request.updated_after {
            let _ = write!(url, "&updated_after={updated_after}");
        }

        let response = self
            .http_client
            .get(url, Some(self.get_common_headers()))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        Self::handle_response(response.status, &response.body)
    }
}
