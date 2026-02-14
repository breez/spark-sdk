use std::{ops::Not, sync::Arc};

use bitcoin::{
    bech32::{self, Bech32m, Hrp},
    hashes::{Hash, HashEngine, sha256},
    secp256k1::PublicKey,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use web_time::{SystemTime, UNIX_EPOCH};

use crate::{
    Network,
    address::SparkAddress,
    operator::{
        OperatorPool,
        rpc::{
            self, OperatorRpcError,
            spark_token::{
                CommitTransactionRequest, QueryTokenMetadataRequest,
                QueryTokenTransactionsByFilters, QueryTokenTransactionsByTxHash,
                QueryTokenTransactionsRequest, SignatureWithIndex, StartTransactionRequest,
                query_token_transactions_request::QueryType,
            },
        },
    },
    services::{
        FreezeIssuerTokenResponse, QueryTokenTransactionsFilter, ReceiverTokenOutput, ServiceError,
        TokenTransaction, TransferObserver, TransferTokenOutput,
    },
    signer::Signer,
    token::{
        GetTokenOutputsFilter, ReservationPurpose, ReservationTarget, SelectionStrategy,
        TokenMetadata, TokenOutputService, TokenOutputWithPrevOut, TokenOutputs,
        with_reserved_token_outputs,
    },
    utils::{
        paging::{PagingFilter, PagingResult, pager},
        time::web_time_to_prost_timestamp,
    },
};

const MAX_TOKEN_TX_INPUTS: usize = 500;
const MAX_TRANSFER_TOKEN_TOO_MANY_OUTPUTS_RETRY_ATTEMPTS: usize = 3;
const MAX_TOKEN_PREEMPTED_RETRY_ATTEMPTS: usize = 3;

/// Checks if an error indicates a transaction was preempted because token outputs
/// were already spent.
fn is_transaction_preempted_error(error: &OperatorRpcError) -> bool {
    matches!(error, OperatorRpcError::Connection(status) if status.code() == tonic::Code::Aborted)
}

const HRP_STR_MAINNET: &str = "btkn";
const HRP_STR_TESTNET: &str = "btknt";
const HRP_STR_REGTEST: &str = "btknrt";
const HRP_STR_SIGNET: &str = "btkns";

pub const BURN_PUBLIC_KEY: &[u8; 33] = &[2; 33];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokensConfig {
    pub expected_withdraw_bond_sats: u64,
    pub expected_withdraw_relative_block_locktime: u64,
    pub transaction_validity_duration_seconds: u64,
}

pub struct TokenService {
    token_output_service: Arc<dyn TokenOutputService>,
    signer: Arc<dyn Signer>,
    operator_pool: Arc<OperatorPool>,
    network: Network,
    split_secret_threshold: u32,
    tokens_config: TokensConfig,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
}

impl TokenService {
    pub fn new(
        token_output_service: Arc<dyn TokenOutputService>,
        signer: Arc<dyn Signer>,
        operator_pool: Arc<OperatorPool>,
        network: Network,
        split_secret_threshold: u32,
        tokens_config: TokensConfig,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        Self {
            token_output_service,
            signer,
            operator_pool,
            network,
            split_secret_threshold,
            tokens_config,
            transfer_observer,
        }
    }

    /// Refreshes token outputs from the operator.
    pub async fn refresh_tokens_outputs(&self) -> Result<(), ServiceError> {
        self.token_output_service.refresh_tokens_outputs().await?;
        Ok(())
    }

    /// Returns the metadata for the given token identifiers.
    ///
    /// For token identifiers that are not found in the local cache, the metadata will be queried from the SE.
    pub async fn get_tokens_metadata(
        &self,
        token_identifiers: &[&str],
        issuer_public_keys: &[PublicKey],
    ) -> Result<Vec<TokenMetadata>, ServiceError> {
        // Separate token identifiers into cached and uncached
        let mut cached_metadata = Vec::new();
        let mut uncached_identifiers = Vec::new();
        let mut uncached_issuer_public_keys = Vec::new();

        for token_id in token_identifiers {
            if let Ok(metadata) = self
                .token_output_service
                .get_token_metadata(GetTokenOutputsFilter::Identifier(token_id))
                .await
            {
                cached_metadata.push(metadata);
            } else {
                uncached_identifiers.push(*token_id);
            }
        }

        for issuer_pk in issuer_public_keys {
            if let Ok(metadata) = self
                .token_output_service
                .get_token_metadata(GetTokenOutputsFilter::IssuerPublicKey(issuer_pk))
                .await
            {
                cached_metadata.push(metadata);
            } else {
                uncached_issuer_public_keys.push(*issuer_pk);
            }
        }

        // Query metadata for uncached tokens
        let mut queried_metadata = Vec::new();
        if !uncached_identifiers.is_empty() || !uncached_issuer_public_keys.is_empty() {
            queried_metadata = self
                .query_tokens_metadata(&uncached_identifiers, &uncached_issuer_public_keys)
                .await?;
        }

        // Combine cached and queried metadata
        let mut all_metadata = cached_metadata;
        all_metadata.extend(queried_metadata);

        Ok(all_metadata)
    }

    async fn query_tokens_metadata(
        &self,
        token_identifiers: &[&str],
        issuer_public_keys: &[PublicKey],
    ) -> Result<Vec<TokenMetadata>, ServiceError> {
        let token_identifiers = token_identifiers
            .iter()
            .map(|id| {
                bech32m_decode_token_id(id, Some(self.network))
                    .map_err(|_| ServiceError::Generic("Invalid token id".to_string()))
            })
            .collect::<Result<Vec<Vec<u8>>, _>>()?;
        let issuer_public_keys = issuer_public_keys
            .iter()
            .map(|k| k.serialize().to_vec())
            .collect::<Vec<_>>();
        let metadata = self
            .query_tokens_metadata_inner(token_identifiers, issuer_public_keys)
            .await?;
        let metadata = metadata
            .into_iter()
            .map(|m| (m, self.network).try_into())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(metadata)
    }

    async fn query_tokens_metadata_inner(
        &self,
        token_identifiers: Vec<Vec<u8>>,
        issuer_public_keys: Vec<Vec<u8>>,
    ) -> Result<Vec<rpc::spark_token::TokenMetadata>, ServiceError> {
        let metadata = self
            .operator_pool
            .get_coordinator()
            .client
            .query_token_metadata(QueryTokenMetadataRequest {
                token_identifiers,
                issuer_public_keys,
            })
            .await?
            .token_metadata;
        Ok(metadata)
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
            None => vec![
                self.signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
            ],
        };

        let order: crate::operator::rpc::spark::Order = paging.order.into();
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .query_token_transactions(QueryTokenTransactionsRequest {
                query_type: Some(QueryType::ByFilters(QueryTokenTransactionsByFilters {
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
                    page_request: None,
                })),
                order: order.into(),
                limit: paging.limit as i64,
                offset: paging.offset as i64,
                ..Default::default()
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
    ) -> Result<PagingResult<TokenTransaction>, ServiceError> {
        let transactions = match paging {
            Some(paging) => self.query_token_transactions_inner(filter, paging).await?,
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

    /// Queries token transactions by their hashes.
    ///
    /// This method uses the `QueryType::ByTxHash` variant which is limited to 100 hashes
    /// per request (enforced by proto). Returns an empty vector if input is empty.
    pub async fn query_token_transactions_by_hashes(
        &self,
        token_transaction_hashes: Vec<String>,
    ) -> Result<Vec<TokenTransaction>, ServiceError> {
        if token_transaction_hashes.is_empty() {
            return Ok(Vec::new());
        }

        let hashes = token_transaction_hashes
            .iter()
            .map(|h| {
                hex::decode(h).map_err(|_| {
                    ServiceError::Generic("Invalid token transaction hash".to_string())
                })
            })
            .collect::<Result<Vec<Vec<u8>>, _>>()?;

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .query_token_transactions(QueryTokenTransactionsRequest {
                query_type: Some(QueryType::ByTxHash(QueryTokenTransactionsByTxHash {
                    token_transaction_hashes: hashes,
                })),
                ..Default::default()
            })
            .await?;

        response
            .token_transactions_with_status
            .into_iter()
            .map(|t| (t, self.network).try_into())
            .collect::<Result<Vec<TokenTransaction>, _>>()
    }

    pub async fn get_issuer_token_metadata(&self) -> Result<TokenMetadata, ServiceError> {
        let identity_public_key = self.signer.get_identity_public_key().await?;
        let tokens_metadata = self
            .get_tokens_metadata(&[], &[identity_public_key])
            .await?;
        Ok(tokens_metadata
            .first()
            .ok_or(ServiceError::Generic("No issuer token found".to_string()))?
            .clone())
    }

    pub async fn create_issuer_token(
        &self,
        name: &str,
        ticker: &str,
        decimals: u32,
        is_freezable: bool,
        max_supply: u128,
    ) -> Result<TokenTransaction, ServiceError> {
        // Check if issuer token already exists and return a clear error
        if self.get_issuer_token_metadata().await.is_ok() {
            return Err(ServiceError::Generic(
                "Issuer token already exists".to_string(),
            ));
        }

        validate_create_token_params(name, ticker, decimals)?;

        let partial_tx = self
            .build_create_token_transaction(name, ticker, decimals, is_freezable, max_supply)
            .await?;
        let final_tx = self.start_transaction(partial_tx).await?;

        self.commit_transaction(final_tx.clone()).await?;

        (final_tx, self.network).try_into()
    }

    pub async fn mint_issuer_token(&self, amount: u128) -> Result<TokenTransaction, ServiceError> {
        let issuer_token_metadata = self.get_issuer_token_metadata().await?;

        let partial_tx = self
            .build_mint_token_transaction(&issuer_token_metadata.identifier, amount)
            .await?;
        let final_tx = self.start_transaction(partial_tx).await?;

        self.commit_transaction(final_tx.clone()).await?;

        (final_tx, self.network).try_into()
    }

    pub async fn burn_issuer_token(
        &self,
        amount: u128,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenTransaction, ServiceError> {
        let burn_public_key =
            PublicKey::from_slice(BURN_PUBLIC_KEY).map_err(|_| ServiceError::InvalidPublicKey)?;
        let burn_spark_address = SparkAddress::new(burn_public_key, self.network, None);
        let receiver_outputs = vec![TransferTokenOutput {
            token_id: self.get_issuer_token_metadata().await?.identifier,
            amount,
            receiver_address: burn_spark_address,
            spark_invoice: None,
        }];
        self.transfer_tokens(receiver_outputs, preferred_outputs, selection_strategy)
            .await
    }

    pub async fn freeze_issuer_token(
        &self,
        spark_address: &SparkAddress,
        should_unfreeze: bool,
    ) -> Result<FreezeIssuerTokenResponse, ServiceError> {
        let owner_public_key = spark_address.identity_public_key.serialize().to_vec();
        let token_metadata = self.get_issuer_token_metadata().await?;
        if !token_metadata.is_freezable {
            return Err(ServiceError::Generic(
                "Issuer token is not freezable".to_string(),
            ));
        }

        let token_identifier =
            bech32m_decode_token_id(&token_metadata.identifier, Some(self.network))?;
        let issuer_provided_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                ServiceError::Generic("issuer_provided_timestamp is before UNIX_EPOCH".to_string())
            })?
            .as_millis() as u64;

        let mut requests = Vec::new();
        for operator in self.operator_pool.get_all_operators() {
            let freeze_tokens_payload = rpc::spark_token::FreezeIssuerTokenPayload {
                version: 1,
                owner_public_key: owner_public_key.clone(),
                token_identifier: Some(token_identifier.clone()),
                should_unfreeze,
                issuer_provided_timestamp,
                operator_identity_public_key: operator.identity_public_key.serialize().to_vec(),
                ..Default::default()
            };
            let payload_hash = hash_freeze_tokens_payload(&freeze_tokens_payload)?;
            let issuer_signature = self
                .signer
                .sign_hash_schnorr_with_identity_key(&payload_hash)
                .await?
                .serialize()
                .to_vec();

            let request =
                operator
                    .client
                    .freeze_tokens(rpc::spark_token::FreezeIssuerTokenRequest {
                        freeze_tokens_payload: Some(freeze_tokens_payload),
                        issuer_signature,
                    });
            requests.push(request);
        }
        let responses = futures::future::try_join_all(requests).await?;
        info!("Freeze tokens responses: {:?}", responses);

        responses
            .first()
            .ok_or(ServiceError::Generic(
                "No response from freeze tokens request".to_string(),
            ))?
            .clone()
            .try_into()
    }

    pub async fn transfer_tokens(
        &self,
        receiver_outputs: Vec<TransferTokenOutput>,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenTransaction, ServiceError> {
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

        let mut attempt = 0;
        let mut preempted_attempt = 0;
        let (token_transaction, reservation) = loop {
            if attempt >= MAX_TRANSFER_TOKEN_TOO_MANY_OUTPUTS_RETRY_ATTEMPTS {
                return Err(ServiceError::NeededTooManyOutputs);
            }

            let reservation = self
                .token_output_service
                .reserve_token_outputs(
                    &token_id,
                    ReservationTarget::MinTotalValue(total_amount),
                    ReservationPurpose::Payment,
                    preferred_outputs.clone(),
                    selection_strategy,
                )
                .await?;

            let result = with_reserved_token_outputs(
                self.token_output_service.as_ref(),
                self.transfer_tokens_inner(
                    &token_id,
                    reservation.token_outputs.outputs.clone(),
                    receiver_outputs.clone(),
                ),
                &reservation,
            )
            .await;

            match result {
                Ok(token_transaction) => break (token_transaction, reservation),
                Err(ServiceError::NeededTooManyOutputs)
                    if attempt < MAX_TRANSFER_TOKEN_TOO_MANY_OUTPUTS_RETRY_ATTEMPTS - 1 =>
                {
                    self.optimize_token_outputs(Some(&token_id), 2).await?;
                    attempt += 1;
                    continue;
                }
                Err(ServiceError::ServiceConnectionError(ref e))
                    if is_transaction_preempted_error(e)
                        && preempted_attempt < MAX_TOKEN_PREEMPTED_RETRY_ATTEMPTS - 1 =>
                {
                    preempted_attempt += 1;
                    warn!(
                        "Token transfer preempted (attempt {preempted_attempt}/{}), refreshing token outputs and retrying",
                        MAX_TOKEN_PREEMPTED_RETRY_ATTEMPTS
                    );
                    self.refresh_tokens_outputs().await?;
                    continue;
                }
                Err(e) => return Err(e),
            }
        };

        let identity_public_key = self.signer.get_identity_public_key().await?;
        let outputs = token_transaction
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, output)| output.owner_public_key == identity_public_key)
            .map(|(vout, output)| TokenOutputWithPrevOut {
                output: output.clone(),
                prev_tx_hash: token_transaction.hash.clone(),
                prev_tx_vout: vout as u32,
            })
            .collect::<Vec<_>>();
        self.token_output_service
            .insert_token_outputs(&TokenOutputs {
                metadata: reservation.token_outputs.metadata,
                outputs,
            })
            .await?;

        Ok(token_transaction)
    }

    async fn transfer_tokens_inner(
        &self,
        token_id: &str,
        inputs: Vec<TokenOutputWithPrevOut>,
        receiver_outputs: Vec<TransferTokenOutput>,
    ) -> Result<TokenTransaction, ServiceError> {
        if inputs.len() > MAX_TOKEN_TX_INPUTS {
            return Err(ServiceError::NeededTooManyOutputs);
        }

        let partial_tx = self
            .build_transfer_token_transaction(inputs.clone(), receiver_outputs.clone())
            .await?;
        let final_tx = self.start_transaction(partial_tx).await?;
        let txid = hex::encode(final_tx.compute_hash(false)?);

        if let Some(observer) = &self.transfer_observer {
            observer
                .before_send_token(
                    &txid,
                    token_id,
                    receiver_outputs
                        .into_iter()
                        .map(|o| {
                            Ok(ReceiverTokenOutput {
                                pay_request: o
                                    .spark_invoice
                                    .or(o.receiver_address.to_address_string().ok())
                                    .ok_or(ServiceError::Generic(
                                        "No pay request available".to_string(),
                                    ))?,
                                amount: o.amount,
                            })
                        })
                        .collect::<Result<Vec<ReceiverTokenOutput>, ServiceError>>()?,
                )
                .await?;
        }

        self.commit_transaction(final_tx.clone()).await?;

        (final_tx, self.network).try_into()
    }

    /// Optimizes token outputs by consolidating them when there are more than the configured threshold.
    /// Processes one token at a time. Token identifier can be provided, otherwise one is automatically selected.
    /// Only optimizes if the number of outputs is greater than the provided `min_outputs_threshold` (min 2).
    pub async fn optimize_token_outputs(
        &self,
        token_identifier: Option<&str>,
        min_outputs_threshold: u32,
    ) -> Result<(), ServiceError> {
        if min_outputs_threshold <= 1 {
            return Err(ServiceError::ValidationError(
                "min_outputs_threshold must be greater than 1".to_string(),
            ));
        }

        info!(
            "Optimizing token outputs starting (optional token identifier: {:?})",
            token_identifier
        );

        let mut outputs = self.token_output_service.list_tokens_outputs().await?;

        if let Some(token_identifier) = token_identifier {
            outputs.retain(|o| o.metadata.identifier == token_identifier);
        }

        let mut did_optimize = false;

        for output in outputs {
            if output.available.len() <= min_outputs_threshold as usize {
                continue;
            }

            did_optimize = true;

            let reservation = self
                .token_output_service
                .reserve_token_outputs(
                    &output.metadata.identifier,
                    ReservationTarget::MaxOutputCount(MAX_TOKEN_TX_INPUTS),
                    ReservationPurpose::Swap,
                    None,
                    Some(SelectionStrategy::SmallestFirst),
                )
                .await?;

            info!(
                "Optimizing token {} - currently has {} available outputs",
                output.metadata.identifier,
                output.available.len(),
            );

            let amount = reservation
                .token_outputs
                .outputs
                .iter()
                .map(|o| o.output.token_amount)
                .sum::<u128>();

            let token_transaction = with_reserved_token_outputs(
                self.token_output_service.as_ref(),
                self.transfer_tokens_inner(
                    &output.metadata.identifier,
                    reservation.token_outputs.outputs.clone(),
                    vec![TransferTokenOutput {
                        token_id: output.metadata.identifier.clone(),
                        amount,
                        receiver_address: SparkAddress::new(
                            self.signer.get_identity_public_key().await?,
                            self.network,
                            None,
                        ),
                        spark_invoice: None,
                    }],
                ),
                &reservation,
            )
            .await?;

            let outputs = token_transaction
                .outputs
                .iter()
                .enumerate()
                .map(|(vout, output)| TokenOutputWithPrevOut {
                    output: output.clone(),
                    prev_tx_hash: token_transaction.hash.clone(),
                    prev_tx_vout: vout as u32,
                })
                .collect::<Vec<_>>();
            self.token_output_service
                .insert_token_outputs(&TokenOutputs {
                    metadata: reservation.token_outputs.metadata,
                    outputs,
                })
                .await?;
        }

        if !did_optimize {
            info!("No token outputs to optimize");
        }

        Ok(())
    }

    async fn build_create_token_transaction(
        &self,
        name: &str,
        ticker: &str,
        decimals: u32,
        is_freezable: bool,
        max_supply: u128,
    ) -> Result<rpc::spark_token::TokenTransaction, ServiceError> {
        let token_inputs = rpc::spark_token::token_transaction::TokenInputs::CreateInput(
            rpc::spark_token::TokenCreateInput {
                issuer_public_key: self
                    .signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
                token_name: name.to_string(),
                token_ticker: ticker.to_string(),
                decimals,
                is_freezable,
                max_supply: max_supply.to_be_bytes().to_vec(),
                ..Default::default()
            },
        );

        self.build_token_transaction(token_inputs, vec![], vec![])
    }

    async fn build_mint_token_transaction(
        &self,
        token_id: &str,
        amount: u128,
    ) -> Result<rpc::spark_token::TokenTransaction, ServiceError> {
        let identity_public_key = self
            .signer
            .get_identity_public_key()
            .await?
            .serialize()
            .to_vec();
        let token_identifier = bech32m_decode_token_id(token_id, Some(self.network))
            .map_err(|_| ServiceError::Generic("Invalid token id".to_string()))?;

        let token_inputs = rpc::spark_token::token_transaction::TokenInputs::MintInput(
            rpc::spark_token::TokenMintInput {
                issuer_public_key: identity_public_key.clone(),
                token_identifier: Some(token_identifier.clone()),
            },
        );
        let token_outputs = vec![rpc::spark_token::TokenOutput {
            owner_public_key: identity_public_key,
            token_identifier: Some(token_identifier),
            token_amount: amount.to_be_bytes().to_vec(),
            ..Default::default()
        }];

        self.build_token_transaction(token_inputs, token_outputs, vec![])
    }

    async fn build_transfer_token_transaction(
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
                    self.signer.get_identity_public_key().await?,
                    self.network,
                    None,
                ),
                spark_invoice: None,
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

        // Spark invoices this tx fulfills
        let invoice_attachments = receiver_outputs
            .into_iter()
            .filter_map(|o| {
                o.spark_invoice
                    .map(|i| rpc::spark_token::InvoiceAttachment { spark_invoice: i })
            })
            .collect::<Vec<_>>();

        // Build transaction
        self.build_token_transaction(inputs, token_outputs, invoice_attachments)
    }

    fn build_token_transaction(
        &self,
        token_inputs: rpc::spark_token::token_transaction::TokenInputs,
        token_outputs: Vec<rpc::spark_token::TokenOutput>,
        invoice_attachments: Vec<rpc::spark_token::InvoiceAttachment>,
    ) -> Result<rpc::spark_token::TokenTransaction, ServiceError> {
        Ok(rpc::spark_token::TokenTransaction {
            version: 2,
            token_outputs,
            spark_operator_identity_public_keys: self.get_operator_identity_public_keys()?,
            expiry_time: None,
            network: self.network.to_proto_network().into(),
            client_created_timestamp: Some(
                web_time_to_prost_timestamp(&SystemTime::now()).map_err(|_| {
                    ServiceError::Generic(
                        "client_created_timestamp is before UNIX_EPOCH".to_string(),
                    )
                })?,
            ),
            token_inputs: Some(token_inputs),
            invoice_attachments,
        })
    }

    fn get_operator_identity_public_keys(&self) -> Result<Vec<Vec<u8>>, ServiceError> {
        let operators = self.operator_pool.get_all_operators();
        let keys = operators
            .map(|o| o.identity_public_key.serialize().to_vec())
            .collect();
        Ok(keys)
    }

    async fn start_transaction(
        &self,
        partial_tx: rpc::spark_token::TokenTransaction,
    ) -> Result<rpc::spark_token::TokenTransaction, ServiceError> {
        let partial_tx_hash = partial_tx.compute_hash(true)?;

        // Sign inputs
        let mut owner_signatures: Vec<SignatureWithIndex> = Vec::new();
        let signature = self
            .signer
            .sign_hash_schnorr_with_identity_key(&partial_tx_hash)
            .await?
            .serialize()
            .to_vec();
        match partial_tx.token_inputs.as_ref() {
            Some(
                rpc::spark_token::token_transaction::TokenInputs::CreateInput(_)
                | rpc::spark_token::token_transaction::TokenInputs::MintInput(_),
            ) => {
                owner_signatures.push(SignatureWithIndex {
                    signature,
                    input_index: 0,
                });
            }
            Some(rpc::spark_token::token_transaction::TokenInputs::TransferInput(input)) => {
                // One signature per input
                for i in 0..input.outputs_to_spend.len() {
                    owner_signatures.push(SignatureWithIndex {
                        signature: signature.clone(),
                        input_index: i as u32,
                    });
                }
            }
            _ => {
                return Err(ServiceError::Generic(
                    "Token inputs are required".to_string(),
                ));
            }
        }

        let start_response = self
            .operator_pool
            .get_coordinator()
            .client
            .start_transaction(StartTransactionRequest {
                identity_public_key: self
                    .signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
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

        Ok(final_tx)
    }

    async fn commit_transaction(
        &self,
        final_tx: rpc::spark_token::TokenTransaction,
    ) -> Result<(), ServiceError> {
        let final_tx_hash = final_tx.compute_hash(false)?;

        let per_operator_signatures = self
            .create_per_operator_signatures(&final_tx, &final_tx_hash)
            .await?;

        self.operator_pool
            .get_coordinator()
            .client
            .commit_transaction(CommitTransactionRequest {
                final_token_transaction: Some(final_tx.clone()),
                final_token_transaction_hash: final_tx_hash.clone(),
                input_ttxo_signatures_per_operator: per_operator_signatures,
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
            })
            .await?;

        Ok(())
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
                rpc::spark_token::token_transaction::TokenInputs::CreateInput(partial_tx_input),
                rpc::spark_token::token_transaction::TokenInputs::CreateInput(final_tx_input),
            ) => {
                if partial_tx_input.issuer_public_key != final_tx_input.issuer_public_key {
                    return Err(ServiceError::Generic(
                        "Issuer public key mismatch in create input".to_string(),
                    ));
                }
            }
            (
                rpc::spark_token::token_transaction::TokenInputs::MintInput(partial_tx_input),
                rpc::spark_token::token_transaction::TokenInputs::MintInput(final_tx_input),
            ) => {
                if partial_tx_input.issuer_public_key != final_tx_input.issuer_public_key {
                    return Err(ServiceError::Generic(
                        "Issuer public key mismatch in mint input".to_string(),
                    ));
                }

                if partial_tx_input.token_identifier != final_tx_input.token_identifier {
                    return Err(ServiceError::Generic(
                        "Token identifier mismatch in mint input".to_string(),
                    ));
                }
            }
            (
                rpc::spark_token::token_transaction::TokenInputs::TransferInput(partial_tx_input),
                rpc::spark_token::token_transaction::TokenInputs::TransferInput(final_tx_input),
            ) => {
                if partial_tx_input.outputs_to_spend.len() != final_tx_input.outputs_to_spend.len()
                {
                    return Err(ServiceError::Generic(
                        "Outputs to spend mismatch in transfer input".to_string(),
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
                            "Prev token transaction hash mismatch in transfer input".to_string(),
                        ));
                    }

                    if partial_output.prev_token_transaction_vout
                        != final_output.prev_token_transaction_vout
                    {
                        return Err(ServiceError::Generic(
                            "Prev token transaction vout mismatch in transfer input".to_string(),
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

    async fn create_per_operator_signatures(
        &self,
        tx: &rpc::spark_token::TokenTransaction,
        tx_hash: &[u8],
    ) -> Result<Vec<rpc::spark_token::InputTtxoSignaturesPerOperator>, ServiceError> {
        let mut per_operator_signatures = Vec::new();

        for operator in self.operator_pool.get_all_operators() {
            let operator_identity_public_key_bytes =
                operator.identity_public_key.serialize().to_vec();

            let tx_hash_hash = sha256::Hash::hash(tx_hash).to_byte_array().to_vec();
            let operator_pubkey_hash = sha256::Hash::hash(&operator_identity_public_key_bytes)
                .to_byte_array()
                .to_vec();
            let final_hash = sha256::Hash::hash(&[tx_hash_hash, operator_pubkey_hash].concat())
                .to_byte_array()
                .to_vec();

            let mut signatures = Vec::new();
            let signature = self
                .signer
                .sign_hash_schnorr_with_identity_key(&final_hash)
                .await?
                .serialize()
                .to_vec();

            match tx.token_inputs.as_ref() {
                Some(
                    rpc::spark_token::token_transaction::TokenInputs::CreateInput(_)
                    | rpc::spark_token::token_transaction::TokenInputs::MintInput(_),
                ) => {
                    signatures.push(rpc::spark_token::SignatureWithIndex {
                        signature,
                        input_index: 0,
                    });
                }
                Some(rpc::spark_token::token_transaction::TokenInputs::TransferInput(input)) => {
                    // One signature per input
                    for i in 0..input.outputs_to_spend.len() {
                        signatures.push(rpc::spark_token::SignatureWithIndex {
                            signature: signature.clone(),
                            input_index: i as u32,
                        });
                    }
                }
                _ => {
                    return Err(ServiceError::Generic(
                        "Token inputs are required".to_string(),
                    ));
                }
            }

            per_operator_signatures.push(rpc::spark_token::InputTtxoSignaturesPerOperator {
                ttxo_signatures: signatures,
                operator_identity_public_key: operator_identity_public_key_bytes,
            });
        }

        Ok(per_operator_signatures)
    }
}

pub fn bech32m_encode_token_id(
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
pub fn bech32m_decode_token_id(
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

pub trait HashableTokenTransaction {
    fn compute_hash(&self, partial: bool) -> Result<Vec<u8>, ServiceError>;
}

impl HashableTokenTransaction for rpc::spark_token::TokenTransaction {
    fn compute_hash(&self, partial: bool) -> Result<Vec<u8>, ServiceError> {
        match self.version {
            1 => compute_hash_v1(self, partial),
            2 => compute_hash_v2(self, partial),
            _ => Err(ServiceError::Generic(
                "Unsupported token transaction version".to_string(),
            )),
        }
    }
}

/// Computes the common hash components shared between V1 and V2 token transactions.
/// Returns a vector of hashes that can be extended with version-specific hashes.
fn compute_common_hash_components(
    transaction: &rpc::spark_token::TokenTransaction,
    partial: bool,
) -> Result<Vec<Vec<u8>>, ServiceError> {
    let mut all_hashes = Vec::new();

    let version_hash = sha256::Hash::hash(&transaction.version.to_be_bytes())
        .to_byte_array()
        .to_vec();
    all_hashes.push(version_hash);

    match transaction.token_inputs.as_ref() {
        Some(rpc::spark_token::token_transaction::TokenInputs::CreateInput(input)) => {
            // Hash transaction type
            let tx_type = (rpc::spark_token::TokenTransactionType::Create as u32).to_be_bytes();
            let tx_type_hash = sha256::Hash::hash(&tx_type).to_byte_array().to_vec();
            all_hashes.push(tx_type_hash);

            // Hash issuer public key
            let issuer_pubkey_hash = sha256::Hash::hash(&input.issuer_public_key)
                .to_byte_array()
                .to_vec();
            all_hashes.push(issuer_pubkey_hash);

            // Hash token name
            let token_name_hash = sha256::Hash::hash(input.token_name.as_bytes())
                .to_byte_array()
                .to_vec();
            all_hashes.push(token_name_hash);

            // Hash token ticker
            let token_ticker_hash = sha256::Hash::hash(input.token_ticker.as_bytes())
                .to_byte_array()
                .to_vec();
            all_hashes.push(token_ticker_hash);

            // Hash decimals
            let decimals_hash = sha256::Hash::hash(&input.decimals.to_be_bytes())
                .to_byte_array()
                .to_vec();
            all_hashes.push(decimals_hash);

            // Hash max supply
            let max_supply_hash = sha256::Hash::hash(&input.max_supply)
                .to_byte_array()
                .to_vec();
            all_hashes.push(max_supply_hash);

            // Hash is freezable
            let is_freezable_byte = if input.is_freezable { 1u8 } else { 0u8 };
            let is_freezable_hash = sha256::Hash::hash(&[is_freezable_byte])
                .to_byte_array()
                .to_vec();
            all_hashes.push(is_freezable_hash);

            // Hash creation entity public key (only for final hash)
            let creation_entity_public_key = if !partial
                && let Some(creation_entity_public_key) = input.creation_entity_public_key.as_ref()
            {
                creation_entity_public_key
            } else {
                &Vec::new()
            };

            let creation_entity_public_key_hash = sha256::Hash::hash(creation_entity_public_key)
                .to_byte_array()
                .to_vec();
            all_hashes.push(creation_entity_public_key_hash);
        }
        Some(rpc::spark_token::token_transaction::TokenInputs::MintInput(input)) => {
            // Hash transaction type
            let tx_type = (rpc::spark_token::TokenTransactionType::Mint as u32).to_be_bytes();
            let tx_type_hash = sha256::Hash::hash(&tx_type).to_byte_array().to_vec();
            all_hashes.push(tx_type_hash);

            // Hash issuer public key
            let issuer_pubkey_hash = sha256::Hash::hash(&input.issuer_public_key)
                .to_byte_array()
                .to_vec();
            all_hashes.push(issuer_pubkey_hash);

            // Hash token identifier
            let zeroed_token_identifier = vec![0; 32];
            let token_identifier = input
                .token_identifier
                .as_ref()
                .unwrap_or(&zeroed_token_identifier);
            let token_identifier_hash = sha256::Hash::hash(token_identifier)
                .to_byte_array()
                .to_vec();
            all_hashes.push(token_identifier_hash);
        }
        Some(rpc::spark_token::token_transaction::TokenInputs::TransferInput(input)) => {
            // Hash transaction type
            let tx_type = (rpc::spark_token::TokenTransactionType::Transfer as u32).to_be_bytes();
            let tx_type_hash = sha256::Hash::hash(&tx_type).to_byte_array().to_vec();
            all_hashes.push(tx_type_hash);

            // Hash outputs to spend length
            let inputs = &input.outputs_to_spend;
            let inputs_len = inputs.len() as u32;
            let inputs_len_hash = sha256::Hash::hash(&inputs_len.to_be_bytes())
                .to_byte_array()
                .to_vec();
            all_hashes.push(inputs_len_hash);

            // Hash outputs to spend
            for input in inputs {
                let mut engine = sha256::Hash::engine();
                engine.input(&input.prev_token_transaction_hash);
                engine.input(&input.prev_token_transaction_vout.to_be_bytes());
                all_hashes.push(sha256::Hash::from_engine(engine).to_byte_array().to_vec());
            }
        }
        _ => {
            return Err(ServiceError::Generic(
                "Token inputs are required".to_string(),
            ));
        }
    }

    let outputs_len = transaction.token_outputs.len() as u32;
    let outputs_len_hash = sha256::Hash::hash(&outputs_len.to_be_bytes())
        .to_byte_array()
        .to_vec();
    all_hashes.push(outputs_len_hash);

    for output in &transaction.token_outputs {
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

            let withdraw_relative_block_locktime =
                output
                    .withdraw_relative_block_locktime
                    .ok_or(ServiceError::Generic(
                        "Withdraw relative block locktime is required".to_string(),
                    ))?;
            engine.input(&withdraw_relative_block_locktime.to_be_bytes());
        }

        let zeroed_pubkey = vec![0; 33];
        let token_pubkey = output.token_public_key.as_ref().unwrap_or(&zeroed_pubkey);
        engine.input(token_pubkey);

        let token_identifier = output
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
    let mut operator_public_keys = transaction.spark_operator_identity_public_keys.clone();
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

    let network_hash = sha256::Hash::hash(&transaction.network.to_be_bytes())
        .to_byte_array()
        .to_vec();
    all_hashes.push(network_hash);

    let unix_timestamp = transaction
        .client_created_timestamp
        .ok_or(ServiceError::Generic(
            "Client created timestamp is required".to_string(),
        ))?;
    let unix_timestamp_ms =
        unix_timestamp.seconds as u64 * 1000 + unix_timestamp.nanos as u64 / 1_000_000;
    let client_created_timestamp_hash = sha256::Hash::hash(&unix_timestamp_ms.to_be_bytes())
        .to_byte_array()
        .to_vec();
    all_hashes.push(client_created_timestamp_hash);

    if !partial {
        let expiry_time = transaction
            .expiry_time
            .map(|t| t.seconds as u64)
            .unwrap_or(0);
        let expiry_time_hash = sha256::Hash::hash(&expiry_time.to_be_bytes())
            .to_byte_array()
            .to_vec();
        all_hashes.push(expiry_time_hash);
    }

    Ok(all_hashes)
}

fn compute_hash_v1(
    transaction: &rpc::spark_token::TokenTransaction,
    partial: bool,
) -> Result<Vec<u8>, ServiceError> {
    let all_hashes = compute_common_hash_components(transaction, partial)?;

    let final_hash = sha256::Hash::hash(&all_hashes.concat())
        .to_byte_array()
        .to_vec();

    Ok(final_hash)
}

fn compute_hash_v2(
    transaction: &rpc::spark_token::TokenTransaction,
    partial: bool,
) -> Result<Vec<u8>, ServiceError> {
    let mut all_hashes = compute_common_hash_components(transaction, partial)?;

    // V2 adds invoice attachment hashing
    let attachments_len = transaction.invoice_attachments.len() as u32;
    let attachments_len_hash = sha256::Hash::hash(&attachments_len.to_be_bytes())
        .to_byte_array()
        .to_vec();
    all_hashes.push(attachments_len_hash);

    // Collect and sort invoice attachments by their ID
    let mut sorted_invoices: Vec<(Vec<u8>, String)> = Vec::new();

    for (i, attachment) in transaction.invoice_attachments.iter().enumerate() {
        let invoice = &attachment.spark_invoice;

        // Parse the SparkAddress from the invoice string
        let address = invoice
            .parse::<SparkAddress>()
            .map_err(|e| ServiceError::Generic(format!("Invalid invoice at index {i}: {e}")))?;

        // Extract the invoice ID
        let invoice_fields = address
            .spark_invoice_fields
            .as_ref()
            .ok_or_else(|| ServiceError::Generic(format!("Missing invoice fields at index {i}")))?;

        let id_bytes = invoice_fields.id.as_bytes().to_vec();

        if id_bytes.len() != 16 {
            return Err(ServiceError::Generic(format!(
                "Invalid invoice ID length at index {i}: expected 16 bytes, got {}",
                id_bytes.len()
            )));
        }

        sorted_invoices.push((id_bytes, invoice.clone()));
    }

    // Sort by ID bytes lexicographically
    sorted_invoices.sort_by(|a, b| {
        for (a_byte, b_byte) in a.0.iter().zip(b.0.iter()) {
            if a_byte != b_byte {
                return a_byte.cmp(b_byte);
            }
        }
        a.0.len().cmp(&b.0.len())
    });

    // Hash each sorted invoice's raw string (UTF-8)
    for (_id, invoice) in sorted_invoices {
        all_hashes.push(
            sha256::Hash::hash(invoice.as_bytes())
                .to_byte_array()
                .to_vec(),
        );
    }

    let final_hash = sha256::Hash::hash(&all_hashes.concat())
        .to_byte_array()
        .to_vec();

    Ok(final_hash)
}

fn validate_create_token_params(
    name: &str,
    ticker: &str,
    decimals: u32,
) -> Result<(), ServiceError> {
    if !unicode_normalization::is_nfc(name) {
        return Err(ServiceError::Generic(
            "Token name must be NFC-normalised UTF-8".to_string(),
        ));
    }
    if !unicode_normalization::is_nfc(ticker) {
        return Err(ServiceError::Generic(
            "Token ticker must be NFC-normalised UTF-8".to_string(),
        ));
    }
    if name.len() < 3 || name.len() > 20 {
        return Err(ServiceError::Generic(
            "Token name must be between 3 and 20 bytes".to_string(),
        ));
    }
    if ticker.len() < 3 || ticker.len() > 6 {
        return Err(ServiceError::Generic(
            "Token ticker must be between 3 and 6 bytes".to_string(),
        ));
    }
    if decimals > 255 {
        return Err(ServiceError::Generic(
            "Decimals must be an between 0 and 255".to_string(),
        ));
    }

    Ok(())
}

fn hash_freeze_tokens_payload(
    payload: &rpc::spark_token::FreezeIssuerTokenPayload,
) -> Result<Vec<u8>, ServiceError> {
    let mut all_hashes = Vec::new();
    let empty_bytes = vec![];

    let version_hash = sha256::Hash::hash(&payload.version.to_be_bytes())
        .to_byte_array()
        .to_vec();
    all_hashes.push(version_hash);

    let owner_public_key_hash = sha256::Hash::hash(&payload.owner_public_key)
        .to_byte_array()
        .to_vec();
    all_hashes.push(owner_public_key_hash);

    let token_identifier = payload.token_identifier.as_ref().unwrap_or(&empty_bytes);
    let token_identifier_hash = sha256::Hash::hash(token_identifier)
        .to_byte_array()
        .to_vec();
    all_hashes.push(token_identifier_hash);

    let should_unfreeze = if payload.should_unfreeze { 1u8 } else { 0u8 };
    let should_unfreeze_hash = sha256::Hash::hash(&[should_unfreeze])
        .to_byte_array()
        .to_vec();
    all_hashes.push(should_unfreeze_hash);

    let issuer_provided_timestamp_hash =
        sha256::Hash::hash(&payload.issuer_provided_timestamp.to_le_bytes())
            .to_byte_array()
            .to_vec();
    all_hashes.push(issuer_provided_timestamp_hash);

    let operator_identity_public_key_hash =
        sha256::Hash::hash(&payload.operator_identity_public_key)
            .to_byte_array()
            .to_vec();
    all_hashes.push(operator_identity_public_key_hash);

    let final_hash = sha256::Hash::hash(&all_hashes.concat())
        .to_byte_array()
        .to_vec();

    Ok(final_hash)
}

#[cfg(test)]
mod tests {
    use macros::test_all;
    use prost_types::Timestamp;

    use crate::{
        Network,
        operator::rpc::{
            self,
            spark_token::{
                TokenOutput, TokenOutputToSpend, TokenTransferInput, token_transaction::TokenInputs,
            },
        },
        services::tokens::{HashableTokenTransaction, validate_create_token_params},
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn create_test_token_transaction(version: u32) -> rpc::spark_token::TokenTransaction {
        rpc::spark_token::TokenTransaction {
            version,
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
            invoice_attachments: vec![rpc::spark_token::InvoiceAttachment {
                spark_invoice: "sparkrt1pgss8cf4gru7ece2ryn8ym3vm3yz8leeend2589m7svq2mgv0xncfyx8zgvssqgjzqqe5p0mj9v8j69ygjsh67m8t2jjyqcgaqr35sx0qparn2k6s24kgnzh3v2mqapzryhgfy27ye9c58mlz2lggmenf8tae4323jgv7s2ldglsu990t8fugefeqk4rzstc98rly7yt0gmnq95dwk2".to_string(),
            }, rpc::spark_token::InvoiceAttachment {
                spark_invoice: "sparkrt1pgss8cf4gru7ece2ryn8ym3vm3yz8leeend2589m7svq2mgv0xncfyx8zg7qsqgjzqqe5p0arydhhu5utuc4zzm732h35fs2yzsc3gs6v8hzpgnaaax0kgcn7r7gq53lnxq0gqnuscptu60nvu02yyszq05p5syke4wzv7gn76gt3r30c90qt8u5nfec4vl60nrxphjgzqm4hgze4xrxejmu2vqlj8sxp4mzux2dlq7fpq9akl0tufcpqd25tcpljc407uexx26".to_string(),
            }],
        }
    }

    #[test_all]
    fn test_compute_token_transaction_hash_v1_non_partial() {
        let tx = create_test_token_transaction(1);

        let hash = tx.compute_hash(false).unwrap();
        // Value taken from JS implementation
        assert_eq!(
            hash,
            hex::decode("0b7b506a33722689744cdad140c8c02702a9ad779869637a5631281f6fbbe0eb")
                .unwrap()
        );
    }

    #[test_all]
    fn test_compute_token_transaction_hash_v1_partial() {
        let tx = create_test_token_transaction(1);

        let hash = tx.compute_hash(true).unwrap();
        // Value taken from JS implementation
        assert_eq!(
            hash,
            hex::decode("2fb877692e90822551c7cfd522139a4119f2395c6c96677e41f5a1c68c872af0")
                .unwrap()
        );
    }

    #[test_all]
    fn test_compute_token_transaction_hash_v2_non_partial() {
        let tx = create_test_token_transaction(2);

        let hash = tx.compute_hash(false).unwrap();
        // Value taken from JS implementation
        assert_eq!(
            hash,
            hex::decode("34d11f87a2621b5598ee874d2965b6e6aa2610d368d435a790343363cd6f292d")
                .unwrap()
        );
    }

    #[test_all]
    fn test_compute_token_transaction_hash_v2_partial() {
        let tx = create_test_token_transaction(2);

        let hash = tx.compute_hash(true).unwrap();
        // Value taken from JS implementation
        assert_eq!(
            hash,
            hex::decode("cd2ad2481353728dc82c7d80565fb5e66e67a5d98deb338740786a052177ffbe")
                .unwrap()
        );
    }

    #[test_all]
    fn test_validate_create_token_params_valid() {
        // Valid parameters
        assert!(validate_create_token_params("Bitcoin", "BTC", 8).is_ok());
        assert!(validate_create_token_params("ABC", "ABC", 0).is_ok());
        assert!(validate_create_token_params("12345678901234567890", "ABCDEF", 255).is_ok());
    }

    #[test_all]
    fn test_validate_create_token_params_name_too_short() {
        let result = validate_create_token_params("AB", "BTC", 8);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Token name must be between 3 and 20 bytes")
        );
    }

    #[test_all]
    fn test_validate_create_token_params_name_too_long() {
        let result = validate_create_token_params("123456789012345678901", "BTC", 8);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Token name must be between 3 and 20 bytes")
        );
    }

    #[test_all]
    fn test_validate_create_token_params_ticker_too_short() {
        let result = validate_create_token_params("Bitcoin", "AB", 8);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Token ticker must be between 3 and 6 bytes")
        );
    }

    #[test_all]
    fn test_validate_create_token_params_ticker_too_long() {
        let result = validate_create_token_params("Bitcoin", "ABCDEFG", 8);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Token ticker must be between 3 and 6 bytes")
        );
    }

    #[test_all]
    fn test_validate_create_token_params_decimals_too_large() {
        let result = validate_create_token_params("Bitcoin", "BTC", 256);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Decimals must be an between 0 and 255")
        );
    }

    #[test_all]
    fn test_validate_create_token_params_name_not_nfc() {
        // Using a string that is not NFC normalized (combining characters)
        // "é" as e + combining acute accent (U+0065 U+0301) instead of composed form (U+00E9)
        let non_nfc_name = "Caf\u{0065}\u{0301}";
        let result = validate_create_token_params(non_nfc_name, "BTC", 8);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Token name must be NFC-normalised UTF-8")
        );
    }

    #[test_all]
    fn test_validate_create_token_params_ticker_not_nfc() {
        // Using a string that is not NFC normalized
        let non_nfc_ticker = "BT\u{0065}\u{0301}";
        let result = validate_create_token_params("Bitcoin", non_nfc_ticker, 8);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Token ticker must be NFC-normalised UTF-8")
        );
    }

    #[test_all]
    fn test_validate_create_token_params_edge_cases() {
        // Minimum valid values
        assert!(validate_create_token_params("ABC", "ABC", 0).is_ok());

        // Maximum valid values
        assert!(validate_create_token_params("12345678901234567890", "ABCDEF", 255).is_ok());

        // Exactly at boundaries
        assert!(validate_create_token_params("123", "ABC", 0).is_ok());
        assert!(validate_create_token_params("12345678901234567890", "ABC", 0).is_ok());
        assert!(validate_create_token_params("Bitcoin", "ABC", 0).is_ok());
        assert!(validate_create_token_params("Bitcoin", "ABCDEF", 0).is_ok());
    }

    #[test_all]
    fn test_bech32m_decode_token_id() {
        let token_id = "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87";
        let decoded = super::bech32m_decode_token_id(token_id, Some(Network::Mainnet)).unwrap();
        assert_eq!(
            hex::encode(&decoded),
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca"
        );
    }

    #[test_all]
    fn test_bech32m_encode_token_id() {
        let raw_token_id =
            hex::decode("ee2f1dc42cf0866420f2b0195bb3607199a730c85d7138214b6ad09b55e47542")
                .unwrap();
        let encoded = super::bech32m_encode_token_id(&raw_token_id, Network::Regtest).unwrap();
        assert_eq!(
            encoded,
            "btknrt1ach3m3pv7zrxgg8jkqv4hvmqwxv6wvxgt4cnsg2tdtgfk40yw4pq98h0dl"
        );

        let decoded = super::bech32m_decode_token_id(&encoded, Some(Network::Regtest)).unwrap();
        assert_eq!(decoded, raw_token_id);
    }
}
