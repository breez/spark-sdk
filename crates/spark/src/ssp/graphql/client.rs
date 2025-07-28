use base64::Engine;
use graphql_client::{GraphQLQuery, Response};
use reqwest::Client;
use reqwest::header::HeaderMap;
use serde::Serialize;
use std::sync::Arc;

use crate::signer::Signer;
use crate::ssp::graphql::auth_provider::AuthProvider;
use crate::ssp::graphql::error::{GraphQLError, GraphQLResult};
use crate::ssp::graphql::queries::{
    self, claim_static_deposit, complete_coop_exit, complete_leaves_swap, coop_exit_fee_quote,
    get_challenge, leaves_swap_fee_estimate, lightning_send_fee_estimate, request_coop_exit,
    request_leaves_swap, request_lightning_receive, request_lightning_send, static_deposit_quote,
    transfers, user_request, verify_challenge,
};
use crate::ssp::graphql::{
    BitcoinNetwork, ClaimStaticDeposit, CoopExitRequest, CurrencyAmount, GraphQLClientConfig,
    LeavesSwapRequest, LightningReceiveRequest, LightningSendRequest, StaticDepositQuote, Transfer,
};
use crate::ssp::{
    ClaimStaticDepositInput, CoopExitFeeQuote, RequestCoopExitInput, RequestLeavesSwapInput,
    RequestLightningReceiveInput, RequestLightningSendInput,
};

/// GraphQL client for interacting with the Spark server
pub struct GraphQLClient<S> {
    client: Client,
    base_url: String,
    schema_endpoint: String,
    auth_provider: Arc<AuthProvider>,
    signer: Arc<S>,
}

impl<S: Signer> GraphQLClient<S> {
    /// Create a new GraphQLClient with the given configuration, and signer
    pub fn new(config: GraphQLClientConfig, signer: Arc<S>) -> Self {
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
            signer,
        }
    }

    fn get_full_url(&self) -> String {
        format!("{}/{}", self.base_url, self.schema_endpoint)
    }

    pub async fn post_query_inner<Q: GraphQLQuery, T>(
        &self,
        url: &str,
        headers: &HeaderMap,
        variables: T,
    ) -> GraphQLResult<Q::ResponseData>
    where
        T: Serialize + Clone + Into<Q::Variables>,
    {
        let body = Q::build_query(variables.into());
        let response = self
            .client
            .post(url)
            .headers(headers.clone())
            .json(&body)
            .send()
            .await?;

        let status_code = response.status();
        let text = response.text().await?;
        tracing::debug!("Response: {text:?}");
        if status_code.is_client_error() {
            return Err(GraphQLError::Network {
                reason: text,
                code: Some(status_code.as_u16()),
            });
        }

        let json: Response<Q::ResponseData> =
            serde_json::from_str(&text).map_err(|e| GraphQLError::Serialization(e.to_string()))?;
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
    pub async fn post_query<Q: GraphQLQuery, T>(
        &self,
        variables: T,
        needs_auth: bool,
    ) -> GraphQLResult<Q::ResponseData>
    where
        T: Serialize + Clone + Into<Q::Variables>,
    {
        if needs_auth && !self.auth_provider.is_authorized().await? {
            self.authenticate().await?;
            tracing::debug!("Authenticated successfully with ssp");
        }

        let full_url = self.get_full_url();
        let mut headers = HeaderMap::new();
        self.auth_provider.add_auth_headers(&mut headers).await?;

        match self
            .post_query_inner::<Q, T>(&full_url, &headers, variables.clone())
            .await
        {
            Ok(response) => Ok(response),
            Err(e) => {
                tracing::debug!("Received error: {}", e.to_string());
                if let GraphQLError::Network {
                    code: Some(status_code),
                    ..
                } = e.clone()
                {
                    if status_code == reqwest::StatusCode::UNAUTHORIZED.as_u16() && needs_auth {
                        self.authenticate().await?;
                        let mut headers = HeaderMap::new();
                        self.auth_provider.add_auth_headers(&mut headers).await?;

                        return self
                            .post_query_inner::<Q, T>(&full_url, &headers, variables)
                            .await;
                    }
                }
                Err(e)
            }
        }
    }

    /// Authenticate with the server using challenge-response
    async fn authenticate(&self) -> GraphQLResult<()> {
        tracing::debug!("Authenticating with ssp");
        self.auth_provider.remove_auth().await?;

        // Get the identity public key
        let identity_public_key = hex::encode(self.signer.get_identity_public_key()?.serialize());

        // Get a challenge from the server
        let challenge_vars = get_challenge::Variables {
            input: get_challenge::GetChallengeInput {
                public_key: identity_public_key.clone(),
            },
        };

        let full_url = self.get_full_url();
        let headers = HeaderMap::new();

        let challenge_response = self
            .post_query_inner::<queries::GetChallenge, _>(&full_url, &headers, challenge_vars)
            .await?;

        tracing::debug!("Received challenge from ssp");
        // Decode the base64 protected challenge
        let challenge_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&challenge_response.get_challenge.protected_challenge)
            .map_err(|e| GraphQLError::serialization(e.to_string()))?;

        tracing::debug!("Decoded challenge bytes: {}", challenge_bytes.len());
        // Sign the challenge with the identity key
        let signature = self
            .signer
            .sign_message_ecdsa_with_identity_key(&challenge_bytes)?
            .serialize_der()
            .to_vec();

        // Verify the challenge
        let verify_vars = verify_challenge::Variables {
            input: verify_challenge::VerifyChallengeInput {
                protected_challenge: challenge_response.get_challenge.protected_challenge,
                signature: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&signature),
                identity_public_key,
                provider: None, // No provider specified
            },
        };

        let verify_response = self
            .post_query_inner::<queries::VerifyChallenge, _>(&full_url, &headers, verify_vars)
            .await?;

        // Store the session token
        self.auth_provider
            .set_auth(
                verify_response.verify_challenge.session_token,
                verify_response.verify_challenge.valid_until,
            )
            .await?;

        Ok(())
    }

    /// Get a swap fee estimate
    pub async fn get_swap_fee_estimate(&self, amount_sats: u64) -> GraphQLResult<CurrencyAmount> {
        let vars = leaves_swap_fee_estimate::Variables {
            input: leaves_swap_fee_estimate::LeavesSwapFeeEstimateInput {
                total_amount_sats: amount_sats as i64,
            },
        };

        let response = self
            .post_query::<queries::LeavesSwapFeeEstimate, _>(vars, true)
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
            .post_query::<queries::LightningSendFeeEstimate, _>(vars, true)
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
            .post_query::<queries::CoopExitFeeQuote, _>(vars, true)
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
                coop_exit_request_id: coop_exit_request_id.to_string(),
            },
        };

        let response = self
            .post_query::<queries::CompleteCoopExit, _>(vars, true)
            .await?;

        Ok(response.complete_coop_exit.request.into())
    }

    /// Request a cooperative exit
    pub async fn request_coop_exit(
        &self,
        input: RequestCoopExitInput,
    ) -> GraphQLResult<CoopExitRequest> {
        let vars = request_coop_exit::Variables { input };

        let response = self
            .post_query::<queries::RequestCoopExit, _>(vars, true)
            .await?;

        Ok(response.request_coop_exit.request.into())
    }

    /// Request lightning receive
    pub async fn request_lightning_receive(
        &self,
        input: RequestLightningReceiveInput,
    ) -> GraphQLResult<LightningReceiveRequest> {
        let vars = request_lightning_receive::Variables { input };

        let response = self
            .post_query::<queries::RequestLightningReceive, _>(vars, true)
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
            .post_query::<queries::RequestLightningSend, _>(vars, true)
            .await?;

        Ok(response.request_lightning_send.request.into())
    }

    /// Request a leaves swap
    pub async fn request_leaves_swap(
        &self,
        input: RequestLeavesSwapInput,
    ) -> GraphQLResult<LeavesSwapRequest> {
        let vars = request_leaves_swap::Variables { input };

        let response = self
            .post_query::<queries::RequestLeavesSwap, _>(vars, true)
            .await?;

        Ok(response.request_leaves_swap.request.into())
    }

    /// Complete a leaves swap
    pub async fn complete_leaves_swap(
        &self,
        adaptor_secret_key: &str,
        user_outbound_transfer_external_id: &str,
        leaves_swap_request_id: &str,
    ) -> GraphQLResult<LeavesSwapRequest> {
        let vars = complete_leaves_swap::Variables {
            input: complete_leaves_swap::CompleteLeavesSwapInput {
                adaptor_secret_key: adaptor_secret_key.to_string(),
                user_outbound_transfer_external_id: user_outbound_transfer_external_id.to_string(),
                leaves_swap_request_id: leaves_swap_request_id.to_string(),
            },
        };

        let response = self
            .post_query::<queries::CompleteLeavesSwap, _>(vars, true)
            .await?;

        Ok(response.complete_leaves_swap.request.into())
    }

    /// Get a lightning receive request by ID
    pub async fn get_lightning_receive_request(
        &self,
        request_id: &str,
    ) -> GraphQLResult<Option<LightningReceiveRequest>> {
        let vars = user_request::Variables {
            request_id: request_id.to_string(),
        };

        let response = self
            .post_query::<queries::UserRequest, _>(vars, true)
            .await?;

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

        let response = self
            .post_query::<queries::UserRequest, _>(vars, true)
            .await?;

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

        let response = self
            .post_query::<queries::UserRequest, _>(vars, true)
            .await?;

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

        let response = self
            .post_query::<queries::UserRequest, _>(vars, true)
            .await?;

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
            .post_query::<queries::StaticDepositQuote, _>(vars, true)
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
            .post_query::<queries::ClaimStaticDeposit, _>(vars, true)
            .await?;

        Ok(response.claim_static_deposit.into())
    }

    /// Get transfers by IDs
    pub async fn get_transfers(
        &self,
        transfer_spark_ids: Vec<&str>,
    ) -> GraphQLResult<Vec<Transfer>> {
        let vars = transfers::Variables {
            transfer_spark_ids: transfer_spark_ids.into_iter().map(String::from).collect(),
        };

        let response = self.post_query::<queries::Transfers, _>(vars, true).await?;

        Ok(response.transfers.into_iter().map(Transfer::from).collect())
    }
}
