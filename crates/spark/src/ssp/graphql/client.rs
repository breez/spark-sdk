use graphql_client::{GraphQLQuery, Response};
use rand::Rng;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use platform_utils::tokio;
use platform_utils::{ContentType, HttpClient, add_content_type_header};
use tokio::time::sleep;

use crate::header_provider::HeaderProvider;
use crate::ssp::graphql::error::{GraphQLError, GraphQLResult};
use crate::ssp::graphql::queries::{
    self, claim_static_deposit, complete_coop_exit, coop_exit_fee_quote, delete_wallet_webhook,
    leaves_swap_fee_estimate, lightning_send_fee_estimate, register_wallet_webhook,
    request_coop_exit, request_lightning_receive, request_lightning_send, request_swap,
    static_deposit_quote, transfers, user_request, wallet_webhooks,
};
use crate::ssp::graphql::{
    BitcoinNetwork, ClaimStaticDeposit, CoopExitRequest, CurrencyAmount, GraphQLClientConfig,
    LeavesSwapRequest, LightningReceiveRequest, LightningSendRequest, SparkWalletWebhookEventType,
    StaticDepositQuote, WebhookEntry,
};
use crate::ssp::{
    ClaimStaticDepositInput, CoopExitFeeQuote, RequestCoopExitInput, RequestLightningReceiveInput,
    RequestLightningSendInput, RequestSwapInput, RetryConfig, SspTransfer,
};

pub(crate) async fn post_graphql_query<Q: GraphQLQuery, T>(
    client: &dyn HttpClient,
    url: &str,
    headers: &HashMap<String, String>,
    variables: T,
) -> GraphQLResult<Q::ResponseData>
where
    T: Serialize + Clone + Into<Q::Variables>,
{
    let body = Q::build_query(variables.into());
    let body_str =
        serde_json::to_string(&body).map_err(|e| GraphQLError::Serialization(e.to_string()))?;

    let mut all_headers = headers.clone();
    add_content_type_header(&mut all_headers, ContentType::Json);

    let response = client
        .post(url.to_string(), Some(all_headers), Some(body_str))
        .await?;

    let status_code = response.status;
    let text = &response.body;
    tracing::trace!("Response: {text:?}");
    if !response.is_success() {
        return Err(GraphQLError::Network {
            reason: text.clone(),
            code: Some(status_code),
        });
    }

    let json: Response<Q::ResponseData> = response
        .json()
        .map_err(|e| GraphQLError::Serialization(e.to_string()))?;
    if let Some(errors) = json.errors
        && !errors.is_empty()
    {
        return Err(GraphQLError::from_graphql_errors(&errors));
    }

    json.data.ok_or(GraphQLError::serialization(
        "Unable to deserialize response",
    ))
}

/// GraphQL client for interacting with the Spark server
pub struct GraphQLClient {
    client: Arc<dyn HttpClient>,
    base_url: String,
    schema_endpoint: String,
    retry_config: RetryConfig,
    header_provider: Arc<dyn HeaderProvider>,
}

impl GraphQLClient {
    /// Create a new GraphQLClient using the supplied HTTP client.
    ///
    /// All SDK instances built with the same `client` share its underlying
    /// pooled `reqwest::Client` and its baked-in user-agent.
    pub fn new_with_client(
        config: GraphQLClientConfig,
        header_provider: Arc<dyn HeaderProvider>,
        client: Arc<dyn HttpClient>,
    ) -> Self {
        let schema_endpoint = config
            .schema_endpoint
            .unwrap_or_else(|| String::from("graphql/spark/2025-03-19"));

        Self {
            client,
            base_url: config.base_url,
            schema_endpoint,
            retry_config: config.retry_config,
            header_provider,
        }
    }

    fn get_full_url(&self) -> String {
        format!("{}/{}", self.base_url, self.schema_endpoint)
    }

    pub async fn post_query_inner<Q: GraphQLQuery, T>(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        variables: T,
    ) -> GraphQLResult<Q::ResponseData>
    where
        T: Serialize + Clone + Into<Q::Variables>,
    {
        post_graphql_query::<Q, _>(self.client.as_ref(), url, headers, variables).await
    }

    /// Execute a raw GraphQL query.
    ///
    /// Retries once on a 401 (after re-fetching auth headers) and up to
    /// `retry_config.max_retries` times on transient 5xx responses with
    /// exponential backoff and jitter.
    pub async fn post_query<Q: GraphQLQuery, T>(
        &self,
        variables: T,
    ) -> GraphQLResult<Q::ResponseData>
    where
        T: Serialize + Clone + Into<Q::Variables>,
    {
        let full_url = self.get_full_url();
        let mut auth_retried = false;
        let mut server_attempt: u32 = 0;

        loop {
            let headers = self
                .header_provider
                .headers()
                .await
                .map_err(|e| GraphQLError::Authentication(e.to_string()))?;

            let err = match self
                .post_query_inner::<Q, T>(&full_url, &headers, variables.clone())
                .await
            {
                Ok(response) => return Ok(response),
                Err(e) => e,
            };

            tracing::debug!("Received error: {}", err);

            let GraphQLError::Network {
                code: Some(status_code),
                ..
            } = &err
            else {
                return Err(err);
            };

            if *status_code == 401 && !auth_retried {
                auth_retried = true;
                continue;
            }

            if (500..600).contains(status_code) && server_attempt < self.retry_config.max_retries {
                let base = self
                    .retry_config
                    .base_delay_ms
                    .saturating_mul(1u64 << server_attempt)
                    .min(self.retry_config.max_delay_ms);
                let jitter = rand::thread_rng().gen_range(0..=base / 2);
                let delay_ms = base.saturating_add(jitter);
                warn!(
                    "Received {} from SSP, retrying in {}ms (attempt {}/{})",
                    status_code,
                    delay_ms,
                    server_attempt + 1,
                    self.retry_config.max_retries
                );
                sleep(Duration::from_millis(delay_ms)).await;
                server_attempt += 1;
                continue;
            }

            return Err(err);
        }
    }

    /// Get a swap fee estimate
    pub async fn get_swap_fee_estimate(&self, amount_sats: u64) -> GraphQLResult<CurrencyAmount> {
        let vars = leaves_swap_fee_estimate::Variables {
            input: leaves_swap_fee_estimate::LeavesSwapFeeEstimateInput {
                total_amount_sats: amount_sats as i64,
            },
        };

        let response = self
            .post_query::<queries::LeavesSwapFeeEstimate, _>(vars)
            .await?;

        Ok(response.leaves_swap_fee_estimate.fee_estimate.into())
    }

    /// Get a lightning send fee estimate
    pub async fn get_lightning_send_fee_estimate(
        &self,
        encoded_invoice: &str,
        amount_sats: Option<u64>,
    ) -> GraphQLResult<CurrencyAmount> {
        let vars = lightning_send_fee_estimate::Variables {
            input: lightning_send_fee_estimate::LightningSendFeeEstimateInput {
                encoded_invoice: encoded_invoice.to_string(),
                amount_sats,
            },
        };

        let response = self
            .post_query::<queries::LightningSendFeeEstimate, _>(vars)
            .await?;

        Ok(response.lightning_send_fee_estimate.fee_estimate.into())
    }

    /// Get a coop exit fee quote
    pub async fn get_coop_exit_fee_quote(
        &self,
        leaf_external_ids: Vec<String>,
        withdrawal_address: &str,
    ) -> GraphQLResult<CoopExitFeeQuote> {
        let vars = coop_exit_fee_quote::Variables {
            input: coop_exit_fee_quote::CoopExitFeeQuoteInput {
                leaf_external_ids,
                withdrawal_address: withdrawal_address.to_string(),
            },
        };

        let response = self
            .post_query::<queries::CoopExitFeeQuote, _>(vars)
            .await?;

        Ok(response.coop_exit_fee_quote.quote.into())
    }

    /// Complete a cooperative exit
    pub async fn complete_coop_exit(
        &self,
        user_outbound_transfer_external_id: &str,
        coop_exit_request_id: &str,
    ) -> GraphQLResult<CoopExitRequest> {
        let vars = complete_coop_exit::Variables {
            input: complete_coop_exit::CompleteCoopExitInput {
                user_outbound_transfer_external_id: user_outbound_transfer_external_id.to_string(),
                coop_exit_request_id: Some(coop_exit_request_id.to_string()),
            },
        };

        let response = self
            .post_query::<queries::CompleteCoopExit, _>(vars)
            .await?;

        Ok(response.complete_coop_exit.request.into())
    }

    /// Request a cooperative exit
    pub async fn request_coop_exit(
        &self,
        input: RequestCoopExitInput,
    ) -> GraphQLResult<CoopExitRequest> {
        let vars = request_coop_exit::Variables { input };

        let response = self.post_query::<queries::RequestCoopExit, _>(vars).await?;

        Ok(response.request_coop_exit.request.into())
    }

    /// Request lightning receive
    pub async fn request_lightning_receive(
        &self,
        input: RequestLightningReceiveInput,
    ) -> GraphQLResult<LightningReceiveRequest> {
        let vars = request_lightning_receive::Variables { input };

        let response = self
            .post_query::<queries::RequestLightningReceive, _>(vars)
            .await?;

        Ok(response.request_lightning_receive.request.into())
    }

    /// Request lightning send
    pub async fn request_lightning_send(
        &self,
        input: RequestLightningSendInput,
    ) -> GraphQLResult<LightningSendRequest> {
        let vars = request_lightning_send::Variables { input };

        let response = self
            .post_query::<queries::RequestLightningSend, _>(vars)
            .await?;

        Ok(response.request_lightning_send.request.into())
    }

    /// Request a swap
    pub async fn request_swap(&self, input: RequestSwapInput) -> GraphQLResult<LeavesSwapRequest> {
        let vars = request_swap::Variables { input };

        let response = self.post_query::<queries::RequestSwap, _>(vars).await?;

        Ok(response.request_swap.request.into())
    }

    /// Get a lightning receive request by ID
    pub async fn get_lightning_receive_request(
        &self,
        request_id: &str,
    ) -> GraphQLResult<Option<LightningReceiveRequest>> {
        let vars = user_request::Variables {
            request_id: request_id.to_string(),
        };

        let response = self.post_query::<queries::UserRequest, _>(vars).await?;

        Ok(response.user_request.and_then(|user_request| {
            if let user_request::UserRequestUserRequest::LightningReceiveRequest(response) =
                user_request
            {
                Some(response.into())
            } else {
                None
            }
        }))
    }

    /// Get a lightning send request by ID
    pub async fn get_lightning_send_request(
        &self,
        request_id: &str,
    ) -> GraphQLResult<Option<LightningSendRequest>> {
        let vars = user_request::Variables {
            request_id: request_id.to_string(),
        };

        let response = self.post_query::<queries::UserRequest, _>(vars).await?;

        Ok(response.user_request.and_then(|user_request| {
            if let user_request::UserRequestUserRequest::LightningSendRequest(response) =
                user_request
            {
                Some(response.into())
            } else {
                None
            }
        }))
    }

    /// Get a leaves swap request by ID
    pub async fn get_leaves_swap_request(
        &self,
        request_id: &str,
    ) -> GraphQLResult<Option<LeavesSwapRequest>> {
        let vars = user_request::Variables {
            request_id: request_id.to_string(),
        };

        let response = self.post_query::<queries::UserRequest, _>(vars).await?;

        Ok(response.user_request.and_then(|user_request| {
            if let user_request::UserRequestUserRequest::LeavesSwapRequest(response) = user_request
            {
                Some(response.into())
            } else {
                None
            }
        }))
    }

    /// Get a cooperative exit request by ID
    pub async fn get_coop_exit_request(
        &self,
        request_id: &str,
    ) -> GraphQLResult<Option<CoopExitRequest>> {
        let vars = user_request::Variables {
            request_id: request_id.to_string(),
        };

        let response = self.post_query::<queries::UserRequest, _>(vars).await?;

        Ok(response.user_request.and_then(|user_request| {
            if let user_request::UserRequestUserRequest::CoopExitRequest(response) = user_request {
                Some(response.into())
            } else {
                None
            }
        }))
    }

    /// Get claim deposit quote
    pub async fn get_claim_deposit_quote(
        &self,
        transaction_id: String,
        output_index: u32,
        network: BitcoinNetwork,
    ) -> GraphQLResult<StaticDepositQuote> {
        let vars = static_deposit_quote::Variables {
            input: static_deposit_quote::StaticDepositQuoteInput {
                transaction_id: transaction_id.to_string(),
                output_index: output_index as i64,
                network,
            },
        };

        let response = self
            .post_query::<queries::StaticDepositQuote, _>(vars)
            .await?;

        Ok(response.static_deposit_quote.into())
    }

    /// Claim static deposit
    pub async fn claim_static_deposit(
        &self,
        input: ClaimStaticDepositInput,
    ) -> GraphQLResult<ClaimStaticDeposit> {
        let vars = claim_static_deposit::Variables { input };

        let response = self
            .post_query::<queries::ClaimStaticDeposit, _>(vars)
            .await?;

        Ok(response.claim_static_deposit.into())
    }

    /// Get transfers by IDs
    pub async fn get_transfers(
        &self,
        transfer_spark_ids: Vec<String>,
    ) -> GraphQLResult<Vec<SspTransfer>> {
        let vars = transfers::Variables { transfer_spark_ids };
        let response = self.post_query::<queries::Transfers, _>(vars).await?;
        Ok(response
            .transfers
            .into_iter()
            .map(SspTransfer::from)
            .collect())
    }

    /// Register a wallet webhook with the SSP
    pub async fn register_wallet_webhook(
        &self,
        url: &str,
        secret: &str,
        event_types: Vec<SparkWalletWebhookEventType>,
    ) -> GraphQLResult<String> {
        let vars = register_wallet_webhook::Variables {
            input: register_wallet_webhook::RegisterSparkWalletWebhookInput {
                url: url.to_string(),
                secret: secret.to_string(),
                event_types,
            },
        };

        let response = self
            .post_query::<queries::RegisterWalletWebhook, _>(vars)
            .await?;

        Ok(response.register_wallet_webhook.webhook_id)
    }

    /// Delete a wallet webhook from the SSP
    pub async fn delete_wallet_webhook(&self, webhook_id: &str) -> GraphQLResult<bool> {
        let vars = delete_wallet_webhook::Variables {
            input: delete_wallet_webhook::DeleteSparkWalletWebhookInput {
                webhook_id: webhook_id.to_string(),
            },
        };

        let response = self
            .post_query::<queries::DeleteWalletWebhook, _>(vars)
            .await?;

        Ok(response.delete_wallet_webhook.success)
    }

    /// List wallet webhooks from the SSP
    pub async fn list_wallet_webhooks(&self) -> GraphQLResult<Vec<WebhookEntry>> {
        let vars = wallet_webhooks::Variables {};

        let response = self.post_query::<queries::WalletWebhooks, _>(vars).await?;

        Ok(response
            .wallet_webhooks
            .webhooks
            .into_iter()
            .map(|w| WebhookEntry {
                webhook_id: w.webhook_id,
                url: w.url,
                event_types: w.event_types,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use macros::async_test_all;
    use platform_utils::HttpError;

    use super::*;
    use crate::header_provider::HeaderProviderError;

    /// Empty-list response for the `WalletWebhooks` query — used as a stand-in
    /// for "any successful GraphQL response" in the retry tests.
    const VALID_WEBHOOKS_RESPONSE: &str = r#"{"data":{"wallet_webhooks":{"webhooks":[]}}}"#;

    #[derive(Default)]
    struct MockHttpInner {
        responses: Mutex<VecDeque<(u16, String)>>,
        post_calls: AtomicUsize,
    }

    #[derive(Clone, Default)]
    struct MockHttpClient(Arc<MockHttpInner>);

    impl MockHttpClient {
        fn with_responses(responses: Vec<(u16, &str)>) -> Self {
            Self(Arc::new(MockHttpInner {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|(s, b)| (s, b.to_string()))
                        .collect(),
                ),
                post_calls: AtomicUsize::new(0),
            }))
        }

        fn post_calls(&self) -> usize {
            self.0.post_calls.load(Ordering::SeqCst)
        }
    }

    #[macros::async_trait]
    impl HttpClient for MockHttpClient {
        async fn get(
            &self,
            _url: String,
            _headers: Option<HashMap<String, String>>,
        ) -> Result<platform_utils::HttpResponse, HttpError> {
            unimplemented!("get not used in these tests")
        }

        async fn post(
            &self,
            _url: String,
            _headers: Option<HashMap<String, String>>,
            _body: Option<String>,
        ) -> Result<platform_utils::HttpResponse, HttpError> {
            self.0.post_calls.fetch_add(1, Ordering::SeqCst);
            let (status, body) = self
                .0
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| HttpError::Other("mock: no more scripted responses".to_string()))?;
            Ok(platform_utils::HttpResponse {
                status,
                body,
                headers: std::collections::HashMap::new(),
            })
        }

        async fn delete(
            &self,
            _url: String,
            _headers: Option<HashMap<String, String>>,
            _body: Option<String>,
        ) -> Result<platform_utils::HttpResponse, HttpError> {
            unimplemented!("delete not used in these tests")
        }
    }

    /// Static header provider that returns a fixed Bearer header — stand-in
    /// for the real challenge-response auth provider in retry tests.
    struct StaticHeaderProvider;

    #[macros::async_trait]
    impl HeaderProvider for StaticHeaderProvider {
        async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
            Ok(HashMap::from([(
                "Authorization".to_string(),
                "Bearer test-token".to_string(),
            )]))
        }
    }

    /// Build a `GraphQLClient` wired up with the mock HTTP client and a
    /// static header provider so `post_query` never triggers an
    /// authentication round-trip.
    async fn build_test_client(http: MockHttpClient, retry_config: RetryConfig) -> GraphQLClient {
        GraphQLClient {
            client: Arc::new(http),
            base_url: "http://test.invalid".to_string(),
            schema_endpoint: "graphql".to_string(),
            retry_config,
            header_provider: Arc::new(StaticHeaderProvider),
        }
    }

    /// Fast retry config that keeps tests snappy.
    fn fast_retry(max_retries: u32) -> RetryConfig {
        RetryConfig {
            max_retries,
            base_delay_ms: 1,
            max_delay_ms: 5,
        }
    }

    #[async_test_all]
    async fn post_query_succeeds_after_5xx_retries() {
        let http = MockHttpClient::with_responses(vec![
            (503, "<html>Bad Gateway</html>"),
            (502, ""),
            (200, VALID_WEBHOOKS_RESPONSE),
        ]);
        let handle = http.clone();
        let client = build_test_client(http, fast_retry(2)).await;

        let result = client.list_wallet_webhooks().await;
        assert!(result.is_ok(), "expected success, got {result:?}");
        assert_eq!(handle.post_calls(), 3);
    }

    #[async_test_all]
    async fn post_query_exhausts_5xx_retries() {
        let max_retries = 2;
        let attempts = (max_retries as usize) + 1;
        let http = MockHttpClient::with_responses(
            std::iter::repeat_n((500, "internal error"), attempts).collect(),
        );
        let handle = http.clone();
        let client = build_test_client(http, fast_retry(max_retries)).await;

        let err = client.list_wallet_webhooks().await.unwrap_err();
        assert!(
            matches!(
                err,
                GraphQLError::Network {
                    code: Some(500),
                    ..
                }
            ),
            "expected Network 500 after exhausting retries, got {err:?}"
        );
        assert_eq!(handle.post_calls(), attempts);
    }

    #[async_test_all]
    async fn post_query_does_not_retry_on_4xx() {
        let http = MockHttpClient::with_responses(vec![(400, "bad request")]);
        let handle = http.clone();
        let client = build_test_client(http, fast_retry(2)).await;

        let err = client.list_wallet_webhooks().await.unwrap_err();
        assert!(
            matches!(
                err,
                GraphQLError::Network {
                    code: Some(400),
                    ..
                }
            ),
            "expected Network 400, got {err:?}"
        );
        assert_eq!(handle.post_calls(), 1);
    }

    #[async_test_all]
    async fn post_query_retries_once_on_401() {
        let http = MockHttpClient::with_responses(vec![
            (401, "unauthorized"),
            (200, VALID_WEBHOOKS_RESPONSE),
        ]);
        let handle = http.clone();
        let client = build_test_client(http, fast_retry(2)).await;

        let result = client.list_wallet_webhooks().await;
        assert!(result.is_ok(), "expected success, got {result:?}");
        assert_eq!(handle.post_calls(), 2);
    }

    #[async_test_all]
    async fn post_query_does_not_retry_401_twice() {
        let http =
            MockHttpClient::with_responses(vec![(401, "unauthorized"), (401, "unauthorized")]);
        let handle = http.clone();
        let client = build_test_client(http, fast_retry(2)).await;

        let err = client.list_wallet_webhooks().await.unwrap_err();
        assert!(
            matches!(
                err,
                GraphQLError::Network {
                    code: Some(401),
                    ..
                }
            ),
            "expected Network 401, got {err:?}"
        );
        assert_eq!(handle.post_calls(), 2);
    }

    #[async_test_all]
    async fn post_query_respects_max_retries_zero() {
        let http = MockHttpClient::with_responses(vec![(500, "boom")]);
        let handle = http.clone();
        let client = build_test_client(http, fast_retry(0)).await;

        let err = client.list_wallet_webhooks().await.unwrap_err();
        assert!(
            matches!(
                err,
                GraphQLError::Network {
                    code: Some(500),
                    ..
                }
            ),
            "expected Network 500 with no retries, got {err:?}"
        );
        assert_eq!(handle.post_calls(), 1);
    }
}
