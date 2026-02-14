use base64::Engine;
use bitcoin::secp256k1::PublicKey;
use graphql_client::{GraphQLQuery, Response};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error};

use platform_utils::{ContentType, HttpClient, add_content_type_header, create_http_client};

use crate::default_user_agent;
use crate::session_manager::{Session, SessionManager};
use crate::signer::Signer;
use crate::ssp::graphql::error::{GraphQLError, GraphQLResult};
use crate::ssp::graphql::queries::{
    self, claim_static_deposit, complete_coop_exit, coop_exit_fee_quote, get_challenge,
    leaves_swap_fee_estimate, lightning_send_fee_estimate, request_coop_exit,
    request_lightning_receive, request_lightning_send, request_swap, static_deposit_quote,
    transfers, user_request, verify_challenge,
};
use crate::ssp::graphql::{
    BitcoinNetwork, ClaimStaticDeposit, CoopExitRequest, CurrencyAmount, GraphQLClientConfig,
    LeavesSwapRequest, LightningReceiveRequest, LightningSendRequest, StaticDepositQuote,
};
use crate::ssp::{
    ClaimStaticDepositInput, CoopExitFeeQuote, RequestCoopExitInput, RequestLightningReceiveInput,
    RequestLightningSendInput, RequestSwapInput, SspTransfer,
};

/// GraphQL client for interacting with the Spark server
pub struct GraphQLClient {
    client: Box<dyn HttpClient>,
    base_url: String,
    schema_endpoint: String,
    signer: Arc<dyn Signer>,
    session_manager: Arc<dyn SessionManager>,
    ssp_identity_public_key: PublicKey,
}

impl GraphQLClient {
    /// Create a new GraphQLClient with the given configuration, and signer
    pub fn new(
        config: GraphQLClientConfig,
        signer: Arc<dyn Signer>,
        session_manager: Arc<dyn SessionManager>,
    ) -> Self {
        let schema_endpoint = config
            .schema_endpoint
            .unwrap_or_else(|| String::from("graphql/spark/2025-03-19"));

        let user_agent = config.user_agent.unwrap_or_else(default_user_agent);
        Self {
            client: create_http_client(Some(&user_agent)),
            base_url: config.base_url,
            schema_endpoint,
            signer,
            session_manager,
            ssp_identity_public_key: config.ssp_identity_public_key,
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
        let body = Q::build_query(variables.into());
        let body_str =
            serde_json::to_string(&body).map_err(|e| GraphQLError::Serialization(e.to_string()))?;

        // Merge Content-Type header with provided headers
        let mut all_headers = headers.clone();
        add_content_type_header(&mut all_headers, ContentType::Json);

        let response = self
            .client
            .post(url.to_string(), Some(all_headers), Some(body_str))
            .await?;

        let status_code = response.status;
        let text = &response.body;
        tracing::trace!("Response: {text:?}");
        if (400..500).contains(&status_code) {
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

    /// Execute a raw GraphQL query
    pub async fn post_query<Q: GraphQLQuery, T>(
        &self,
        variables: T,
    ) -> GraphQLResult<Q::ResponseData>
    where
        T: Serialize + Clone + Into<Q::Variables>,
    {
        let session = self.get_session().await?;
        let full_url = self.get_full_url();
        let mut headers = HashMap::new();
        self.add_auth_headers(&session, &mut headers)?;

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
                    && status_code == 401
                {
                    let session = self.get_session().await?;
                    let mut headers = HashMap::new();
                    self.add_auth_headers(&session, &mut headers)?;

                    return self
                        .post_query_inner::<Q, T>(&full_url, &headers, variables)
                        .await;
                }
                Err(e)
            }
        }
    }

    /// Authenticate with the server using challenge-response
    async fn authenticate(&self) -> GraphQLResult<Session> {
        tracing::debug!("Authenticating with ssp");

        // Get the identity public key
        let identity_public_key =
            hex::encode(self.signer.get_identity_public_key().await?.serialize());

        // Get a challenge from the server
        let challenge_vars = get_challenge::Variables {
            input: get_challenge::GetChallengeInput {
                public_key: identity_public_key.clone(),
            },
        };

        let full_url = self.get_full_url();
        let headers = HashMap::new();

        let challenge_response = self
            .post_query_inner::<queries::GetChallenge, _>(&full_url, &headers, challenge_vars)
            .await?;

        tracing::debug!("Received challenge from ssp");
        // Decode the base64 protected challenge
        let challenge_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&challenge_response.get_challenge.protected_challenge)
            .map_err(|e| GraphQLError::serialization(e.to_string()))?;

        tracing::debug!("Decoded challenge bytes length: {}", challenge_bytes.len());
        // Sign the challenge with the identity key
        let signature = self
            .signer
            .sign_message_ecdsa_with_identity_key(&challenge_bytes)
            .await?
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

        Ok(Session {
            token: verify_response.verify_challenge.session_token,
            expiration: verify_response
                .verify_challenge
                .valid_until
                .timestamp()
                .try_into()
                .map_err(|_| {
                    GraphQLError::Authentication("Invalid expiration timestamp".to_string())
                })?,
        })
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

    async fn get_session(&self) -> GraphQLResult<Session> {
        let current_session = self
            .session_manager
            .get_session(&self.ssp_identity_public_key)
            .await;
        let valid_session = match current_session {
            Ok(session) => {
                if session.is_valid() {
                    session
                } else {
                    self.authenticate().await?
                }
            }
            Err(e) => {
                match e {
                    crate::session_manager::SessionManagerError::NotFound => {
                        debug!("Operator session not found, authenticating")
                    }
                    crate::session_manager::SessionManagerError::Generic(e) => {
                        error!("Failed to get operator session from session manager: {}", e)
                    }
                };
                self.authenticate().await?
            }
        };
        self.session_manager
            .set_session(&self.ssp_identity_public_key, valid_session.clone())
            .await?;
        Ok(valid_session)
    }

    fn add_auth_headers(
        &self,
        session: &Session,
        headers: &mut HashMap<String, String>,
    ) -> Result<(), GraphQLError> {
        add_content_type_header(headers, ContentType::Json);
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", session.token),
        );

        Ok(())
    }
}
