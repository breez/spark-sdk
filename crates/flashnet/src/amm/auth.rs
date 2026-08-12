use std::collections::HashMap;

use base64::Engine;
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, trace};

use super::api::FlashnetClient;
use super::models::{ChallengeRequest, ChallengeResponse, VerifyRequest, VerifyResponse};
use crate::error::FlashnetError;

use platform_utils::{ContentType, add_content_type_header};

const ACCESS_TOKEN_CACHE_KEY: &str = "access_token";
const HOUR_MS: u32 = 60 * 60 * 1000;

/// Shaved off the lifetime so a request cannot go out on a token that expires
/// in flight.
const TOKEN_EXPIRY_BUFFER_SECS: u64 = 30;
/// Bounds on the cached lifetime. `exp` comes from a JWT the client never
/// verifies: without a floor a past `exp` re-authenticates on every request, at
/// one identity-key signature each; without a ceiling a dead token stays cached.
const MIN_TOKEN_TTL_MS: u128 = 60 * 1000;
const MAX_TOKEN_TTL_MS: u128 = 12 * 60 * 60 * 1000;

/// Format of every authentication challenge: this prefix followed by 64 hex
/// characters.
const CHALLENGE_PREFIX: &str = "FLASHNET_AUTH_CHALLENGE_V1:";
const CHALLENGE_NONCE_HEX_LEN: usize = 64;

/// Rejects a challenge whose format the client does not recognise.
///
/// The challenge is signed with the identity key, which applies no domain
/// separation, so a signature over it is valid for any other structure with the
/// same bytes. Pinning the format is what keeps a challenge from doubling as
/// one. A version bump on the server side fails here rather than silently
/// widening what the client will sign.
fn ensure_challenge_format(challenge: &str) -> Result<(), FlashnetError> {
    let Some(nonce) = challenge.strip_prefix(CHALLENGE_PREFIX) else {
        return Err(FlashnetError::InvalidChallenge {
            reason: format!("expected the prefix {CHALLENGE_PREFIX}"),
        });
    };
    if nonce.len() != CHALLENGE_NONCE_HEX_LEN || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(FlashnetError::InvalidChallenge {
            reason: format!("expected {CHALLENGE_NONCE_HEX_LEN} hex characters after the prefix"),
        });
    }
    Ok(())
}

/// Whether the server rejected the token we presented. Narrow on purpose: a 403
/// is an authorisation decision that re-authenticating will not change, and a
/// 5xx says nothing about the token.
fn is_unauthorized(err: &FlashnetError) -> bool {
    matches!(
        err,
        FlashnetError::Network {
            code: Some(401),
            ..
        }
    )
}

/// Bounded token cache lifetime. A missing `exp` falls back to an hour, which
/// is already inside the band.
fn clamp_token_ttl_ms(exp: Option<u64>, now_secs: u64) -> u128 {
    let Some(exp) = exp else {
        return HOUR_MS.into();
    };
    let remaining = exp
        .saturating_sub(now_secs)
        .saturating_sub(TOKEN_EXPIRY_BUFFER_SECS);
    let ttl_ms = u128::from(remaining).saturating_mul(1000);
    let clamped = ttl_ms.clamp(MIN_TOKEN_TTL_MS, MAX_TOKEN_TTL_MS);
    if clamped != ttl_ms {
        tracing::warn!("Flashnet token TTL {ttl_ms}ms out of range, using {clamped}ms");
    }
    clamped
}

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
        let retry_query = query.clone();
        // A 401 means the request was never acted on, so replaying it is safe.
        match self
            .get_request_inner(endpoint, Some(&access_token), query)
            .await
        {
            Err(e) if is_unauthorized(&e) => {
                let access_token = self.reauthenticate(&access_token).await?;
                self.get_request_inner(endpoint, Some(&access_token), retry_query)
                    .await
            }
            result => result,
        }
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
        let retry_body = body.clone();
        // A 401 means the request was never acted on, so replaying it is safe.
        match self
            .post_request_inner(endpoint, Some(&access_token), body)
            .await
        {
            Err(e) if is_unauthorized(&e) => {
                let access_token = self.reauthenticate(&access_token).await?;
                self.post_request_inner(endpoint, Some(&access_token), retry_body)
                    .await
            }
            result => result,
        }
    }

    /// Replaces a token the server rejected, collapsing concurrent callers onto
    /// one authentication. Without the recheck, every request holding the same
    /// dead token would authenticate separately, each one an identity-key
    /// signature over a string the server chose.
    async fn reauthenticate(&self, stale_token: &str) -> Result<String, FlashnetError> {
        let _guard = self.auth_mutex.lock().await;

        if let Some(current) = self
            .cache_store
            .get::<String>(ACCESS_TOKEN_CACHE_KEY)
            .await?
            && current != stale_token
        {
            trace!("Another caller already replaced the rejected token");
            return Ok(current);
        }

        debug!("Access token rejected, re-authenticating");
        self.cache_store.remove(ACCESS_TOKEN_CACHE_KEY).await;
        self.authenticate().await
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

        ensure_challenge_format(&challenge_response.challenge_string)?;
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

        let exp = verify_response
            .access_token
            .split('.')
            .nth(1)
            .and_then(|payload| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(payload)
                    .ok()
            })
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|json| json.get("exp").and_then(serde_json::Value::as_u64));
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| FlashnetError::Generic("Failed to get current time".to_string()))?
            .as_secs();
        let ttl_ms = clamp_token_ttl_ms(exp, now_secs);
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
        // Acquire lock to serialize authentication attempts
        let _guard = self.auth_mutex.lock().await;

        if let Some(access_token) = self.cache_store.get(ACCESS_TOKEN_CACHE_KEY).await? {
            trace!("Using cached access token");
            return Ok(access_token);
        }

        debug!("No access token in cache, authenticating");
        self.authenticate().await
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
                let qs = serde_urlencoded::to_string(&q).map_err(|e| {
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

#[cfg(test)]
mod tests {
    use super::{
        CHALLENGE_PREFIX, FlashnetError, HOUR_MS, MAX_TOKEN_TTL_MS, MIN_TOKEN_TTL_MS,
        TOKEN_EXPIRY_BUFFER_SECS, clamp_token_ttl_ms, ensure_challenge_format, is_unauthorized,
    };

    const NOW: u64 = 1_700_000_000;

    #[test]
    fn missing_exp_falls_back_to_an_hour() {
        assert_eq!(clamp_token_ttl_ms(None, NOW), u128::from(HOUR_MS));
    }

    #[test]
    fn normal_exp_keeps_its_lifetime_less_the_buffer() {
        let exp = NOW + 3600;
        let expected = u128::from(3600 - TOKEN_EXPIRY_BUFFER_SECS) * 1000;
        assert_eq!(clamp_token_ttl_ms(Some(exp), NOW), expected);
    }

    #[test]
    fn past_exp_is_floored_instead_of_forcing_reauth_per_request() {
        assert_eq!(
            clamp_token_ttl_ms(Some(NOW - 10_000), NOW),
            MIN_TOKEN_TTL_MS
        );
        assert_eq!(clamp_token_ttl_ms(Some(0), NOW), MIN_TOKEN_TTL_MS);
    }

    #[test]
    fn exp_inside_the_buffer_is_floored() {
        let exp = NOW + TOKEN_EXPIRY_BUFFER_SECS - 1;
        assert_eq!(clamp_token_ttl_ms(Some(exp), NOW), MIN_TOKEN_TTL_MS);
    }

    #[test]
    fn absurd_future_exp_is_capped() {
        assert_eq!(clamp_token_ttl_ms(Some(u64::MAX), NOW), MAX_TOKEN_TTL_MS);
        let exp = NOW + 365 * 24 * 60 * 60;
        assert_eq!(clamp_token_ttl_ms(Some(exp), NOW), MAX_TOKEN_TTL_MS);
    }

    /// Captured from mainnet. The nonce is the 64 hex characters.
    const LIVE_CHALLENGE: &str = "FLASHNET_AUTH_CHALLENGE_V1:cb78c5950e3ef6c295855ceff7f7ae8626d829f861b1c26df91edd889ed0b05c";

    #[test]
    fn the_live_challenge_format_is_accepted() {
        assert!(ensure_challenge_format(LIVE_CHALLENGE).is_ok());
    }

    #[test]
    fn a_serialized_intent_is_never_signed_as_a_challenge() {
        use crate::amm::models::{ClawbackIntent, ExecuteSwapIntent};
        use spark_wallet::PublicKey;
        use std::str::FromStr;

        let key = PublicKey::from_str(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
        )
        .unwrap();
        let swap = serde_json::to_string(&ExecuteSwapIntent {
            user_public_key: key,
            lp_identity_public_key: key,
            asset_in_spark_transfer_id: "transfer-1".to_string(),
            asset_in_address: "aa".to_string(),
            asset_out_address: "bb".to_string(),
            amount_in: 1_000,
            min_amount_out: 950,
            max_slippage_bps: 10,
            nonce: "00".repeat(16),
            total_integrator_fee_rate_bps: 5,
        })
        .unwrap();
        let clawback = serde_json::to_string(&ClawbackIntent {
            sender_public_key: key,
            spark_transfer_id: "transfer-1".to_string(),
            lp_identity_public_key: key,
            nonce: "00".repeat(16),
        })
        .unwrap();

        for intent in [&swap, &clawback] {
            assert!(ensure_challenge_format(intent).is_err());
            // Also when dressed up to look like a challenge.
            assert!(ensure_challenge_format(&format!("{CHALLENGE_PREFIX}{intent}")).is_err());
        }
    }

    #[test]
    fn a_challenge_without_the_expected_prefix_is_rejected() {
        let nonce = &LIVE_CHALLENGE[CHALLENGE_PREFIX.len()..];
        for challenge in [
            String::new(),
            nonce.to_string(),
            format!("FLASHNET_AUTH_CHALLENGE_V2:{nonce}"),
            format!("flashnet_auth_challenge_v1:{nonce}"),
            format!(" {CHALLENGE_PREFIX}{nonce}"),
        ] {
            assert!(
                ensure_challenge_format(&challenge).is_err(),
                "accepted {challenge:?}"
            );
        }
    }

    #[test]
    fn a_nonce_of_the_wrong_shape_is_rejected() {
        for nonce in ["", &"a".repeat(63), &"a".repeat(65), &"z".repeat(64)] {
            let challenge = format!("{CHALLENGE_PREFIX}{nonce}");
            assert!(
                ensure_challenge_format(&challenge).is_err(),
                "accepted nonce {nonce:?}"
            );
        }
    }

    #[test]
    fn only_a_401_triggers_re_authentication() {
        let unauthorized = FlashnetError::Network {
            reason: "nope".to_string(),
            code: Some(401),
        };
        assert!(is_unauthorized(&unauthorized));

        for code in [None, Some(400), Some(403), Some(429), Some(500)] {
            let err = FlashnetError::Network {
                reason: "nope".to_string(),
                code,
            };
            assert!(!is_unauthorized(&err), "treated {code:?} as unauthorized");
        }
        assert!(!is_unauthorized(&FlashnetError::Generic("x".to_string())));
    }

    #[test]
    fn the_fallback_sits_inside_the_band() {
        assert!(u128::from(HOUR_MS) >= MIN_TOKEN_TTL_MS);
        assert!(u128::from(HOUR_MS) <= MAX_TOKEN_TTL_MS);
    }
}
