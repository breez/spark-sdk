//! HTTP client for the Flashnet Orchestra cross-chain API.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use bitcoin::hashes::{Hash, sha256};
use platform_utils::{ContentType, HttpClient, add_content_type_header};
use spark_wallet::{SparkAddress, SparkWallet, TransferTokenOutput};
use tracing::debug;

use super::models::{
    EstimateRequest, EstimateResponse, QuoteRequest, QuoteResponse, Route, RoutesResponse,
    StatusResponse, SubmitRequestSpark, SubmitResponse,
};
use crate::cache::CacheStore;
use crate::config::OrchestraConfig;
use crate::error::FlashnetError;

const ROUTES_CACHE_KEY: &str = "orchestra_routes";
/// One hour — Orchestra routes are effectively static between deployments.
const ROUTES_TTL_MS: u128 = 60 * 60 * 1000;

pub struct OrchestraClient {
    config: OrchestraConfig,
    http_client: platform_utils::DefaultHttpClient,
    cache_store: CacheStore,
    spark_wallet: Arc<SparkWallet>,
}

impl OrchestraClient {
    pub fn new(config: OrchestraConfig, spark_wallet: Arc<SparkWallet>) -> Self {
        Self {
            config,
            http_client: platform_utils::DefaultHttpClient::default(),
            cache_store: CacheStore::default(),
            spark_wallet,
        }
    }

    /// Fetch all supported cross-chain routes. Responses are cached for
    /// [`ROUTES_TTL_MS`] so repeated parser/UI calls are synchronous.
    pub async fn routes(&self) -> Result<RoutesResponse, FlashnetError> {
        if let Some(cached) = self
            .cache_store
            .get::<RoutesResponse>(ROUTES_CACHE_KEY)
            .await?
        {
            return Ok(cached);
        }
        debug!("Orchestra: GET /v1/orchestration/routes (cache miss)");
        let response: RoutesResponse = self
            .get("v1/orchestration/routes", None::<()>, false)
            .await?;
        self.cache_store
            .set(ROUTES_CACHE_KEY, &response, ROUTES_TTL_MS)
            .await?;
        Ok(response)
    }

    /// Return routes where Spark is involved as source or destination.
    ///
    /// When `is_send` is `true`, returns routes with `source_chain == "spark"`
    /// (sending from Spark to another chain). When `false`, returns routes with
    /// `destination_chain == "spark"` (receiving into Spark from another chain).
    ///
    /// Driven by the cached [`Self::routes`] response — cheap to call
    /// repeatedly from the parser / UI layer.
    pub async fn spark_routes(&self, is_send: bool) -> Result<Vec<Route>, FlashnetError> {
        let response = self.routes().await?;
        Ok(response
            .routes
            .into_iter()
            .filter(|r| {
                if is_send {
                    r.source_chain.eq_ignore_ascii_case("spark")
                } else {
                    r.destination_chain.eq_ignore_ascii_case("spark")
                }
            })
            .collect())
    }

    /// Price preview (no auth).
    pub async fn estimate(
        &self,
        request: EstimateRequest,
    ) -> Result<EstimateResponse, FlashnetError> {
        debug!("Orchestra: GET /v1/orchestration/estimate");
        self.get("v1/orchestration/estimate", Some(request), false)
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

    /// Transfer the quoted `amount_in` to `deposit_address` via the Spark
    /// wallet, then submit the resulting tx hash to Orchestra. Mirrors the
    /// AMM client's `execute_swap` shape: the caller supplies the prepared
    /// quote and Orchestra returns a processing order id.
    ///
    /// * `quote_id` / `deposit_address` / `amount_in` come from the
    ///   [`QuoteResponse`] returned by [`Self::quote`].
    ///
    /// Submit an already-sent deposit tx hash for an existing quote.
    ///
    /// Requires auth and an idempotency key. The key is derived
    /// deterministically from the quote id so retries are safe.
    pub async fn submit_spark(
        &self,
        request: SubmitRequestSpark,
    ) -> Result<SubmitResponse, FlashnetError> {
        let idem = derive_idempotency_key("submit", &request.quote_id);
        debug!(
            "Orchestra: POST /v1/orchestration/submit quoteId={} idem={}",
            request.quote_id, idem
        );
        self.post("v1/orchestration/submit", &request, true, Some(idem))
            .await
    }

    /// Send `amount_in` sats (or tokens) to the Orchestra-provided
    /// `deposit_address` via the Spark wallet. Returns the resulting Spark
    /// transfer id / token tx hash, which `/submit` expects as
    /// `sparkTxHash`.
    pub async fn transfer_to_deposit(
        &self,
        deposit_address: &str,
        amount_in: u128,
        token_identifier: Option<&str>,
    ) -> Result<String, FlashnetError> {
        let receiver_address = SparkAddress::from_str(deposit_address).map_err(|e| {
            FlashnetError::Generic(format!(
                "Failed to parse Orchestra deposit address '{deposit_address}': {e}"
            ))
        })?;

        let id = match token_identifier {
            None => {
                // BTC sats — plain Spark transfer.
                let amount_sats = u64::try_from(amount_in)
                    .map_err(|e| FlashnetError::Generic(format!("amount_in exceeds u64: {e}")))?;
                self.spark_wallet
                    .transfer(amount_sats, &receiver_address, None)
                    .await?
                    .id
                    .to_string()
            }
            Some(token_identifier) => {
                // USDB (or other Spark token) — token transfer.
                // `token_identifier` is already a bech32m-encoded token id (e.g. btkn1x...).
                self.spark_wallet
                    .transfer_tokens(
                        vec![TransferTokenOutput {
                            token_id: token_identifier.to_string(),
                            amount: amount_in,
                            receiver_address,
                            spark_invoice: None,
                        }],
                        None,
                        None,
                    )
                    .await?
                    .hash
            }
        };
        Ok(id)
    }

    /// Look up an order by its id.
    pub async fn status_by_id(&self, order_id: &str) -> Result<StatusResponse, FlashnetError> {
        #[derive(serde::Serialize)]
        struct Query<'a> {
            id: &'a str,
        }
        self.get(
            "v1/orchestration/status",
            Some(Query { id: order_id }),
            true,
        )
        .await
    }

    /// Look up an order by the originating quote id (useful before `/submit`
    /// returns or when the order id is not yet known).
    pub async fn status_by_quote_id(
        &self,
        quote_id: &str,
    ) -> Result<StatusResponse, FlashnetError> {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Query<'a> {
            quote_id: &'a str,
        }
        self.get("v1/orchestration/status", Some(Query { quote_id }), true)
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
        let url = format!("{}/{}{}", self.config.base_url, endpoint, query_string);

        let mut headers = HashMap::new();
        add_content_type_header(&mut headers, ContentType::Json);
        if authed {
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", self.config.api_key),
            );
        }

        let response = self.http_client.get(url, Some(headers)).await?;
        if !response.is_success() {
            return Err(FlashnetError::Network {
                reason: extract_error_message(&response.body),
                code: Some(response.status),
            });
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
        let url = format!("{}/{}", self.config.base_url, endpoint);
        let body_json = serde_json::to_string(body).map_err(|e| {
            FlashnetError::Generic(format!("Failed to serialize orchestra body: {e}"))
        })?;

        let mut headers = HashMap::new();
        add_content_type_header(&mut headers, ContentType::Json);
        if authed {
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", self.config.api_key),
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
            return Err(FlashnetError::Network {
                reason: extract_error_message(&response.body),
                code: Some(response.status),
            });
        }

        response
            .json::<D>()
            .map_err(|e| FlashnetError::Generic(format!("Failed to parse orchestra response: {e}")))
    }
}

/// Build a deterministic idempotency key so that retrying the same logical
/// request (same endpoint + scope) is safe.
fn derive_idempotency_key(scope: &str, key_input: &str) -> String {
    let hash = sha256::Hash::hash(format!("orchestra:{scope}:{key_input}").as_bytes());
    hash.to_string()
}

/// Try to extract a human-readable message from an Orchestra JSON error body.
/// Orchestra errors follow the shape `{"error":{"code":"...","message":"..."}}`.
/// Returns the `message` field if present, otherwise the raw body.
fn extract_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| body.to_string())
}
