use std::{collections::HashMap, sync::Arc, time::Duration};

use platform_utils::tokio;
use tracing::{info, warn};

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{self, QueryAllTokenOutputsRequest, spark_token::QueryTokenMetadataRequest},
    },
    signer::SparkSigner,
    token::{
        GetTokenOutputsFilter, ReservationPurpose, ReservationTarget, SelectionStrategy,
        TokenMetadata, TokenOutputService, TokenOutputStore, TokenOutputWithPrevOut, TokenOutputs,
        TokenOutputsPerStatus, TokenOutputsReservation, TokenOutputsReservationId,
        error::TokenOutputServiceError,
    },
};

const SELECT_TOKEN_OUTPUTS_MAX_RETRIES: u32 = 3;

pub struct SynchronousTokenOutputService {
    network: Network,
    operator_pool: Arc<OperatorPool>,
    state: Arc<dyn TokenOutputStore>,
    spark_signer: Arc<dyn SparkSigner>,
}

#[macros::async_trait]
impl TokenOutputService for SynchronousTokenOutputService {
    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError> {
        self.state.list_tokens_outputs().await
    }

    async fn get_token_balances(
        &self,
    ) -> Result<Vec<(TokenMetadata, u128)>, TokenOutputServiceError> {
        self.state.get_token_balances().await
    }

    async fn refresh_tokens_outputs(&self) -> Result<(), TokenOutputServiceError> {
        // Capture the start time before any network calls from the store's clock.
        // This uses the DB server time for database-backed stores to avoid clock skew.
        // Outputs added after this time will be preserved even if not in the refresh data.
        let refresh_started_at = self.state.now().await?;

        let outputs = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_token_outputs(QueryAllTokenOutputsRequest {
                owner_public_keys: vec![
                    self.spark_signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
                ],
                network: self.network.to_proto_network().into(),
                ..Default::default()
            })
            .await?;
        if outputs.is_empty() {
            // Clear stored token outputs if none are returned
            self.state
                .set_tokens_outputs(&TokenOutputs::default(), refresh_started_at)
                .await?;
            return Ok(());
        }

        // Raw token id to token outputs map
        let mut outputs_map: HashMap<Vec<u8>, Vec<TokenOutputWithPrevOut>> = HashMap::new();
        for output_with_previous_transaction_data in outputs {
            let Some(output) = &output_with_previous_transaction_data.output else {
                warn!("An empty output was returned from query_token_outputs, skipping");
                continue;
            };

            let token_id = output.token_identifier().to_vec();
            let token_output: TokenOutputWithPrevOut =
                (output_with_previous_transaction_data, self.network).try_into()?;

            outputs_map.entry(token_id).or_default().push(token_output);
        }

        // Fetch metadata for owned tokens
        let token_identifiers = outputs_map.keys().cloned().collect();
        let metadata = self.query_tokens_metadata(token_identifiers).await?;

        if metadata.len() != outputs_map.keys().len() {
            return Err(TokenOutputServiceError::Generic(
                "Metadata not found for all tokens".to_string(),
            ));
        }

        let mut token_outputs = TokenOutputs::default();
        for (token_id, outputs) in outputs_map {
            let token_metadata = metadata
                .iter()
                .find(|m| m.token_identifier == token_id)
                .ok_or_else(|| {
                    TokenOutputServiceError::Generic("Metadata not found".to_string())
                })?;
            token_outputs
                .metadata
                .push((token_metadata.clone(), self.network).try_into()?);
            token_outputs.outputs.extend(outputs);
        }

        self.state
            .set_tokens_outputs(&token_outputs, refresh_started_at)
            .await?;

        Ok(())
    }

    async fn get_token_metadata(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<TokenMetadata, TokenOutputServiceError> {
        Ok(self.state.get_token_outputs(filter).await?.metadata)
    }

    async fn update_token_outputs(
        &self,
        outputs_to_remove: &[(String, u32)],
        outputs_to_add: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError> {
        self.state
            .update_token_outputs(outputs_to_remove, outputs_to_add)
            .await
    }

    async fn reserve_token_outputs(
        &self,
        targets: &[(String, ReservationTarget)],
        purpose: ReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        for i in 0..SELECT_TOKEN_OUTPUTS_MAX_RETRIES {
            if let Ok(reservation) = self
                .state
                .reserve_token_outputs(
                    targets,
                    purpose,
                    preferred_outputs.clone(),
                    selection_strategy,
                )
                .await
            {
                return Ok(reservation);
            }

            info!("Failed to reserve token outputs, refreshing and retrying");
            self.refresh_tokens_outputs().await?;
            if self.any_target_unaffordable(targets).await? {
                break;
            }

            if i < SELECT_TOKEN_OUTPUTS_MAX_RETRIES - 1 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }

        Err(TokenOutputServiceError::InsufficientFunds)
    }

    async fn select_token_outputs(
        &self,
        targets: &[(String, ReservationTarget)],
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputs, TokenOutputServiceError> {
        for i in 0..SELECT_TOKEN_OUTPUTS_MAX_RETRIES {
            if let Ok(token_outputs) = self
                .state
                .select_token_outputs(targets, preferred_outputs.clone(), selection_strategy)
                .await
            {
                return Ok(token_outputs);
            }

            info!("Failed to select token outputs, refreshing and retrying");
            self.refresh_tokens_outputs().await?;
            if self.any_target_unaffordable(targets).await? {
                break;
            }

            if i < SELECT_TOKEN_OUTPUTS_MAX_RETRIES - 1 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }

        Err(TokenOutputServiceError::InsufficientFunds)
    }

    async fn reserve_token_outputs_by_outpoints(
        &self,
        outpoints: &[(String, u32)],
        purpose: ReservationPurpose,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        self.state
            .reserve_token_outputs_by_outpoints(outpoints, purpose)
            .await
    }

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        self.state.cancel_reservation(id).await
    }

    async fn finalize_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        self.state.finalize_reservation(id).await
    }
}

impl SynchronousTokenOutputService {
    /// Whether any target now asks for more than that token's balance. Retrying
    /// cannot help once one token is short, since the reservation is all-or-nothing.
    async fn any_target_unaffordable(
        &self,
        targets: &[(String, ReservationTarget)],
    ) -> Result<bool, TokenOutputServiceError> {
        for (token_identifier, target) in targets {
            let ReservationTarget::MinTotalValue(amount) = target else {
                continue;
            };
            let balance = self
                .state
                .get_token_outputs(GetTokenOutputsFilter::Identifier(token_identifier))
                .await?
                .balance();
            if *amount > balance {
                info!(
                    "Insufficient funds for token {token_identifier} after refresh: requested {amount}, balance {balance}"
                );
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn new(
        network: Network,
        operator_pool: Arc<OperatorPool>,
        state: Arc<dyn TokenOutputStore>,
        spark_signer: Arc<dyn SparkSigner>,
    ) -> Self {
        Self {
            network,
            operator_pool,
            state,
            spark_signer,
        }
    }

    async fn query_tokens_metadata(
        &self,
        token_identifiers: Vec<Vec<u8>>,
    ) -> Result<Vec<rpc::spark_token::TokenMetadata>, TokenOutputServiceError> {
        let metadata = self
            .operator_pool
            .get_coordinator()
            .client
            .query_token_metadata(QueryTokenMetadataRequest {
                token_identifiers,
                ..Default::default()
            })
            .await?
            .token_metadata;
        Ok(metadata)
    }
}
