use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio_with_wasm::alias as tokio;
use tracing::{info, warn};

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{self, QueryAllTokenOutputsRequest, spark_token::QueryTokenMetadataRequest},
    },
    signer::Signer,
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
    signer: Arc<dyn Signer>,
}

#[macros::async_trait]
impl TokenOutputService for SynchronousTokenOutputService {
    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError> {
        self.state.list_tokens_outputs().await
    }

    async fn refresh_tokens_outputs(&self) -> Result<(), TokenOutputServiceError> {
        let outputs = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_token_outputs(QueryAllTokenOutputsRequest {
                owner_public_keys: vec![
                    self.signer
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
            self.state.set_tokens_outputs(&[]).await?;
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

        let token_outputs = outputs_map
            .into_iter()
            .map(|(token_id, outputs)| {
                let metadata = metadata
                    .iter()
                    .find(|m| m.token_identifier == token_id)
                    .ok_or_else(|| {
                        TokenOutputServiceError::Generic("Metadata not found".to_string())
                    })?;
                let metadata = (metadata.clone(), self.network).try_into()?;
                Ok(TokenOutputs { metadata, outputs })
            })
            .collect::<Result<Vec<TokenOutputs>, TokenOutputServiceError>>()?;

        self.state.set_tokens_outputs(&token_outputs).await?;

        Ok(())
    }

    async fn get_token_metadata(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<TokenMetadata, TokenOutputServiceError> {
        Ok(self.state.get_token_outputs(filter).await?.metadata)
    }

    async fn insert_token_outputs(
        &self,
        token_outputs: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError> {
        self.state.insert_token_outputs(token_outputs).await
    }

    async fn reserve_token_outputs(
        &self,
        token_identifier: &str,
        target: ReservationTarget,
        purpose: ReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        let mut reservation: Option<TokenOutputsReservation> = None;

        for i in 0..SELECT_TOKEN_OUTPUTS_MAX_RETRIES {
            let reserve_res = self
                .state
                .reserve_token_outputs(
                    token_identifier,
                    target,
                    purpose,
                    preferred_outputs.clone(),
                    selection_strategy,
                )
                .await;
            if let Ok(token_outputs_reservation) = reserve_res {
                reservation = Some(token_outputs_reservation);
                break;
            }

            info!("Failed to reserve token outputs, refreshing and retrying");
            self.refresh_tokens_outputs().await?;
            let token_balance = self
                .state
                .get_token_outputs(GetTokenOutputsFilter::Identifier(token_identifier))
                .await?
                .balance();
            if let ReservationTarget::MinTotalValue(amount) = &target
                && *amount > token_balance
            {
                info!(
                    "Insufficient funds to select token outputs after refresh: requested {amount}, balance {token_balance}"
                );
                break;
            }

            if i < SELECT_TOKEN_OUTPUTS_MAX_RETRIES - 1 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }

        reservation.ok_or_else(|| TokenOutputServiceError::InsufficientFunds)
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
    pub fn new(
        network: Network,
        operator_pool: Arc<OperatorPool>,
        state: Arc<dyn TokenOutputStore>,
        signer: Arc<dyn Signer>,
    ) -> Self {
        Self {
            network,
            operator_pool,
            state,
            signer,
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
