use std::{collections::HashMap, ops::Not, sync::Arc};

use bitcoin::{
    bech32::{self, Bech32m, Hrp},
    hashes::{Hash, HashEngine, sha256},
};
use prost_types::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;
use web_time::SystemTime;

use crate::{
    Network,
    address::SparkAddress,
    operator::{
        OperatorPool,
        rpc::{
            self,
            spark_token::{
                CommitTransactionRequest, QueryTokenMetadataRequest, QueryTokenOutputsRequest,
                QueryTokenTransactionsRequest, SignatureWithIndex, StartTransactionRequest,
            },
        },
    },
    services::{
        QueryTokenTransactionsFilter, ServiceError, TokenMetadata, TokenOutputWithPrevOut,
        TokenTransaction, TransferTokenOutput,
    },
    signer::Signer,
    utils::paging::{PagingFilter, PagingResult, pager},
};

const MAX_TOKEN_TX_INPUTS: usize = 500;

const HRP_STR_MAINNET: &str = "btkn";
const HRP_STR_TESTNET: &str = "btknt";
const HRP_STR_REGTEST: &str = "btknrt";
const HRP_STR_SIGNET: &str = "btkns";

#[derive(Clone)]
pub struct TokenOutputs {
    pub metadata: TokenMetadata,
    pub outputs: Vec<TokenOutputWithPrevOut>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokensConfig {
    pub expected_withdraw_bond_sats: u64,
    pub expected_withdraw_relative_block_locktime: u64,
    pub transaction_validity_duration_seconds: u64,
}

pub struct TokenService<S> {
    tokens_outputs: Mutex<HashMap<String, TokenOutputs>>,
    signer: Arc<S>,
    operator_pool: Arc<OperatorPool<S>>,
    network: Network,
    split_secret_threshold: u32,
    tokens_config: TokensConfig,
}

impl<S: Signer> TokenService<S> {
    pub fn new(
        signer: Arc<S>,
        operator_pool: Arc<OperatorPool<S>>,
        network: Network,
        split_secret_threshold: u32,
        tokens_config: TokensConfig,
    ) -> Self {
        Self {
            tokens_outputs: Mutex::new(HashMap::new()),
            signer,
            operator_pool,
            network,
            split_secret_threshold,
            tokens_config,
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
        let metadata = self.query_tokens_metadata_inner(token_identifiers).await?;

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

    /// Returns the metadata for the given token identifiers.
    ///
    /// For token identifiers that are not found in the local cache, the metadata will be queried from the SE.
    pub async fn get_tokens_metadata(
        &self,
        token_identifiers: &[&str],
    ) -> Result<Vec<TokenMetadata>, ServiceError> {
        let cached_outputs = { self.tokens_outputs.lock().await.clone() };

        // Separate token identifiers into cached and uncached
        let mut cached_metadata = Vec::new();
        let mut uncached_identifiers = Vec::new();

        for token_id in token_identifiers {
            if let Some(outputs) = cached_outputs.get(*token_id) {
                cached_metadata.push(outputs.metadata.clone());
            } else {
                uncached_identifiers.push(*token_id);
            }
        }

        // Query metadata for uncached tokens
        let mut queried_metadata = Vec::new();
        if !uncached_identifiers.is_empty() {
            queried_metadata = self.query_tokens_metadata(&uncached_identifiers).await?;
        }

        // Combine cached and queried metadata
        let mut all_metadata = cached_metadata;
        all_metadata.extend(queried_metadata);

        Ok(all_metadata)
    }

    async fn query_tokens_metadata(
        &self,
        token_identifiers: &[&str],
    ) -> Result<Vec<TokenMetadata>, ServiceError> {
        let token_identifiers = token_identifiers
            .iter()
            .map(|id| {
                bech32m_decode_token_id(id, Some(self.network))
                    .map_err(|_| ServiceError::Generic("Invalid token id".to_string()))
            })
            .collect::<Result<Vec<Vec<u8>>, _>>()?;
        let metadata = self.query_tokens_metadata_inner(token_identifiers).await?;
        let metadata = metadata
            .into_iter()
            .map(|m| (m, self.network).try_into())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(metadata)
    }

    async fn query_tokens_metadata_inner(
        &self,
        token_identifiers: Vec<Vec<u8>>,
    ) -> Result<Vec<rpc::spark_token::TokenMetadata>, ServiceError> {
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

    /// Returns owned token outputs from the local cache.
    pub async fn get_tokens_outputs(&self) -> HashMap<String, TokenOutputs> {
        self.tokens_outputs.lock().await.clone()
    }

    pub async fn query_token_transactions_inner(
        &self,
        filter: QueryTokenTransactionsFilter,
        paging: PagingFilter,
    ) -> Result<PagingResult<TokenTransaction>, ServiceError> {
        let owner_public_keys = match filter.owner_public_keys {
            Some(keys) => keys
                .iter()
                .map(|k| k.serialize().to_vec())
                .collect::<Vec<_>>(),
            None => vec![self.signer.get_identity_public_key()?.serialize().to_vec()],
        };

        // TODO: ask for ordering field to be added to QueryTokenTransactionsRequest
        //  until then, PagingFilter's order is not being respected
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
                        bech32m_decode_token_id(id, Some(self.network))
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

    pub async fn transfer_tokens(
        &self,
        receiver_outputs: Vec<TransferTokenOutput>,
    ) -> Result<String, ServiceError> {
        // Validate parameters
        if receiver_outputs.is_empty() {
            return Err(ServiceError::Generic(
                "No receiver outputs provided".to_string(),
            ));
        }
        let token_id = receiver_outputs[0].token_id.clone();
        if receiver_outputs.iter().any(|o| o.token_id != token_id) {
            return Err(ServiceError::Generic(
                "All receiver outputs must have the same token id".to_string(),
            ));
        }

        let total_amount: u128 = receiver_outputs.iter().map(|o| o.amount).sum();

        // Get outputs matching token id
        let mut outputs = self.tokens_outputs.lock().await;
        let Some(this_token_outputs) = outputs.get_mut(&token_id) else {
            return Err(ServiceError::Generic(format!(
                "No tokens available for token id: {token_id}"
            )));
        };

        let inputs = Self::select_token_outputs(&this_token_outputs.outputs, total_amount)?;

        if inputs.len() > MAX_TOKEN_TX_INPUTS {
            // We may consider doing an intermediate self transfer here to aggregate the inputs
            return Err(ServiceError::Generic(format!(
                "Needed too many outputs ({}) to transfer tokens",
                inputs.len()
            )));
        }

        let partial_tx = self
            .build_partial_tx(inputs.clone(), receiver_outputs)
            .await?;

        let (txid, final_tx) = self
            .finalize_broadcast_transaction(partial_tx.clone())
            .await?;

        // Removed used outputs from local cache and add any change outputs
        this_token_outputs.outputs.retain(|o| !inputs.contains(o));
        let identity_public_key_bytes = self.signer.get_identity_public_key()?.serialize();
        final_tx
            .token_outputs
            .into_iter()
            .enumerate()
            .filter(|(_, o)| o.owner_public_key == identity_public_key_bytes)
            .try_for_each(|(vout, o)| -> Result<(), ServiceError> {
                this_token_outputs.outputs.push(TokenOutputWithPrevOut {
                    output: (o, self.network).try_into()?,
                    prev_tx_hash: txid.clone(),
                    prev_tx_vout: vout as u32,
                });
                Ok(())
            })?;

        Ok(txid)
    }

    /// Selects tokens to match a given amount.
    ///
    /// Prioritizes smaller outputs.
    fn select_token_outputs(
        outputs: &[TokenOutputWithPrevOut],
        amount: u128,
    ) -> Result<Vec<TokenOutputWithPrevOut>, ServiceError> {
        if outputs.iter().map(|o| o.output.token_amount).sum::<u128>() < amount {
            return Err(ServiceError::Generic(
                "Not enough outputs to transfer tokens".to_string(),
            ));
        }

        // If there's an exact match, return it
        if let Some(output) = outputs.iter().find(|o| o.output.token_amount == amount) {
            return Ok(vec![output.clone()]);
        }

        // TODO: support other selection strategies (JS supports either smallest or largest first)
        // Sort outputs by amount, smallest first
        let mut sorted_outputs = outputs.to_vec();
        sorted_outputs.sort_by_key(|o| o.output.token_amount);

        // Select outputs to match the amount
        let mut selected_outputs = Vec::new();
        let mut remaining_amount = amount;
        for output in sorted_outputs {
            if remaining_amount == 0 {
                break;
            }
            selected_outputs.push(output.clone());
            remaining_amount = remaining_amount.saturating_sub(output.output.token_amount);
        }

        // We should never get here, but just in case
        if remaining_amount > 0 {
            return Err(ServiceError::Generic(format!(
                "Not enough outputs to transfer tokens, remaining amount: {remaining_amount}"
            )));
        }

        Ok(selected_outputs)
    }

    async fn build_partial_tx(
        &self,
        mut inputs: Vec<TokenOutputWithPrevOut>,
        mut receiver_outputs: Vec<TransferTokenOutput>,
    ) -> Result<rpc::spark_token::TokenTransaction, ServiceError> {
        // Ensure inputs are ordered by vout ascending so that the input indices
        // used for owner signatures match the order expected by the SO, which sorts
        // inputs by "prevTokenTransactionVout" before validating signatures.
        inputs.sort_by_key(|o| o.prev_tx_vout);

        // If the inputs amount is greater than the outputs amount, we add a change output
        let inputs_amount = inputs.iter().map(|o| o.output.token_amount).sum::<u128>();
        let outputs_amount = receiver_outputs.iter().map(|o| o.amount).sum::<u128>();
        if inputs_amount > outputs_amount {
            receiver_outputs.push(TransferTokenOutput {
                token_id: receiver_outputs[0].token_id.clone(),
                amount: inputs_amount - outputs_amount,
                receiver_address: SparkAddress::new(
                    self.signer.get_identity_public_key()?,
                    self.network,
                    None,
                    None,
                ),
            });
        }

        // Prepare inputs
        let outputs_to_spend = inputs
            .iter()
            .map(|o| {
                Ok(rpc::spark_token::TokenOutputToSpend {
                    prev_token_transaction_hash: hex::decode(&o.prev_tx_hash)
                        .map_err(|_| ServiceError::Generic("Invalid prev tx hash".to_string()))?,
                    prev_token_transaction_vout: o.prev_tx_vout,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;
        let inputs = rpc::spark_token::token_transaction::TokenInputs::TransferInput(
            rpc::spark_token::TokenTransferInput { outputs_to_spend },
        );

        // Prepare outputs
        let token_outputs = receiver_outputs
            .iter()
            .map(|o| {
                Ok(rpc::spark_token::TokenOutput {
                    owner_public_key: o.receiver_address.identity_public_key.serialize().to_vec(),
                    token_identifier: Some(
                        bech32m_decode_token_id(&o.token_id, Some(self.network))
                            .map_err(|_| ServiceError::Generic("Invalid token id".to_string()))?,
                    ),
                    token_amount: o.amount.to_be_bytes().to_vec(),
                    ..Default::default()
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        // Build transaction
        let transaction = rpc::spark_token::TokenTransaction {
            version: 1,
            token_outputs,
            spark_operator_identity_public_keys: self.get_operator_identity_public_keys()?,
            expiry_time: None,
            network: self.network.to_proto_network().into(),
            client_created_timestamp: Some({
                let now_ms = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(|_| {
                        ServiceError::Generic(
                            "client_created_timestamp is before UNIX_EPOCH".to_string(),
                        )
                    })?;
                Timestamp {
                    seconds: now_ms.as_secs() as i64,
                    nanos: now_ms.subsec_nanos() as i32,
                }
            }),
            token_inputs: Some(inputs),
        };

        Ok(transaction)
    }

    fn get_operator_identity_public_keys(&self) -> Result<Vec<Vec<u8>>, ServiceError> {
        let operators = self.operator_pool.get_all_operators();
        let keys = operators
            .map(|o| o.identity_public_key.serialize().to_vec())
            .collect();
        Ok(keys)
    }

    async fn finalize_broadcast_transaction(
        &self,
        partial_tx: rpc::spark_token::TokenTransaction,
    ) -> Result<(String, rpc::spark_token::TokenTransaction), ServiceError> {
        let partial_tx_hash = partial_tx.compute_hash(true)?;

        // Sign inputs
        let mut owner_signatures: Vec<SignatureWithIndex> = Vec::new();
        let Some(rpc::spark_token::token_transaction::TokenInputs::TransferInput(input)) =
            partial_tx.token_inputs.as_ref()
        else {
            return Err(ServiceError::Generic(
                "Token inputs are required".to_string(),
            ));
        };
        let signature = self
            .signer
            .sign_hash_schnorr_with_identity_key(&partial_tx_hash)?
            .serialize()
            .to_vec();
        for i in 0..input.outputs_to_spend.len() {
            owner_signatures.push(SignatureWithIndex {
                signature: signature.clone(),
                input_index: i as u32,
            });
        }

        let start_response = self
            .operator_pool
            .get_coordinator()
            .client
            .start_transaction(StartTransactionRequest {
                identity_public_key: self.signer.get_identity_public_key()?.serialize().to_vec(),
                partial_token_transaction: Some(partial_tx.clone()),
                partial_token_transaction_owner_signatures: owner_signatures,
                validity_duration_seconds: self.tokens_config.transaction_validity_duration_seconds,
            })
            .await?;

        let Some(final_tx) = start_response.final_token_transaction else {
            return Err(ServiceError::Generic(
                "No final transaction returned from start_transaction".to_string(),
            ));
        };
        let Some(keyshare_info) = start_response.keyshare_info else {
            return Err(ServiceError::Generic(
                "No keyshare info returned from start_transaction".to_string(),
            ));
        };

        self.validate_token_transaction(&partial_tx, &final_tx, &keyshare_info)?;

        let final_tx_hash = final_tx.compute_hash(false)?;

        let per_operator_signatures =
            self.create_per_operator_signatures(&final_tx, &final_tx_hash)?;

        self.operator_pool
            .get_coordinator()
            .client
            .commit_transaction(CommitTransactionRequest {
                final_token_transaction: Some(final_tx.clone()),
                final_token_transaction_hash: final_tx_hash.clone(),
                input_ttxo_signatures_per_operator: per_operator_signatures,
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
            })
            .await?;

        Ok((hex::encode(final_tx_hash), final_tx))
    }

    fn validate_token_transaction(
        &self,
        partial_tx: &rpc::spark_token::TokenTransaction,
        final_tx: &rpc::spark_token::TokenTransaction,
        keyshare_info: &rpc::spark::SigningKeyshare,
    ) -> Result<(), ServiceError> {
        if final_tx.network != partial_tx.network {
            return Err(ServiceError::Generic(
                "Network mismatch between partial and final transaction".to_string(),
            ));
        }

        let partial_tx_inputs = partial_tx
            .token_inputs
            .as_ref()
            .ok_or(ServiceError::Generic(
                "Token inputs missing from partial tx".to_string(),
            ))?;
        let final_tx_inputs = final_tx.token_inputs.as_ref().ok_or(ServiceError::Generic(
            "Token inputs missing from final tx".to_string(),
        ))?;

        match (partial_tx_inputs, final_tx_inputs) {
            (
                rpc::spark_token::token_transaction::TokenInputs::TransferInput(partial_tx_input),
                rpc::spark_token::token_transaction::TokenInputs::TransferInput(final_tx_input),
            ) => {
                if partial_tx_input.outputs_to_spend.len() != final_tx_input.outputs_to_spend.len()
                {
                    return Err(ServiceError::Generic(
                        "Outputs to spend mismatch between partial and final tx".to_string(),
                    ));
                }

                for (partial_output, final_output) in partial_tx_input
                    .outputs_to_spend
                    .iter()
                    .zip(final_tx_input.outputs_to_spend.iter())
                {
                    if partial_output.prev_token_transaction_hash
                        != final_output.prev_token_transaction_hash
                    {
                        return Err(ServiceError::Generic(
                            "Prev token transaction hash mismatch between partial and final tx"
                                .to_string(),
                        ));
                    }

                    if partial_output.prev_token_transaction_vout
                        != final_output.prev_token_transaction_vout
                    {
                        return Err(ServiceError::Generic(
                            "Prev token transaction vout mismatch between partial and final tx"
                                .to_string(),
                        ));
                    }
                }
            }
            _ => {
                return Err(ServiceError::Generic(
                    "Unexpected token inputs type".to_string(),
                ));
            }
        }

        if partial_tx.spark_operator_identity_public_keys.len()
            != final_tx.spark_operator_identity_public_keys.len()
        {
            return Err(ServiceError::Generic(
                "Spark operator identity public keys mismatch between partial and final tx"
                    .to_string(),
            ));
        }

        if partial_tx.token_outputs.len() != final_tx.token_outputs.len() {
            return Err(ServiceError::Generic(
                "Token outputs mismatch between partial and final tx".to_string(),
            ));
        }

        for (partial_output, final_output) in partial_tx
            .token_outputs
            .iter()
            .zip(final_tx.token_outputs.iter())
        {
            if partial_output.owner_public_key != final_output.owner_public_key {
                return Err(ServiceError::Generic(
                    "Owner public key mismatch between partial and final tx".to_string(),
                ));
            }

            if partial_output.token_amount != final_output.token_amount {
                return Err(ServiceError::Generic(
                    "Token amount mismatch between partial and final tx".to_string(),
                ));
            }

            if let Some(final_withdraw_bond_sats) = final_output.withdraw_bond_sats
                && final_withdraw_bond_sats != self.tokens_config.expected_withdraw_bond_sats
            {
                return Err(ServiceError::Generic(
                    "Unexpected withdraw bond sats in final tx".to_string(),
                ));
            }

            if let Some(final_withdraw_relative_block_locktime) =
                final_output.withdraw_relative_block_locktime
                && final_withdraw_relative_block_locktime
                    != self.tokens_config.expected_withdraw_relative_block_locktime
            {
                return Err(ServiceError::Generic(
                    "Unexpected withdraw relative block locktime in final tx".to_string(),
                ));
            }
        }

        if keyshare_info.threshold != self.split_secret_threshold {
            return Err(ServiceError::Generic(
                "Unexpected threshold in keyshare info".to_string(),
            ));
        }

        if keyshare_info.owner_identifiers.len() != self.operator_pool.get_all_operators().count() {
            return Err(ServiceError::Generic(
                "Keyshare info owner identifiers amount differs from operators amount".to_string(),
            ));
        }

        for identifier in &keyshare_info.owner_identifiers {
            if self
                .operator_pool
                .get_all_operators()
                .any(|o| hex::encode(o.identifier.serialize()) == *identifier)
                .not()
            {
                return Err(ServiceError::Generic(
                    "Keyshare info owner identifier not found in operators".to_string(),
                ));
            }
        }

        if final_tx
            .client_created_timestamp
            .ok_or(ServiceError::Generic(
                "Client created timestamp is required".to_string(),
            ))?
            != partial_tx
                .client_created_timestamp
                .ok_or(ServiceError::Generic(
                    "Client created timestamp is required".to_string(),
                ))?
        {
            return Err(ServiceError::Generic(
                "Client created timestamp mismatch between partial and final tx".to_string(),
            ));
        }

        Ok(())
    }

    fn create_per_operator_signatures(
        &self,
        tx: &rpc::spark_token::TokenTransaction,
        tx_hash: &[u8],
    ) -> Result<Vec<rpc::spark_token::InputTtxoSignaturesPerOperator>, ServiceError> {
        let mut per_operator_signatures = Vec::new();

        for operator in self.operator_pool.get_all_operators() {
            let operator_identity_public_key_bytes =
                operator.identity_public_key.serialize().to_vec();

            let mut signatures = Vec::new();

            let rpc::spark_token::token_transaction::TokenInputs::TransferInput(input) =
                tx.token_inputs.as_ref().ok_or(ServiceError::Generic(
                    "Token inputs are required".to_string(),
                ))?
            else {
                return Err(ServiceError::Generic(
                    "Token transfer inputs are required".to_string(),
                ));
            };
            let inputs_len = input.outputs_to_spend.len();

            let tx_hash_hash = sha256::Hash::hash(tx_hash).to_byte_array().to_vec();
            let operator_pubkey_hash = sha256::Hash::hash(&operator_identity_public_key_bytes)
                .to_byte_array()
                .to_vec();
            let final_hash = sha256::Hash::hash(&[tx_hash_hash, operator_pubkey_hash].concat())
                .to_byte_array()
                .to_vec();

            let signature = self
                .signer
                .sign_hash_schnorr_with_identity_key(&final_hash)?
                .serialize()
                .to_vec();

            for i in 0..inputs_len {
                signatures.push(rpc::spark_token::SignatureWithIndex {
                    signature: signature.clone(),
                    input_index: i as u32,
                });
            }

            per_operator_signatures.push(rpc::spark_token::InputTtxoSignaturesPerOperator {
                ttxo_signatures: signatures,
                operator_identity_public_key: operator_identity_public_key_bytes,
            });
        }

        Ok(per_operator_signatures)
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

/// Decodes a token id from a string.
///
/// If a network is provided, it will be checked against the network in the token id.
pub(crate) fn bech32m_decode_token_id(
    token_id: &str,
    network: Option<Network>,
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
    if let Some(network) = network
        && bech32_network != network
    {
        return Err(ServiceError::Generic(format!(
            "Invalid network: {bech32_network}"
        )));
    }
    Ok(data)
}

const TOKEN_TRANSACTION_TRANSFER_TYPE: u32 = 3;

trait HashableTokenTransaction {
    fn compute_hash(&self, partial: bool) -> Result<Vec<u8>, ServiceError>;
}

impl HashableTokenTransaction for rpc::spark_token::TokenTransaction {
    fn compute_hash(&self, partial: bool) -> Result<Vec<u8>, ServiceError> {
        let mut all_hashes = Vec::new();

        let version_hash = sha256::Hash::hash(&self.version.to_be_bytes())
            .to_byte_array()
            .to_vec();
        all_hashes.push(version_hash);

        // We only support transfer transactions
        let tx_type_hash = sha256::Hash::hash(&TOKEN_TRANSACTION_TRANSFER_TYPE.to_be_bytes())
            .to_byte_array()
            .to_vec();
        all_hashes.push(tx_type_hash);

        let rpc::spark_token::token_transaction::TokenInputs::TransferInput(input) =
            self.token_inputs.as_ref().ok_or(ServiceError::Generic(
                "Token inputs are required".to_string(),
            ))?
        else {
            return Err(ServiceError::Generic(
                "Token transfer inputs are required".to_string(),
            ));
        };
        let inputs = &input.outputs_to_spend;
        let inputs_len = inputs.len() as u32;
        let inputs_len_hash = sha256::Hash::hash(&inputs_len.to_be_bytes())
            .to_byte_array()
            .to_vec();
        all_hashes.push(inputs_len_hash);

        for input in inputs {
            let mut engine = sha256::Hash::engine();
            engine.input(&input.prev_token_transaction_hash);
            engine.input(&input.prev_token_transaction_vout.to_be_bytes());
            all_hashes.push(sha256::Hash::from_engine(engine).to_byte_array().to_vec());
        }

        let outputs_len = self.token_outputs.len() as u32;
        let outputs_len_hash = sha256::Hash::hash(&outputs_len.to_be_bytes())
            .to_byte_array()
            .to_vec();
        all_hashes.push(outputs_len_hash);

        for output in &self.token_outputs {
            let mut engine = sha256::Hash::engine();
            if !partial && let Some(id) = &output.id {
                engine.input(id.as_bytes());
            }
            engine.input(&output.owner_public_key);

            if !partial {
                let revocation_commitment =
                    output
                        .revocation_commitment
                        .as_ref()
                        .ok_or(ServiceError::Generic(
                            "Revocation commitment is required".to_string(),
                        ))?;
                engine.input(revocation_commitment);

                let withdraw_bond_sats = output.withdraw_bond_sats.ok_or(ServiceError::Generic(
                    "Withdraw bond sats is required".to_string(),
                ))?;
                engine.input(&withdraw_bond_sats.to_be_bytes());

                let withdraw_relative_block_locktime = output
                    .withdraw_relative_block_locktime
                    .ok_or(ServiceError::Generic(
                        "Withdraw relative block locktime is required".to_string(),
                    ))?;
                engine.input(&withdraw_relative_block_locktime.to_be_bytes());
            }

            let zeroed_pubkey = vec![0; 33];
            let token_pubkey = output.token_public_key.as_ref().unwrap_or(&zeroed_pubkey);
            engine.input(token_pubkey);

            let token_identifier =
                output
                    .token_identifier
                    .as_ref()
                    .ok_or(ServiceError::Generic(
                        "Token identifier is required".to_string(),
                    ))?;
            engine.input(token_identifier);

            engine.input(&output.token_amount);

            all_hashes.push(sha256::Hash::from_engine(engine).to_byte_array().to_vec());
        }

        // Sort operator public keys before hashing
        let mut operator_public_keys = self.spark_operator_identity_public_keys.clone();
        operator_public_keys.sort_by(|a, b| {
            // Compare bytes one by one
            for (a_byte, b_byte) in a.iter().zip(b.iter()) {
                if a_byte != b_byte {
                    return a_byte.cmp(b_byte);
                }
            }
            // If all bytes match up to the shorter length, compare lengths
            a.len().cmp(&b.len())
        });

        let operator_pubkeys_len = operator_public_keys.len() as u32;
        let operator_pubkeys_len_hash = sha256::Hash::hash(&operator_pubkeys_len.to_be_bytes())
            .to_byte_array()
            .to_vec();
        all_hashes.push(operator_pubkeys_len_hash);

        for pubkey in operator_public_keys {
            all_hashes.push(sha256::Hash::hash(&pubkey).to_byte_array().to_vec());
        }

        let network_hash = sha256::Hash::hash(&self.network.to_be_bytes())
            .to_byte_array()
            .to_vec();
        all_hashes.push(network_hash);

        let unix_timestamp = self.client_created_timestamp.ok_or(ServiceError::Generic(
            "Client created timestamp is required".to_string(),
        ))?;
        let unix_timestamp_ms =
            unix_timestamp.seconds as u64 * 1000 + unix_timestamp.nanos as u64 / 1_000_000;
        let client_created_timestamp_hash = sha256::Hash::hash(&unix_timestamp_ms.to_be_bytes())
            .to_byte_array()
            .to_vec();
        all_hashes.push(client_created_timestamp_hash);

        if !partial {
            let expiry_time = self.expiry_time.map(|t| t.seconds as u64).unwrap_or(0);
            let expiry_time_hash = sha256::Hash::hash(&expiry_time.to_be_bytes())
                .to_byte_array()
                .to_vec();
            all_hashes.push(expiry_time_hash);
        }

        let final_hash = sha256::Hash::hash(&all_hashes.concat())
            .to_byte_array()
            .to_vec();

        Ok(final_hash)
    }
}

#[cfg(test)]
mod tests {
    use macros::test_all;
    use prost_types::Timestamp;

    use crate::{
        operator::rpc::{
            self,
            spark_token::{
                TokenOutput, TokenOutputToSpend, TokenTransferInput, token_transaction::TokenInputs,
            },
        },
        services::tokens::HashableTokenTransaction,
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_compute_token_transaction_hash_non_partial() {
        let tx =
            rpc::spark_token::TokenTransaction {
                version: 1,
                token_outputs: vec![TokenOutput {
                id: Some("660e8400-e29b-41d4-a716-446655440001".to_string()),
                owner_public_key: hex::decode(
                    "02c0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247c9",
                )
                .unwrap(),
                revocation_commitment: Some(
                    hex::decode(
                        "03d0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247ca",
                    )
                    .unwrap(),
                ),
                withdraw_bond_sats: Some(500),
                withdraw_relative_block_locktime: Some(50),
                token_public_key: Some(
                    hex::decode(
                        "02e0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cb",
                    )
                    .unwrap(),
                ),
                token_identifier: Some(
                    hex::decode("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                        .unwrap(),
                ),
                token_amount: 50_u128.to_be_bytes().to_vec(),
            }, TokenOutput {
                id: Some("660e8400-e29b-41d4-a716-446655440002".to_string()),
                owner_public_key: hex::decode(
                    "02f0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cc",
                )
                .unwrap(),
                revocation_commitment: Some(
                    hex::decode(
                        "03e0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cb",
                    )
                    .unwrap(),
                ),
                withdraw_bond_sats: Some(300),
                withdraw_relative_block_locktime: Some(30),
                token_public_key: Some(
                    hex::decode(
                        "02f0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cc",
                    )
                    .unwrap(),
                ),
                token_identifier: Some(
                    hex::decode("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                        .unwrap(),
                ),
                token_amount: 100_u128.to_be_bytes().to_vec(),
            }],
                spark_operator_identity_public_keys: vec![
                    hex::decode(
                        "02e0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cb",
                    )
                    .unwrap(),
                    hex::decode(
                        "02f0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cc",
                    )
                    .unwrap(),
                ],
                expiry_time: Some(Timestamp {
                    seconds: 2103123456,
                    nanos: 321000000,
                }),
                network: rpc::spark::Network::Mainnet as i32,
                client_created_timestamp: Some(Timestamp {
                    seconds: 1703123456,
                    nanos: 123000000,
                }),
                token_inputs: Some(TokenInputs::TransferInput(TokenTransferInput {
                    outputs_to_spend: vec![
                        TokenOutputToSpend {
                            prev_token_transaction_hash: hex::decode(
                                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                            )
                            .unwrap(),
                            prev_token_transaction_vout: 0,
                        },
                        TokenOutputToSpend {
                            prev_token_transaction_hash: hex::decode(
                                "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                            )
                            .unwrap(),
                            prev_token_transaction_vout: 1,
                        },
                    ],
                })),
            };

        let hash = tx.compute_hash(false).unwrap();
        // Value taken from JS implementation
        assert_eq!(
            hash,
            hex::decode("0b7b506a33722689744cdad140c8c02702a9ad779869637a5631281f6fbbe0eb")
                .unwrap()
        );
    }

    #[test_all]
    fn test_compute_token_transaction_hash_partial() {
        let tx =
            rpc::spark_token::TokenTransaction {
                version: 1,
                token_outputs: vec![TokenOutput {
                id: Some("660e8400-e29b-41d4-a716-446655440001".to_string()),
                owner_public_key: hex::decode(
                    "02c0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247c9",
                )
                .unwrap(),
                revocation_commitment: Some(
                    hex::decode(
                        "03d0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247ca",
                    )
                    .unwrap(),
                ),
                withdraw_bond_sats: Some(500),
                withdraw_relative_block_locktime: Some(50),
                token_public_key: Some(
                    hex::decode(
                        "02e0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cb",
                    )
                    .unwrap(),
                ),
                token_identifier: Some(
                    hex::decode("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                        .unwrap(),
                ),
                token_amount: 50_u128.to_be_bytes().to_vec(),
            }, TokenOutput {
                id: Some("660e8400-e29b-41d4-a716-446655440002".to_string()),
                owner_public_key: hex::decode(
                    "02f0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cc",
                )
                .unwrap(),
                revocation_commitment: Some(
                    hex::decode(
                        "03e0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cb",
                    )
                    .unwrap(),
                ),
                withdraw_bond_sats: Some(300),
                withdraw_relative_block_locktime: Some(30),
                token_public_key: Some(
                    hex::decode(
                        "02f0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cc",
                    )
                    .unwrap(),
                ),
                token_identifier: Some(
                    hex::decode("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                        .unwrap(),
                ),
                token_amount: 100_u128.to_be_bytes().to_vec(),
            }],
                spark_operator_identity_public_keys: vec![
                    hex::decode(
                        "02e0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cb",
                    )
                    .unwrap(),
                    hex::decode(
                        "02f0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247cc",
                    )
                    .unwrap(),
                ],
                expiry_time: Some(Timestamp {
                    seconds: 2103123456,
                    nanos: 321000000,
                }),
                network: rpc::spark::Network::Mainnet as i32,
                client_created_timestamp: Some(Timestamp {
                    seconds: 1703123456,
                    nanos: 123000000,
                }),
                token_inputs: Some(TokenInputs::TransferInput(TokenTransferInput {
                    outputs_to_spend: vec![
                        TokenOutputToSpend {
                            prev_token_transaction_hash: hex::decode(
                                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                            )
                            .unwrap(),
                            prev_token_transaction_vout: 0,
                        },
                        TokenOutputToSpend {
                            prev_token_transaction_hash: hex::decode(
                                "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                            )
                            .unwrap(),
                            prev_token_transaction_vout: 1,
                        },
                    ],
                })),
            };

        let hash = tx.compute_hash(true).unwrap();
        // Value taken from JS implementation
        assert_eq!(
            hash,
            hex::decode("2fb877692e90822551c7cfd522139a4119f2395c6c96677e41f5a1c68c872af0")
                .unwrap()
        );
    }
}
