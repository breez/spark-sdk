use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use bitcoin::hashes::{Hash, sha256};
use platform_utils::{ContentType, HttpClient, add_content_type_header};
use spark_wallet::{SparkAddress, SparkWallet, TokenRecipient, TransferId};
use tokio::sync::OnceCell;
use tracing::debug;

use super::models::{
    EstimateRequest, EstimateResponse, LimitsResponse, QuoteRequest, QuoteResponse,
    RouteWithLimits, StatusResponse, SubmitRequest, SubmitResponse,
};
use crate::cache::CacheStore;
use crate::config::OrchestraConfig;
use crate::error::FlashnetError;
use crate::models::AssetTransfer;

/// One hour. Orchestra routes are effectively static between deployments, and
/// the published bounds move slowly enough that a stale ceiling is harmless for
/// UI validation.
const ROUTES_TTL_MS: u128 = 60 * 60 * 1000;

/// Resolves the Orchestra config (base URL + API key) on demand.
///
/// The provider is built without a network call: config is resolved lazily on
/// the first Orchestra request and cached on success. A failed resolution is
/// not cached, so a transient outage self-heals on the next cross-chain action.
#[macros::async_trait]
pub trait OrchestraConfigResolver: Send + Sync {
    async fn resolve(&self) -> Result<OrchestraConfig, FlashnetError>;
}

pub struct OrchestraClient {
    config_resolver: Arc<dyn OrchestraConfigResolver>,
    config: OnceCell<OrchestraConfig>,
    http_client: Arc<dyn HttpClient>,
    cache_store: CacheStore,
    spark_wallet: Arc<SparkWallet>,
}

impl OrchestraClient {
    pub fn new(
        config_resolver: Arc<dyn OrchestraConfigResolver>,
        spark_wallet: Arc<SparkWallet>,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            config_resolver,
            config: OnceCell::new(),
            http_client,
            cache_store: CacheStore::default(),
            spark_wallet,
        }
    }

    /// Resolves the config on first use and caches it. Only success is cached:
    /// a failure is retried on the next call.
    async fn config(&self) -> Result<&OrchestraConfig, FlashnetError> {
        self.config
            .get_or_try_init(|| self.config_resolver.resolve())
            .await
    }

    /// Return the routes where `chain` is involved, each with the amount bounds
    /// the provider publishes for it.
    ///
    /// When `is_send` is `true`, returns routes with `source_chain == chain`
    /// (funding from `chain` to another chain, e.g. `spark` for a wallet send or
    /// `lightning` for a Cash App onramp). When `false`, returns routes with
    /// `destination_chain == chain` (receiving into `chain`).
    ///
    /// Reads `/limits` rather than `/routes`: it returns the same route set
    /// under the same filter with the bounds attached, and filtering server-side
    /// keeps the payload a fraction of the unfiltered route list. Responses are
    /// cached per `(chain, direction)` for [`ROUTES_TTL_MS`] so repeated
    /// parser/UI calls are synchronous.
    pub async fn filter_routes(
        &self,
        chain: &str,
        is_send: bool,
    ) -> Result<Vec<RouteWithLimits>, FlashnetError> {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Query<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            source_chain: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            destination_chain: Option<&'a str>,
        }

        let direction = if is_send { "source" } else { "destination" };
        let cache_key = format!("orchestra_limits:{direction}:{chain}");
        if let Some(cached) = self.cache_store.get::<LimitsResponse>(&cache_key).await? {
            return Ok(cached.routes);
        }
        debug!("Orchestra: GET /v1/orchestration/limits?{direction}Chain={chain} (cache miss)");

        let query = if is_send {
            Query {
                source_chain: Some(chain),
                destination_chain: None,
            }
        } else {
            Query {
                source_chain: None,
                destination_chain: Some(chain),
            }
        };

        let response: LimitsResponse = self
            .get("v1/orchestration/limits", Some(query), false)
            .await?;
        self.cache_store
            .set(&cache_key, &response, ROUTES_TTL_MS)
            .await?;
        Ok(response.routes)
    }

    pub async fn estimate(
        &self,
        request: EstimateRequest,
    ) -> Result<EstimateResponse, FlashnetError> {
        debug!("Orchestra: GET /v1/orchestration/estimate");
        self.get("v1/orchestration/estimate", Some(request), true)
            .await
    }

    /// Create a quote. Requires auth + idempotency key.
    pub async fn quote(&self, request: QuoteRequest) -> Result<QuoteResponse, FlashnetError> {
        debug!(
            "Orchestra: POST /v1/orchestration/quote (source={}/{} dest={}/{})",
            request.source_chain,
            request.source_asset,
            request.destination_chain,
            request.destination_asset
        );
        let idem = uuid::Uuid::new_v4().to_string();
        self.post("v1/orchestration/quote", &request, true, Some(idem))
            .await
    }

    /// Submit a quote.
    ///
    /// `idempotency_key` is sent as `X-Idempotency-Key`. Reusing a key
    /// replays the cached response, so a caller that expects Orchestra state
    /// to advance between calls must vary the key. The caller picks the
    /// policy (deterministic to collide retries, or fresh per call).
    pub async fn submit(
        &self,
        request: SubmitRequest,
        idempotency_key: String,
    ) -> Result<SubmitResponse, FlashnetError> {
        debug!(
            "Orchestra: POST /v1/orchestration/submit quoteId={} idem={}",
            request.quote_id, idempotency_key
        );
        self.post(
            "v1/orchestration/submit",
            &request,
            true,
            Some(idempotency_key),
        )
        .await
    }

    /// Send `amount_in` sats (or tokens) to the Orchestra-provided `deposit_address`
    /// via the Spark wallet. Returns the full [`AssetTransfer`].
    ///
    /// `transfer_id` is threaded into the underlying Spark BTC transfer for
    /// protocol-level idempotency on retry. Ignored on the token branch:
    /// `spark_wallet::transfer_tokens` does not accept an idempotency token,
    /// matching the SDK's contract that token-source sends carry no retry
    /// guarantee.
    pub async fn transfer_to_deposit(
        &self,
        deposit_address: &str,
        amount_in: u128,
        token_identifier: Option<&str>,
        transfer_id: Option<TransferId>,
    ) -> Result<AssetTransfer, FlashnetError> {
        let receiver_address = SparkAddress::from_str(deposit_address).map_err(|e| {
            FlashnetError::Generic(format!(
                "Failed to parse Orchestra deposit address '{deposit_address}': {e}"
            ))
        })?;

        match token_identifier {
            None => {
                // BTC sats — plain Spark transfer.
                let amount_sats = u64::try_from(amount_in)
                    .map_err(|e| FlashnetError::Generic(format!("amount_in exceeds u64: {e}")))?;
                let transfer = self
                    .spark_wallet
                    .transfer(amount_sats, &receiver_address, transfer_id)
                    .await?;
                Ok(AssetTransfer::Spark(transfer))
            }
            Some(token_identifier) => {
                // USDB (or other Spark token) — token transfer.
                // `token_identifier` is already a bech32m-encoded token id (e.g. btkn1x...).
                let token_tx = self
                    .spark_wallet
                    .transfer_tokens(
                        vec![TokenRecipient::Address {
                            token_id: token_identifier.to_string(),
                            amount: amount_in,
                            receiver_address,
                        }],
                        None,
                        None,
                    )
                    .await?;
                Ok(AssetTransfer::Token(token_tx))
            }
        }
    }

    /// Look up an order by its id.
    ///
    /// `read_token` is the opaque token returned by `/submit` — required by
    /// Orchestra to authorise status reads for partner API keys.
    pub async fn status_by_id(
        &self,
        order_id: &str,
        read_token: Option<&str>,
    ) -> Result<StatusResponse, FlashnetError> {
        #[derive(serde::Serialize)]
        struct Query<'a> {
            id: &'a str,
        }
        self.get_with_read_token(
            "v1/orchestration/status",
            Some(Query { id: order_id }),
            true,
            read_token,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // internals
    // -----------------------------------------------------------------------

    async fn get<S, D>(
        &self,
        endpoint: &str,
        query: Option<S>,
        authed: bool,
    ) -> Result<D, FlashnetError>
    where
        S: serde::Serialize,
        D: serde::de::DeserializeOwned,
    {
        self.get_with_read_token(endpoint, query, authed, None)
            .await
    }

    async fn get_with_read_token<S, D>(
        &self,
        endpoint: &str,
        query: Option<S>,
        authed: bool,
        read_token: Option<&str>,
    ) -> Result<D, FlashnetError>
    where
        S: serde::Serialize,
        D: serde::de::DeserializeOwned,
    {
        let query_string = match query {
            Some(q) => {
                let qs = serde_urlencoded::to_string(&q).map_err(|e| {
                    FlashnetError::Generic(format!(
                        "Failed to serialize orchestra query params: {e}"
                    ))
                })?;
                if qs.is_empty() {
                    String::new()
                } else {
                    format!("?{qs}")
                }
            }
            None => String::new(),
        };
        let config = self.config().await?;
        let url = format!("{}/{}{}", config.base_url, endpoint, query_string);

        let mut headers = HashMap::new();
        add_content_type_header(&mut headers, ContentType::Json);
        if authed {
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", config.api_key),
            );
        }
        if let Some(token) = read_token {
            headers.insert("X-Read-Token".to_string(), token.to_string());
        }

        let response = self.http_client.get(url, Some(headers)).await?;
        if !response.is_success() {
            return Err(error_from_body(&response.body, response.status));
        }
        response
            .json::<D>()
            .map_err(|e| FlashnetError::Generic(format!("Failed to parse orchestra response: {e}")))
    }

    async fn post<S, D>(
        &self,
        endpoint: &str,
        body: &S,
        authed: bool,
        idempotency_key: Option<String>,
    ) -> Result<D, FlashnetError>
    where
        S: serde::Serialize,
        D: serde::de::DeserializeOwned,
    {
        let config = self.config().await?;
        let url = format!("{}/{}", config.base_url, endpoint);
        let body_json = serde_json::to_string(body).map_err(|e| {
            FlashnetError::Generic(format!("Failed to serialize orchestra body: {e}"))
        })?;

        let mut headers = HashMap::new();
        add_content_type_header(&mut headers, ContentType::Json);
        if authed {
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", config.api_key),
            );
        }
        if let Some(idem) = idempotency_key {
            headers.insert("X-Idempotency-Key".to_string(), idem);
        }

        let response = self
            .http_client
            .post(url, Some(headers), Some(body_json))
            .await?;

        if !response.is_success() {
            return Err(error_from_body(&response.body, response.status));
        }

        response
            .json::<D>()
            .map_err(|e| FlashnetError::Generic(format!("Failed to parse orchestra response: {e}")))
    }
}

/// Build a deterministic idempotency key so that retrying the same logical
/// request (same endpoint + scope) is safe.
pub fn derive_idempotency_key(scope: &str, key_input: &str) -> String {
    let hash = sha256::Hash::hash(format!("orchestra:{scope}:{key_input}").as_bytes());
    hash.to_string()
}

/// Classify an Orchestra error body, which has the shape
/// `{"error":{"code":"...","message":"..."}}`.
///
/// The amount-rejection codes become [`FlashnetError::AmountOutOfRange`] so
/// callers can react to them without matching on prose. Orchestra does not
/// include the bound it applied, so the error carries the direction only.
/// Anything else keeps its message and the HTTP status.
fn error_from_body(body: &str, status: u16) -> FlashnetError {
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let field = |name: &str| {
        parsed
            .as_ref()?
            .get("error")?
            .get(name)?
            .as_str()
            .map(String::from)
    };
    let reason = field("message").unwrap_or_else(|| body.to_string());
    match field("code").as_deref() {
        Some("amount_too_small") => FlashnetError::AmountOutOfRange {
            reason,
            too_small: true,
        },
        Some("amount_too_large" | "amount_exceeds_liquidity") => FlashnetError::AmountOutOfRange {
            reason,
            too_small: false,
        },
        _ => FlashnetError::Network {
            reason,
            code: Some(status),
        },
    }
}

#[cfg(test)]
mod error_body_tests {
    use super::*;

    #[test]
    fn amount_rejections_become_typed_errors() {
        let small = error_from_body(
            r#"{"error":{"code":"amount_too_small","message":"Amount too small"}}"#,
            400,
        );
        assert!(matches!(
            small,
            FlashnetError::AmountOutOfRange {
                too_small: true,
                ref reason
            } if reason == "Amount too small"
        ));

        for code in ["amount_too_large", "amount_exceeds_liquidity"] {
            let body = format!(r#"{{"error":{{"code":"{code}","message":"Amount too large"}}}}"#);
            assert!(matches!(
                error_from_body(&body, 400),
                FlashnetError::AmountOutOfRange {
                    too_small: false,
                    ..
                }
            ));
        }
    }

    #[test]
    fn other_errors_keep_the_message_and_status() {
        let err = error_from_body(
            r#"{"error":{"code":"route_unavailable","message":"No route"}}"#,
            503,
        );
        assert!(matches!(
            err,
            FlashnetError::Network { code: Some(503), ref reason } if reason == "No route"
        ));
    }

    #[test]
    fn a_non_json_body_is_carried_through_verbatim() {
        let err = error_from_body("upstream timeout", 502);
        assert!(matches!(
            err,
            FlashnetError::Network { code: Some(502), ref reason } if reason == "upstream timeout"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same scope + same input must always produce the same key.
    #[test]
    fn derive_idempotency_key_is_deterministic() {
        let a = derive_idempotency_key("submit", "q_abc");
        let b = derive_idempotency_key("submit", "q_abc");
        assert_eq!(a, b);
    }

    /// Different inputs (or scopes) must produce different keys: a single
    /// collision would conflate two logically distinct requests.
    #[test]
    fn derive_idempotency_key_disambiguates_inputs_and_scopes() {
        let by_input = derive_idempotency_key("submit", "q_abc");
        let other_input = derive_idempotency_key("submit", "q_xyz");
        assert_ne!(by_input, other_input);

        let other_scope = derive_idempotency_key("refund", "q_abc");
        assert_ne!(by_input, other_scope);
    }
}
