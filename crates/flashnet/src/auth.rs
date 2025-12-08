use std::time::Duration;

use jwt::{Claims, Header, Token};
use reqwest::{
    RequestBuilder,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use tracing::{debug, trace};
use web_time::{SystemTime, UNIX_EPOCH};

use crate::{
    FlashnetClient, FlashnetError,
    models::{ChallengeRequest, ChallengeResponse, VerifyRequest, VerifyResponse},
};

const ACCESS_TOKEN_CACHE_KEY: &str = "access_token";
const HOUR_MS: u32 = 60 * 60 * 1000;

impl FlashnetClient {
    pub(crate) async fn get_request<S, D>(
        &self,
        endpoint: &str,
        query: Option<S>,
    ) -> Result<D, FlashnetError>
    where
        S: serde::Serialize + Clone,
        D: serde::de::DeserializeOwned,
    {
        let headers = self.get_headers().await?;
        self.get_request_inner(endpoint, headers, query).await
    }

    pub(crate) async fn post_request<S, D>(
        &self,
        endpoint: &str,
        body: S,
    ) -> Result<D, FlashnetError>
    where
        S: serde::Serialize + Clone,
        D: serde::de::DeserializeOwned,
    {
        let headers = self.get_headers().await?;
        self.post_request_inner(endpoint, headers, body).await
    }

    async fn authenticate(&self) -> Result<String, FlashnetError> {
        debug!("Authenticating with flashnet");
        let identity_public_key = self.spark_wallet.get_identity_public_key().to_string();
        let challenge_request = ChallengeRequest {
            public_key: identity_public_key.clone(),
        };
        let challenge_response = self.auth_challenge(challenge_request).await?;
        trace!(
            "Received challenge from flashnet, challenge: {}, challenge_string: {}, request_id: {}",
            challenge_response.challenge,
            challenge_response.challenge_string,
            challenge_response.request_id
        );

        let signature = self
            .spark_wallet
            .sign_message(&challenge_response.challenge_string)
            .await?;
        let verify_request = VerifyRequest {
            public_key: identity_public_key,
            signature: hex::encode(signature.serialize_compact()),
        };
        let verify_response = self.auth_verify(verify_request).await?;
        trace!("Received verify from flashnet",);

        let token: Token<Header, Claims, _> =
            Token::parse_unverified(&verify_response.access_token).map_err(|e| {
                FlashnetError::Generic(format!("Failed to parse access token: {e:?}"))
            })?;
        let ttl_ms = match token.claims().registered.expiration {
            Some(exp) => {
                let expires = Duration::from_secs(exp);
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
                    FlashnetError::Generic("Failed to get current time".to_string())
                })?;
                let buffer = Duration::from_secs(30);
                expires
                    .saturating_sub(now)
                    .saturating_sub(buffer)
                    .as_millis()
            }
            None => HOUR_MS.into(),
        };
        self.cache_store
            .set(
                ACCESS_TOKEN_CACHE_KEY,
                &verify_response.access_token,
                ttl_ms,
            )
            .await?;

        Ok(verify_response.access_token)
    }

    async fn get_access_token(&self) -> Result<String, FlashnetError> {
        let access_token =
            if let Some(access_token) = self.cache_store.get(ACCESS_TOKEN_CACHE_KEY).await? {
                trace!("Using cached access token");
                access_token
            } else {
                debug!("No access token in cache, authenticating");
                self.authenticate().await?
            };
        Ok(access_token)
    }

    async fn get_headers(&self) -> Result<HeaderMap, FlashnetError> {
        let access_token = self.get_access_token().await?;
        let mut headers: HeaderMap = HeaderMap::new();

        let auth_value = format!("Bearer {access_token}");
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_value)
                .map_err(|_| FlashnetError::Generic("Invalid header".to_string()))?,
        );

        Ok(headers)
    }

    async fn auth_challenge(
        &self,
        request: ChallengeRequest,
    ) -> Result<ChallengeResponse, FlashnetError> {
        debug!("Posting auth challenge to flashnet");
        let mut headers: HeaderMap = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        self.post_request_inner("v1/auth/challenge", headers, request)
            .await
    }

    async fn auth_verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FlashnetError> {
        debug!("Posting auth verify to flashnet");
        let mut headers: HeaderMap = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        self.post_request_inner("v1/auth/verify", headers, request)
            .await
    }

    async fn get_request_inner<S, D>(
        &self,
        endpoint: &str,
        headers: HeaderMap,
        query: Option<S>,
    ) -> Result<D, FlashnetError>
    where
        S: serde::Serialize + Clone,
        D: serde::de::DeserializeOwned,
    {
        let client = reqwest::Client::new();
        let query = match query {
            Some(q) => {
                let qs_config =
                    serde_qs::Config::new().array_format(serde_qs::ArrayFormat::Unindexed);
                let qs = qs_config.serialize_string(&q).map_err(|e| {
                    FlashnetError::Generic(format!("Failed to serialize query parameters: {e}"))
                })?;
                format!("?{qs}")
            }
            None => String::new(),
        };
        let url = format!("{}/{}{}", self.config.base_url, endpoint, query);
        let builder = client.get(&url).headers(headers);

        self.request_inner(builder).await
    }

    async fn post_request_inner<S, D>(
        &self,
        endpoint: &str,
        headers: HeaderMap,
        body: S,
    ) -> Result<D, FlashnetError>
    where
        S: serde::Serialize + Clone,
        D: serde::de::DeserializeOwned,
    {
        let client = reqwest::Client::new();
        let url = format!("{}/{}", self.config.base_url, endpoint);
        let builder = client.post(&url).headers(headers).json(&body);
        self.request_inner(builder).await
    }

    async fn request_inner<D>(&self, builder: RequestBuilder) -> Result<D, FlashnetError>
    where
        D: serde::de::DeserializeOwned,
    {
        let response = builder.send().await?;
        let status_code = response.status();
        let text = response.text().await?;
        trace!("Response: {text:?}");
        if !status_code.is_success() {
            return Err(FlashnetError::Network {
                reason: text,
                code: Some(status_code.as_u16()),
            });
        }
        serde_json::from_str::<D>(&text)
            .map_err(|e| FlashnetError::Generic(format!("Failed to parse response JSON: {e}")))
    }
}
