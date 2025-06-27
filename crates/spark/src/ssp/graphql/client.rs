use base64::Engine;
use reqwest::Client;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

use crate::core::Network;
use crate::signer::Signer;
use crate::ssp::graphql::auth_provider::AuthProvider;
use crate::ssp::graphql::error::{GraphQLError, GraphQLResult};
use crate::ssp::graphql::{mutations, queries, types::*};

/// GraphQL client for interacting with the Spark server
pub struct GraphQLClient<S>
where
    S: Signer,
{
    client: Client,
    base_url: String,
    schema_endpoint: String,
    auth_provider: Arc<AuthProvider>,
    network: Network,
    signer: S,
}

impl<S> GraphQLClient<S>
where
    S: Signer,
{
    /// Create a new GraphQLClient with the given configuration, network, and signer
    pub fn new(config: GraphQLClientConfig, network: Network, signer: S) -> Self {
        let schema_endpoint = config
            .schema_endpoint
            .unwrap_or_else(|| String::from("graphql/spark/2025-03-19"));

        Self {
            client: Client::builder()
                .user_agent("rust-spark/0.1.0")
                .build()
                .unwrap(),
            base_url: config.base_url,
            schema_endpoint,
            auth_provider: Arc::new(AuthProvider::new()),
            network,
            signer,
        }
    }

    fn get_full_url(&self) -> String {
        format!("{}/{}", self.base_url, self.schema_endpoint)
    }

    // TODO: WASM handling of Send + Sync
    async fn execute_raw_query_inner<T, V>(
        &self,
        url: &str,
        headers: &HeaderMap,
        query: &str,
        variables: V,
    ) -> GraphQLResult<T>
    where
        T: DeserializeOwned + 'static,
        V: Serialize + Send + Sync,
    {
        let graphql_query = GraphQLQuery {
            query: query.to_string(),
            variables: serde_json::to_value(variables)
                .map_err(|e| GraphQLError::Serialization(e.to_string()))?
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            operation_name: None,
        };

        let response = self
            .client
            .post(url)
            .headers(headers.clone())
            .json(&graphql_query)
            .send()
            .await?;

        let json: GraphQLResponse<T> = response.json().await?;

        if let Some(errors) = json.errors {
            if !errors.is_empty() {
                return Err(GraphQLError::from_graphql_errors(&errors));
            }
        }

        json.data.ok_or(GraphQLError::serialization(
            "Unable to deserialize response",
        ))
    }

    /// Execute a raw GraphQL query
    async fn execute_raw_query<T, V>(
        &self,
        query: &str,
        variables: V,
        needs_auth: bool,
    ) -> GraphQLResult<T>
    where
        T: DeserializeOwned + 'static,
        V: Serialize + Send + Sync + Clone,
    {
        if needs_auth && !self.auth_provider.is_authorized()? {
            self.authenticate().await?;
        }

        let full_url = self.get_full_url();
        let mut headers = HeaderMap::new();
        self.auth_provider.add_auth_headers(&mut headers)?;

        match self
            .execute_raw_query_inner(&full_url, &headers, query, variables.clone())
            .await
        {
            Ok(response) => Ok(response),
            Err(e) => {
                if let GraphQLError::Network {
                    code: Some(status_code),
                    ..
                } = e.clone()
                {
                    if status_code == reqwest::StatusCode::UNAUTHORIZED.as_u16() && needs_auth {
                        self.authenticate().await?;
                        let mut headers = HeaderMap::new();
                        self.auth_provider.add_auth_headers(&mut headers)?;

                        self.execute_raw_query_inner(&full_url, &headers, query, variables)
                            .await?
                    }
                }
                Err(e)
            }
        }
    }

    /// Authenticate with the server using challenge-response
    async fn authenticate(&self) -> GraphQLResult<()> {
        self.auth_provider.remove_auth()?;

        // Get the identity public key
        let identity_public_key = hex::encode(self.signer.get_identity_public_key()?.serialize());

        // Get a challenge from the server
        let challenge_vars = serde_json::json!({
            "public_key": identity_public_key
        });

        let full_url = self.get_full_url();
        let headers = HeaderMap::new();
        let challenge_response: GetChallengeOutput = self
            .execute_raw_query_inner(
                &full_url,
                &headers,
                &mutations::get_challenge(),
                challenge_vars,
            )
            .await?;

        // Decode the base64 protected challenge
        let challenge_bytes = base64::engine::general_purpose::STANDARD
            .decode(&challenge_response.protected_challenge)
            .map_err(|_| GraphQLError::serialization("Failed to decode challenge"))?;

        // Sign the challenge with the identity key
        let signature = self
            .signer
            .sign_message_ecdsa_with_identity_key(&challenge_bytes)?
            .serialize_der()
            .to_vec();

        // Verify the challenge
        let verify_vars = serde_json::json!({
            "protected_challenge": challenge_response.protected_challenge,
            "signature": base64::engine::general_purpose::STANDARD.encode(signature),
            "identity_public_key": identity_public_key
        });

        let verify_response: VerifyChallengeOutput = self
            .execute_raw_query_inner(
                &full_url,
                &headers,
                &mutations::verify_challenge(),
                verify_vars,
            )
            .await?;

        // Store the session token
        self.auth_provider
            .set_auth(verify_response.session_token, verify_response.valid_until)?;

        Ok(())
    }

    /// Get a swap fee estimate
    pub async fn get_swap_fee_estimate(
        &self,
        amount_sats: u64,
    ) -> GraphQLResult<LeavesSwapFeeEstimateOutput> {
        let vars = serde_json::json!({
            "total_amount_sats": amount_sats
        });

        #[derive(Deserialize)]
        struct Response {
            leaves_swap_fee_estimate: LeavesSwapFeeEstimateOutput,
        }

        let response: Response = self
            .execute_raw_query(&queries::leaves_swap_fee_estimate(), vars, true)
            .await?;

        Ok(response.leaves_swap_fee_estimate)
    }

    /// Get a lightning send fee estimate
    pub async fn get_lightning_send_fee_estimate(
        &self,
        encoded_invoice: &str,
    ) -> GraphQLResult<LightningSendFeeEstimateOutput> {
        let vars = serde_json::json!({
            "encoded_invoice": encoded_invoice
        });

        #[derive(Deserialize)]
        struct Response {
            lightning_send_fee_estimate: LightningSendFeeEstimateOutput,
        }

        let response: Response = self
            .execute_raw_query(&queries::lightning_send_fee_estimate(), vars, true)
            .await?;

        Ok(response.lightning_send_fee_estimate)
    }

    /// Get a coop exit fee estimate
    pub async fn get_coop_exit_fee_estimate(
        &self,
        leaf_external_ids: Vec<String>,
        withdrawal_address: &str,
    ) -> GraphQLResult<CoopExitFeeEstimatesOutput> {
        let vars = serde_json::json!({
            "leaf_external_ids": leaf_external_ids,
            "withdrawal_address": withdrawal_address
        });

        #[derive(Deserialize)]
        struct Response {
            coop_exit_fee_estimates: CoopExitFeeEstimatesOutput,
        }

        let response: Response = self
            .execute_raw_query(&queries::coop_exit_fee_estimate(), vars, true)
            .await?;

        Ok(response.coop_exit_fee_estimates)
    }

    /// Complete a cooperative exit
    pub async fn complete_coop_exit(
        &self,
        user_outbound_transfer_external_id: &str,
        coop_exit_request_id: &str,
    ) -> GraphQLResult<CoopExitRequest> {
        let vars = serde_json::json!({
            "user_outbound_transfer_external_id": user_outbound_transfer_external_id,
            "coop_exit_request_id": coop_exit_request_id
        });

        #[derive(Deserialize)]
        struct Response {
            complete_coop_exit: CompleteCoopExitResponse,
        }

        #[derive(Deserialize)]
        struct CompleteCoopExitResponse {
            request: CoopExitRequest,
        }

        let response: Response = self
            .execute_raw_query(&mutations::complete_coop_exit(), vars, true)
            .await?;

        Ok(response.complete_coop_exit.request)
    }

    /// Request a cooperative exit
    pub async fn request_coop_exit(
        &self,
        input: RequestCoopExitInput,
    ) -> GraphQLResult<CoopExitRequest> {
        let vars =
            serde_json::to_value(input).map_err(|e| GraphQLError::Serialization(e.to_string()))?;

        #[derive(Deserialize)]
        struct Response {
            request_coop_exit: RequestCoopExitResponse,
        }

        #[derive(Deserialize)]
        struct RequestCoopExitResponse {
            request: CoopExitRequest,
        }

        let response: Response = self
            .execute_raw_query(&mutations::request_coop_exit(), vars, true)
            .await?;

        Ok(response.request_coop_exit.request)
    }

    /// Request lightning receive
    pub async fn request_lightning_receive(
        &self,
        input: RequestLightningReceiveInput,
    ) -> GraphQLResult<LightningReceiveRequest> {
        let vars =
            serde_json::to_value(input).map_err(|e| GraphQLError::Serialization(e.to_string()))?;

        #[derive(Deserialize)]
        struct Response {
            request_lightning_receive: RequestLightningReceiveResponse,
        }

        #[derive(Deserialize)]
        struct RequestLightningReceiveResponse {
            request: LightningReceiveRequest,
        }

        let response: Response = self
            .execute_raw_query(&mutations::request_lightning_receive(), vars, true)
            .await?;

        Ok(response.request_lightning_receive.request)
    }

    /// Request lightning send
    pub async fn request_lightning_send(
        &self,
        input: RequestLightningSendInput,
    ) -> GraphQLResult<LightningSendRequest> {
        let vars =
            serde_json::to_value(input).map_err(|e| GraphQLError::Serialization(e.to_string()))?;

        #[derive(Deserialize)]
        struct Response {
            request_lightning_send: RequestLightningSendResponse,
        }

        #[derive(Deserialize)]
        struct RequestLightningSendResponse {
            request: LightningSendRequest,
        }

        let response: Response = self
            .execute_raw_query(&mutations::request_lightning_send(), vars, true)
            .await?;

        Ok(response.request_lightning_send.request)
    }

    /// Get claim deposit quote
    pub async fn get_claim_deposit_quote(
        &self,
        transaction_id: String,
        output_index: i32,
        network: BitcoinNetwork,
    ) -> GraphQLResult<StaticDepositQuoteOutput> {
        let vars = serde_json::json!({
            "transaction_id": transaction_id,
            "output_index": output_index,
            "network": network
        });

        #[derive(Deserialize)]
        struct Response {
            static_deposit_quote: StaticDepositQuoteOutput,
        }

        let response: Response = self
            .execute_raw_query(&queries::get_claim_deposit_quote(), vars, true)
            .await?;

        Ok(response.static_deposit_quote)
    }

    /// Get a lightning receive request by ID
    pub async fn get_lightning_receive_request(
        &self,
        id: &str,
    ) -> GraphQLResult<Option<LightningReceiveRequest>> {
        let vars = serde_json::json!({
            "request_id": id
        });

        #[derive(Deserialize)]
        struct Response {
            user_request: Option<LightningReceiveRequest>,
        }

        let response: Response = self
            .execute_raw_query(&queries::user_request(), vars, true)
            .await?;

        Ok(response.user_request)
    }

    /// Get a lightning send request by ID
    pub async fn get_lightning_send_request(
        &self,
        id: &str,
    ) -> GraphQLResult<Option<LightningSendRequest>> {
        let vars = serde_json::json!({
            "request_id": id
        });

        #[derive(Deserialize)]
        struct Response {
            user_request: Option<LightningSendRequest>,
        }

        let response: Response = self
            .execute_raw_query(&queries::user_request(), vars, true)
            .await?;

        Ok(response.user_request)
    }

    /// Get a cooperative exit request by ID
    pub async fn get_coop_exit_request(&self, id: &str) -> GraphQLResult<Option<CoopExitRequest>> {
        let vars = serde_json::json!({
            "request_id": id
        });

        #[derive(Deserialize)]
        struct Response {
            user_request: Option<CoopExitRequest>,
        }

        let response: Response = self
            .execute_raw_query(&queries::user_request(), vars, true)
            .await?;

        Ok(response.user_request)
    }
}
