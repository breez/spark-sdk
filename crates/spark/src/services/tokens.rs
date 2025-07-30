use std::{collections::HashMap, sync::Arc};

use bitcoin::bech32::{self, Bech32m, Hrp};
use tokio::sync::Mutex;
use tracing::warn;

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::spark_token::{
            QueryTokenMetadataRequest, QueryTokenOutputsRequest, QueryTokenTransactionsRequest,
        },
    },
    services::{
        QueryTokenTransactionsFilter, ServiceError, TokenMetadata, TokenOutput, TokenTransaction,
    },
    signer::Signer,
    utils::paging::{PagingFilter, PagingResult, pager},
};

const HRP_STR_MAINNET: &str = "btkn";
const HRP_STR_TESTNET: &str = "btknt";
const HRP_STR_REGTEST: &str = "btknrt";
const HRP_STR_SIGNET: &str = "btkns";

#[derive(Clone)]
pub struct TokenOutputs {
    pub metadata: TokenMetadata,
    pub outputs: Vec<TokenOutput>,
}

pub struct TokenService<S> {
    tokens_outputs: Mutex<HashMap<String, TokenOutputs>>,
    signer: Arc<S>,
    operator_pool: Arc<OperatorPool<S>>,
    network: Network,
}

impl<S: Signer> TokenService<S> {
    pub fn new(signer: Arc<S>, operator_pool: Arc<OperatorPool<S>>, network: Network) -> Self {
        Self {
            tokens_outputs: Mutex::new(HashMap::new()),
            signer,
            operator_pool,
            network,
        }
    }

    /// Fetches all owned token outputs from the SE and updates the local cache.
    pub async fn refresh_tokens(&self) -> Result<(), ServiceError> {
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
            return Ok(());
        }

        // Raw token id to token outputs map
        let mut outputs_map: HashMap<Vec<u8>, Vec<TokenOutput>> = HashMap::new();

        for output in outputs {
            let Some(output) = output.output else {
                warn!("An empty output was returned from query_token_outputs, skipping");
                continue;
            };

            let token_id = output.token_identifier().to_vec();
            let token_outputs: TokenOutput = (output, self.network).try_into()?;

            outputs_map
                .entry(token_id)
                .or_insert(vec![])
                .push(token_outputs);
        }

        // Fetch metadata for owned tokens
        let token_identifiers = outputs_map.keys().cloned().collect();
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

        if metadata.len() != outputs_map.keys().len() {
            return Err(ServiceError::Generic(
                "Metadata not found for all tokens".to_string(),
            ));
        }

        let outputs_with_metadata_map = outputs_map
            .into_iter()
            .map(|(token_id, outputs)| {
                let metadata = metadata
                    .iter()
                    .find(|m| m.token_identifier == token_id)
                    .ok_or_else(|| ServiceError::Generic("Metadata not found".to_string()))?;
                let metadata: TokenMetadata = (metadata.clone(), self.network).try_into()?;
                Ok((
                    metadata.identifier.clone(),
                    TokenOutputs { metadata, outputs },
                ))
            })
            .collect::<Result<HashMap<String, TokenOutputs>, ServiceError>>()?;

        let mut tokens_outputs = self.tokens_outputs.lock().await;
        *tokens_outputs = outputs_with_metadata_map;

        Ok(())
    }

    /// Returns owned token outputs from the local cache.
    pub async fn get_tokens_outputs(&self) -> HashMap<String, TokenOutputs> {
        self.tokens_outputs.lock().await.clone()
    }

    pub async fn query_token_transactions_inner(
        &self,
        filter: QueryTokenTransactionsFilter,
        paging: PagingFilter,
    ) -> Result<PagingResult<TokenTransaction>, ServiceError> {
        let mut owner_public_keys = filter
            .owner_public_keys
            .iter()
            .map(|k| k.serialize().to_vec())
            .collect::<Vec<_>>();
        if owner_public_keys.is_empty() {
            owner_public_keys.push(self.signer.get_identity_public_key()?.serialize().to_vec());
        }
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .query_token_transactions(QueryTokenTransactionsRequest {
                output_ids: filter.output_ids,
                owner_public_keys,
                issuer_public_keys: filter
                    .issuer_public_keys
                    .iter()
                    .map(|k| k.serialize().to_vec())
                    .collect(),
                token_identifiers: filter
                    .token_ids
                    .iter()
                    .map(|id| {
                        bech32m_decode_token_id(id, self.network)
                            .map_err(|_| ServiceError::Generic("Invalid token id".to_string()))
                    })
                    .collect::<Result<Vec<Vec<u8>>, _>>()?,
                token_transaction_hashes: filter
                    .token_transaction_hashes
                    .iter()
                    .map(|id| {
                        hex::decode(id).map_err(|_| {
                            ServiceError::Generic("Invalid token transaction hash".to_string())
                        })
                    })
                    .collect::<Result<Vec<Vec<u8>>, _>>()?,
                limit: paging.limit as i64,
                offset: paging.offset as i64,
            })
            .await?;

        Ok(PagingResult {
            items: response
                .token_transactions_with_status
                .into_iter()
                .map(|t| (t, self.network).try_into())
                .collect::<Result<Vec<TokenTransaction>, _>>()?,
            next: paging.next_from_offset(response.offset),
        })
    }

    pub async fn query_token_transactions(
        &self,
        filter: QueryTokenTransactionsFilter,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<TokenTransaction>, ServiceError> {
        let transactions = match paging {
            Some(paging) => {
                self.query_token_transactions_inner(filter, paging)
                    .await?
                    .items
            }
            None => {
                pager(
                    |p| self.query_token_transactions_inner(filter.clone(), p),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(transactions)
    }
}

pub(crate) fn bech32m_encode_token_id(
    raw_token_id: &[u8],
    network: Network,
) -> Result<String, ServiceError> {
    let hrp_str = match network {
        Network::Mainnet => HRP_STR_MAINNET,
        Network::Testnet => HRP_STR_TESTNET,
        Network::Regtest => HRP_STR_REGTEST,
        Network::Signet => HRP_STR_SIGNET,
    };
    let hrp = Hrp::parse_unchecked(hrp_str);
    let bech32 = bech32::encode::<Bech32m>(hrp, raw_token_id)
        .map_err(|e| ServiceError::Generic(format!("Failed to encode token id: {e}")))?;
    Ok(bech32)
}

pub(crate) fn bech32m_decode_token_id(
    token_id: &str,
    network: Network,
) -> Result<Vec<u8>, ServiceError> {
    let (hrp, data) = bech32::decode(token_id)
        .map_err(|e| ServiceError::Generic(format!("Failed to decode token id: {e}")))?;
    let bech32_network = match hrp.as_str() {
        "btkn" => Network::Mainnet,
        "btknt" => Network::Testnet,
        "btknrt" => Network::Regtest,
        "btkns" => Network::Signet,
        _ => return Err(ServiceError::Generic(format!("Invalid network: {hrp}"))),
    };
    if bech32_network != network {
        return Err(ServiceError::Generic(format!(
            "Invalid network: {bech32_network}"
        )));
    }
    Ok(data)
}
