use std::{collections::HashMap, sync::Arc};

use tracing::warn;

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            self,
            spark_token::{QueryTokenMetadataRequest, QueryTokenOutputsRequest},
        },
    },
    signer::Signer,
    token::{
        GetTokenOutputsFilter, TokenMetadata, TokenOutputService, TokenOutputStore,
        TokenOutputWithPrevOut, TokenOutputs, TokenOutputsReservation, TokenOutputsReservationId,
        error::TokenOutputServiceError,
    },
};

pub struct SynchronousTokenOutputService {
    network: Network,
    operator_pool: Arc<OperatorPool>,
    state: Arc<dyn TokenOutputStore>,
    signer: Arc<dyn Signer>,
}

#[macros::async_trait]
impl TokenOutputService for SynchronousTokenOutputService {
    async fn list_tokens_outputs(&self) -> Result<Vec<TokenOutputs>, TokenOutputServiceError> {
        self.state.list_tokens_outputs().await
    }

    async fn refresh_tokens_outputs(&self) -> Result<(), TokenOutputServiceError> {
        let outputs = self
            .operator_pool
            .get_coordinator()
            .client
            .query_token_outputs(QueryTokenOutputsRequest {
                owner_public_keys: vec![
                    self.signer.get_identity_public_key()?.serialize().to_vec(),
                ],
                network: self.network.to_proto_network().into(),
                ..Default::default()
            })
            .await?
            .outputs_with_previous_transaction_data;
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
    ) -> Result<Option<TokenMetadata>, TokenOutputServiceError> {
        self.state
            .get_token_outputs(filter)
            .await
            .map(|to| to.map(|to| to.metadata))
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
        amount: u128,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        self.state
            .reserve_token_outputs(token_identifier, amount, preferred_outputs)
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
