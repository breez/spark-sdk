use std::collections::HashMap;
use std::time::Duration;

use jwt::{Claims, Header, Token};
use tracing::{debug, trace};
use web_time::{SystemTime, UNIX_EPOCH};

use crate::{
    FlashnetClient, FlashnetError,
    models::{ChallengeRequest, ChallengeResponse, VerifyRequest, VerifyResponse},
};

use platform_utils::{ContentType, HttpClient, add_content_type_header};

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
        let access_token = self.get_access_token().await?;
        self.get_request_inner(endpoint, Some(&access_token), query)
            .await
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
        let access_token = self.get_access_token().await?;
        self.post_request_inner(endpoint, Some(&access_token), body)
            .await
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

    async fn auth_challenge(
        &self,
        request: ChallengeRequest,
    ) -> Result<ChallengeResponse, FlashnetError> {
        debug!("Posting auth challenge to flashnet");
        self.post_request_inner::<_, ChallengeResponse>("v1/auth/challenge", None, request)
            .await
    }

    async fn auth_verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FlashnetError> {
        debug!("Posting auth verify to flashnet");
        self.post_request_inner::<_, VerifyResponse>("v1/auth/verify", None, request)
            .await
    }

    async fn get_request_inner<S, D>(
        &self,
        endpoint: &str,
        access_token: Option<&str>,
        query: Option<S>,
    ) -> Result<D, FlashnetError>
    where
        S: serde::Serialize + Clone,
        D: serde::de::DeserializeOwned,
    {
        let query_string = match query {
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
        let url = format!("{}/{}{}", self.config.base_url, endpoint, query_string);

        let mut headers = HashMap::new();
        add_content_type_header(&mut headers, ContentType::Json);
        if let Some(token) = access_token {
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }

        let response = self.http_client.get(url, Some(headers)).await?;

        if !response.is_success() {
            return Err(FlashnetError::Network {
                reason: response.body,
                code: Some(response.status),
            });
        }

        response
            .json::<D>()
            .map_err(|e| FlashnetError::Generic(format!("Failed to parse response JSON: {e}")))
    }

    async fn post_request_inner<S, D>(
        &self,
        endpoint: &str,
        access_token: Option<&str>,
        body: S,
    ) -> Result<D, FlashnetError>
    where
        S: serde::Serialize + Clone,
        D: serde::de::DeserializeOwned,
    {
        let url = format!("{}/{}", self.config.base_url, endpoint);
        let body_json = serde_json::to_string(&body)
            .map_err(|e| FlashnetError::Generic(format!("Failed to serialize body: {e}")))?;

        let mut headers = HashMap::new();
        add_content_type_header(&mut headers, ContentType::Json);
        if let Some(token) = access_token {
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }

        let response = self
            .http_client
            .post(url, Some(headers), Some(body_json))
            .await?;

        if !response.is_success() {
            return Err(FlashnetError::Network {
                reason: response.body,
                code: Some(response.status),
            });
        }

        response
            .json::<D>()
            .map_err(|e| FlashnetError::Generic(format!("Failed to parse response JSON: {e}")))
    }
}
