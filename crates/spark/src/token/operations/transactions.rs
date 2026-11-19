use std::collections::BTreeMap;

use bitcoin::secp256k1::PublicKey;
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use prost::Message as _;
use tracing::{error, info, warn};

use crate::{
    address::SparkAddress,
    operator::rpc::{
        OperatorRpcError,
        spark_token::{BroadcastTransactionRequest, CommitStatus},
    },
    services::{
        ReceiverTokenOutput, ServiceError, TokenInputs, TokenOutputToSpend, TokenTransaction,
        TokenTransactionStatus, TokenTransferInput, TransferTokenOutput,
    },
    signer::{PrepareTokenTransactionRequest, TokenTransactionKind},
    token::{
        BURN_PUBLIC_KEY, MAX_TOKEN_EXTRA_METADATA_BYTES, PreparedTokenPackage,
        PreparedTokenReceiverOutput, PreparedTokenTransfer, ReservationPurpose, ReservationTarget,
        SelectionStrategy, TokenOutpoint, TokenOutput, TokenOutputWithPrevOut, TokenOutputs,
        TokenService, bech32m_decode_token_id, with_reserved_token_outputs,
    },
};

const MAX_TRANSFER_TOKEN_TOO_MANY_OUTPUTS_RETRY_ATTEMPTS: usize = 3;
const MAX_TOKEN_PREEMPTED_RETRY_ATTEMPTS: usize = 3;

/// Sums the requested amount per token id into one reservation target each,
/// ordered by token id so the targets are deterministic.
fn reservation_targets(
    receiver_outputs: &[TransferTokenOutput],
) -> Result<Vec<(String, ReservationTarget)>, ServiceError> {
    if receiver_outputs.is_empty() {
        return Err(ServiceError::Generic(
            "No receiver outputs provided".to_string(),
        ));
    }

    let mut totals = BTreeMap::<String, u128>::new();
    for output in receiver_outputs {
        let total = totals.entry(output.token_id.clone()).or_default();
        *total = total
            .checked_add(output.amount)
            .ok_or_else(|| ServiceError::Generic("Amount overflow".to_string()))?;
    }
    Ok(totals
        .into_iter()
        .map(|(token_id, amount)| (token_id, ReservationTarget::MinTotalValue(amount)))
        .collect())
}

/// Returns the token identifier contributing the most inputs, breaking ties by
/// token id so the choice is deterministic.
fn most_common_token(inputs: &[TokenOutputWithPrevOut]) -> Option<String> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for input in inputs {
        *counts
            .entry(input.output.token_identifier.as_str())
            .or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(token_id, count)| (count, std::cmp::Reverse(token_id)))
        .map(|(token_id, _)| token_id.to_string())
}

/// Checks if an error indicates a transaction was preempted because token outputs
/// were already spent.
fn is_transaction_preempted_error(error: &OperatorRpcError) -> bool {
    matches!(error, OperatorRpcError::Connection(status) if status.code() == tonic::Code::Aborted)
}

/// Build the list of `TransferTokenOutput` entries that consolidation should send
/// to the wallet's own spark address.
///
/// The summed input `amount` is split into `target_output_count` outputs of
/// `amount / target_output_count`. The transfer builder auto-adds any remainder
/// as a change output to the same address, so the resulting on-chain output
/// count is exactly `target_output_count`. If the per-output amount would round
/// down to zero (i.e., `amount < target_output_count`), we fall back to a
/// single output of `amount` to avoid creating zero-value outputs.
fn build_consolidation_outputs(
    token_id: &str,
    amount: u128,
    target_output_count: u32,
    receiver_address: &SparkAddress,
) -> Vec<TransferTokenOutput> {
    let n = u128::from(target_output_count);
    if n <= 1 || amount < n {
        if amount < n {
            warn!(
                "Consolidation amount {} smaller than target_output_count {}; falling back to a single output",
                amount, target_output_count
            );
        }
        return vec![TransferTokenOutput {
            token_id: token_id.to_string(),
            amount,
            receiver_address: receiver_address.clone(),
            spark_invoice: None,
        }];
    }

    let per_output = amount / n;
    // Emit (n - 1) explicit outputs of `per_output`. The remainder
    // (amount - (n - 1) * per_output) becomes the change output added by the transfer builder,
    // also routed back to receiver_address, landing us at exactly `target_output_count` outputs.
    (0..(target_output_count - 1))
        .map(|_| TransferTokenOutput {
            token_id: token_id.to_string(),
            amount: per_output,
            receiver_address: receiver_address.clone(),
            spark_invoice: None,
        })
        .collect()
}

impl TokenService {
    /// Schnorr-signs a 32-byte token-transaction digest with the identity key.
    pub(super) async fn sign_token_digest(
        &self,
        kind: TokenTransactionKind,
        hash: &[u8],
    ) -> Result<Vec<u8>, ServiceError> {
        let digest: [u8; 32] = hash.try_into().map_err(|_| {
            ServiceError::Generic("token transaction hash must be 32 bytes".to_string())
        })?;
        Ok(self
            .spark_signer
            .prepare_token_transaction(PrepareTokenTransactionRequest { kind, digest })
            .await?
            .signature
            .serialize()
            .to_vec())
    }

    pub async fn create_issuer_token(
        &self,
        name: &str,
        ticker: &str,
        decimals: u32,
        is_freezable: bool,
        max_supply: u128,
        extra_metadata: Option<Vec<u8>>,
    ) -> Result<TokenTransaction, ServiceError> {
        if self.get_issuer_token_metadata().await.is_ok() {
            return Err(ServiceError::Generic(
                "Issuer token already exists".to_string(),
            ));
        }

        validate_create_token_params(name, ticker, decimals, extra_metadata.as_deref())?;

        let identity_public_key = self.spark_signer.get_identity_public_key().await?;
        let now = SystemTime::now();
        let built = spark_primitives::construct_partial_create_transaction(
            spark_primitives::CreateBuildRequest {
                issuer_public_key: identity_public_key.serialize().to_vec(),
                token_name: name.to_string(),
                token_ticker: ticker.to_string(),
                decimals,
                max_supply: max_supply.to_be_bytes().to_vec(),
                is_freezable,
                // Absent and empty select different token identifier formats on the operator, so
                // empty is sent as absent to keep the format of tokens created without metadata.
                extra_metadata: extra_metadata.filter(|m| !m.is_empty()),
                operator_identity_public_keys: self.get_operator_identity_public_keys()?,
                network: self.proto_network()?,
                validity_duration_seconds: self.tokens_config.transaction_validity_duration_seconds,
                client_created_timestamp_unix_micros: unix_micros(now)?,
                execute_before_unix_micros: None,
            },
        )
        .map_err(|e| ServiceError::Generic(e.to_string()))?;

        self.broadcast_issuer_transaction(identity_public_key, built, now)
            .await
    }

    pub async fn mint_issuer_token(&self, amount: u128) -> Result<TokenTransaction, ServiceError> {
        let issuer_token_metadata = self.get_issuer_token_metadata().await?;
        let token_identifier =
            bech32m_decode_token_id(&issuer_token_metadata.identifier, Some(self.network))
                .map_err(|_| ServiceError::Generic("Invalid token id".to_string()))?;

        let identity_public_key = self.spark_signer.get_identity_public_key().await?;
        let now = SystemTime::now();
        let built = spark_primitives::construct_partial_mint_transaction(
            spark_primitives::MintBuildRequest {
                issuer_public_key: identity_public_key.serialize().to_vec(),
                token_identifier,
                token_amount: amount.to_be_bytes().to_vec(),
                // Mint to the issuer itself.
                receiver_public_key: None,
                operator_identity_public_keys: self.get_operator_identity_public_keys()?,
                network: self.proto_network()?,
                validity_duration_seconds: self.tokens_config.transaction_validity_duration_seconds,
                client_created_timestamp_unix_micros: unix_micros(now)?,
                withdraw_bond_sats: self.tokens_config.expected_withdraw_bond_sats,
                withdraw_relative_block_locktime: self
                    .tokens_config
                    .expected_withdraw_relative_block_locktime,
                execute_before_unix_micros: None,
            },
        )
        .map_err(|e| ServiceError::Generic(e.to_string()))?;

        self.broadcast_issuer_transaction(identity_public_key, built, now)
            .await
    }

    /// Signs and broadcasts a create or mint partial transaction. Both carry exactly one issuer
    /// signature, at input index 0.
    async fn broadcast_issuer_transaction(
        &self,
        identity_public_key: PublicKey,
        built: spark_primitives::PartialTransferBuildResult,
        created_timestamp: SystemTime,
    ) -> Result<TokenTransaction, ServiceError> {
        let identity_public_key_bytes = identity_public_key.serialize().to_vec();
        let partial_txid = hex::encode(&built.partial_token_transaction_hash);
        let signature = self
            .sign_token_digest(
                TokenTransactionKind::Partial,
                &built.partial_token_transaction_hash,
            )
            .await?;

        let broadcast_bytes = spark_primitives::build_broadcast_transaction_request(
            spark_primitives::BroadcastBuildRequest {
                identity_public_key: identity_public_key_bytes.clone(),
                partial_token_transaction_bytes: built.partial_token_transaction_bytes,
                owner_signatures: vec![spark_primitives::SignatureWithIndexInput {
                    input_index: 0,
                    public_key: identity_public_key_bytes,
                    signature,
                }],
            },
        )
        .map_err(|e| ServiceError::Generic(e.to_string()))?;
        let broadcast_req = BroadcastTransactionRequest::decode(broadcast_bytes.as_slice())
            .map_err(|e| {
                ServiceError::Generic(format!(
                    "Failed to decode broadcast request for tx {partial_txid}: {e}"
                ))
            })?;

        let broadcast_response = self
            .operator_pool
            .get_coordinator()
            .client
            .broadcast_transaction(broadcast_req)
            .await?;

        let commit_status = broadcast_response.commit_status();
        let final_token_transaction =
            broadcast_response
                .final_token_transaction
                .ok_or(ServiceError::Generic(format!(
                    "broadcast response for tx {partial_txid} missing final_token_transaction"
                )))?;
        let final_tx_hash =
            spark_primitives::hash_final_token_transaction(final_token_transaction.encode_to_vec())
                .map_err(|e| {
                    ServiceError::Generic(format!(
                        "Failed to compute final token transaction hash for tx {partial_txid}: {e}"
                    ))
                })?;
        let hash = hex::encode(final_tx_hash);
        let inputs = final_token_transaction
            .token_inputs
            .ok_or(ServiceError::Generic(format!(
                "final token transaction {hash} missing token inputs"
            )))?
            .try_into()?;
        let outputs = final_token_transaction
            .final_token_outputs
            .into_iter()
            .map(|output| (output, self.network).try_into())
            .collect::<Result<Vec<TokenOutput>, ServiceError>>()?;

        let status = match commit_status {
            CommitStatus::CommitFinalized => TokenTransactionStatus::Finalized,
            CommitStatus::CommitProcessing => {
                warn!(
                    "broadcast_transaction for tx {hash} returned COMMIT_PROCESSING; \
                     committed operators: {:?}, uncommitted operators: {:?}",
                    broadcast_response
                        .commit_progress
                        .as_ref()
                        .map(|p| &p.committed_operator_public_keys),
                    broadcast_response
                        .commit_progress
                        .as_ref()
                        .map(|p| &p.uncommitted_operator_public_keys),
                );
                TokenTransactionStatus::Unknown
            }
            CommitStatus::CommitUnspecified => {
                warn!("broadcast_transaction for tx {hash} returned COMMIT_UNSPECIFIED");
                TokenTransactionStatus::Unknown
            }
        };

        Ok(TokenTransaction {
            hash,
            inputs,
            outputs,
            status,
            created_timestamp,
            fulfilled_invoices: Vec::new(),
        })
    }

    fn proto_network(&self) -> Result<u32, ServiceError> {
        u32::try_from(self.network.to_proto_network() as i32)
            .map_err(|_| ServiceError::Generic("network proto value is negative".to_string()))
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
        self.transfer_tokens(
            receiver_outputs,
            preferred_outputs,
            selection_strategy,
            None,
        )
        .await
    }

    pub async fn transfer_tokens(
        &self,
        receiver_outputs: Vec<TransferTokenOutput>,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
        execute_before_unix_micros: Option<i64>,
    ) -> Result<TokenTransaction, ServiceError> {
        let reservation_targets = reservation_targets(&receiver_outputs)?;

        // Stable for the wallet's lifetime; fetch once and reuse across retries and below.
        let identity_public_key = self.spark_signer.get_identity_public_key().await?;

        let mut attempt = 0;
        let mut preempted_attempt = 0;
        let (token_transaction, reservation) = loop {
            if attempt >= MAX_TRANSFER_TOKEN_TOO_MANY_OUTPUTS_RETRY_ATTEMPTS {
                return Err(ServiceError::NeededTooManyOutputs);
            }

            let reservation = self
                .token_output_service
                .reserve_token_outputs(
                    &reservation_targets,
                    ReservationPurpose::Payment,
                    preferred_outputs.clone(),
                    selection_strategy,
                )
                .await?;

            let inputs = reservation.token_outputs.outputs.clone();
            // Captured before `inputs` is consumed, for the over-cap retry below.
            let heaviest_token = most_common_token(&inputs);

            let result = with_reserved_token_outputs(
                self.token_output_service.as_ref(),
                self.transfer_tokens_inner(
                    identity_public_key,
                    inputs,
                    receiver_outputs.clone(),
                    execute_before_unix_micros,
                ),
                &reservation,
            )
            .await;

            match result {
                Ok(token_transaction) => break (token_transaction, reservation),
                Err(ServiceError::NeededTooManyOutputs)
                    if attempt < MAX_TRANSFER_TOKEN_TOO_MANY_OUTPUTS_RETRY_ATTEMPTS - 1 =>
                {
                    // Emergency consolidation during a transfer retry: collapse the token
                    // that contributed the most inputs to a single output, regardless of
                    // the wallet's configured target. That is the one selection whose
                    // shrinking can bring the next attempt back under the cap.
                    if let Some(token_id) = &heaviest_token {
                        self.optimize_token_outputs(Some(token_id), 2, 1).await?;
                    }
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

        let our_outputs = token_transaction
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
        let outputs_to_add = TokenOutputs {
            metadata: reservation.token_outputs.metadata,
            outputs: our_outputs,
        };
        if let Err(e) = self
            .token_output_service
            .update_token_outputs(&[], &outputs_to_add)
            .await
        {
            error!("Failed to update token outputs after broadcast: {e:?}");
        }

        Ok(token_transaction)
    }

    async fn transfer_tokens_inner(
        &self,
        identity_public_key: PublicKey,
        inputs: Vec<TokenOutputWithPrevOut>,
        receiver_outputs: Vec<TransferTokenOutput>,
        execute_before_unix_micros: Option<i64>,
    ) -> Result<TokenTransaction, ServiceError> {
        let prepared = self
            .build_partial_token_transfer(
                identity_public_key,
                inputs,
                receiver_outputs,
                execute_before_unix_micros,
            )
            .await?;
        let signature = self
            .sign_token_digest(
                TokenTransactionKind::Partial,
                &prepared.partial_token_transaction_hash,
            )
            .await?;
        self.broadcast_token_transfer(identity_public_key, prepared, signature)
            .await
    }

    async fn build_partial_token_transfer(
        &self,
        identity_public_key: PublicKey,
        inputs: Vec<TokenOutputWithPrevOut>,
        receiver_outputs: Vec<TransferTokenOutput>,
        execute_before_unix_micros: Option<i64>,
    ) -> Result<PreparedTokenTransfer, ServiceError> {
        if inputs.len() > self.tokens_config.max_tx_inputs {
            return Err(ServiceError::NeededTooManyOutputs);
        }

        let identity_public_key_bytes = identity_public_key.serialize().to_vec();

        // SO sorts inputs by vout before validating owner signatures; match that here.
        let mut inputs = inputs;
        inputs.sort_by_key(|o| o.prev_tx_vout);

        // The transfer builder appends one change output per token, after the caller's receiver
        // outputs and owned by our identity key, so `receiver_outputs` here holds only the caller's
        // outputs and they occupy vouts 0..receiver_outputs.len() in both the partial and the final
        // transaction.
        let selected_outputs = inputs
            .iter()
            .map(|o| {
                let token_identifier =
                    bech32m_decode_token_id(&o.output.token_identifier, Some(self.network))
                        .map_err(|e| {
                            ServiceError::Generic(format!(
                                "Invalid token id '{}': {e}",
                                o.output.token_identifier
                            ))
                        })?;
                Ok(spark_primitives::SelectedTokenOutput {
                    previous_transaction_hash: hex::decode(&o.prev_tx_hash)
                        .map_err(|_| ServiceError::Generic("Invalid prev tx hash".to_string()))?,
                    previous_transaction_vout: o.prev_tx_vout,
                    owner_public_key: o.output.owner_public_key.serialize().to_vec(),
                    token_identifier,
                    token_amount: o.output.token_amount.to_be_bytes().to_vec(),
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let prepared_receivers = receiver_outputs
            .iter()
            .map(|o| {
                let pay_request = match &o.spark_invoice {
                    Some(invoice) => invoice.clone(),
                    None => o
                        .receiver_address
                        .to_address_string()
                        .map_err(|e| ServiceError::Generic(e.to_string()))?,
                };
                Ok(PreparedTokenReceiverOutput {
                    token_id: o.token_id.clone(),
                    amount: o.amount,
                    pay_request,
                    spark_invoice: o.spark_invoice.clone(),
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let prim_receiver_outputs = prepared_receivers
            .iter()
            .map(|o| {
                Ok(spark_primitives::ReceiverTokenOutput {
                    receiver_spark_address: o.pay_request.clone(),
                    token_identifier: Some(
                        bech32m_decode_token_id(&o.token_id, Some(self.network)).map_err(|e| {
                            ServiceError::Generic(format!("Invalid token id '{}': {e}", o.token_id))
                        })?,
                    ),
                    token_amount: Some(o.amount.to_be_bytes().to_vec()),
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let now = SystemTime::now();
        let client_created_timestamp_unix_micros = unix_micros(now)?;

        let result = spark_primitives::construct_partial_transfer_transaction(
            spark_primitives::TransferBuildRequest {
                identity_public_key: identity_public_key_bytes,
                selected_outputs,
                receiver_outputs: prim_receiver_outputs,
                operator_identity_public_keys: self.get_operator_identity_public_keys()?,
                network: self.proto_network()?,
                validity_duration_seconds: self.tokens_config.transaction_validity_duration_seconds,
                client_created_timestamp_unix_micros,
                withdraw_bond_sats: self.tokens_config.expected_withdraw_bond_sats,
                withdraw_relative_block_locktime: self
                    .tokens_config
                    .expected_withdraw_relative_block_locktime,
                execute_before_unix_micros,
            },
        )
        .map_err(|e| ServiceError::Generic(e.to_string()))?;

        let spent_outpoints = inputs
            .iter()
            .map(|o| TokenOutpoint {
                prev_tx_hash: o.prev_tx_hash.clone(),
                prev_tx_vout: o.prev_tx_vout,
                token_id: o.output.token_identifier.clone(),
            })
            .collect();

        Ok(PreparedTokenTransfer {
            partial_token_transaction_hash: result.partial_token_transaction_hash,
            partial_token_transaction_bytes: result.partial_token_transaction_bytes,
            spent_outpoints,
            receiver_outputs: prepared_receivers,
            created_timestamp: now,
        })
    }

    async fn broadcast_token_transfer(
        &self,
        identity_public_key: PublicKey,
        prepared: PreparedTokenTransfer,
        signature: Vec<u8>,
    ) -> Result<TokenTransaction, ServiceError> {
        let identity_public_key_bytes = identity_public_key.serialize().to_vec();
        let PreparedTokenTransfer {
            partial_token_transaction_hash,
            partial_token_transaction_bytes,
            spent_outpoints,
            receiver_outputs,
            created_timestamp,
        } = prepared;

        let owner_signatures = spent_outpoints
            .iter()
            .enumerate()
            .map(|(i, _)| spark_primitives::SignatureWithIndexInput {
                input_index: i as u32,
                public_key: identity_public_key_bytes.clone(),
                signature: signature.clone(),
            })
            .collect::<Vec<_>>();

        let partial_txid = hex::encode(&partial_token_transaction_hash);

        if let Some(observer) = &self.transfer_observer {
            observer
                .before_send_token(
                    &partial_txid,
                    receiver_outputs
                        .iter()
                        .map(|o| ReceiverTokenOutput {
                            pay_request: o.pay_request.clone(),
                            amount: o.amount,
                            token_id: o.token_id.clone(),
                        })
                        .collect::<Vec<ReceiverTokenOutput>>(),
                )
                .await?;
        }

        let broadcast_bytes = spark_primitives::build_broadcast_transaction_request(
            spark_primitives::BroadcastBuildRequest {
                identity_public_key: identity_public_key_bytes,
                partial_token_transaction_bytes,
                owner_signatures,
            },
        )
        .map_err(|e| ServiceError::Generic(e.to_string()))?;

        let broadcast_req = BroadcastTransactionRequest::decode(broadcast_bytes.as_slice())
            .map_err(|e| {
                ServiceError::Generic(format!(
                    "Failed to decode broadcast request for tx {partial_txid} (encoded bytes len={}): {e}",
                    broadcast_bytes.len()
                ))
            })?;

        let broadcast_response = self
            .operator_pool
            .get_coordinator()
            .client
            .broadcast_transaction(broadcast_req)
            .await?;

        // Broadcast succeeded, so the reserved inputs are spent. Errors must not propagate past
        // here: with_reserved_token_outputs cancels the reservation on Err, wrongly returning the
        // spent outputs to the available pool. Log and continue instead.
        let commit_status = broadcast_response.commit_status();
        let commit_progress = broadcast_response.commit_progress;
        let final_token_transaction = broadcast_response.final_token_transaction;

        // Fall back to the partial txid if the response is malformed, so a parse failure can't roll
        // back the now-broadcast reservation.
        let final_outputs = match final_token_transaction {
            Some(final_token_transaction) => spark_primitives::hash_final_token_transaction(
                final_token_transaction.encode_to_vec(),
            )
            .map_err(|e| format!("Failed to compute final token transaction hash: {e}"))
            .and_then(|final_tx_hash| {
                let txid = hex::encode(&final_tx_hash);
                final_token_transaction
                    .final_token_outputs
                    .into_iter()
                    .map(|fo| {
                        (fo, self.network)
                            .try_into()
                            .map_err(|e: ServiceError| e.to_string())
                    })
                    .collect::<Result<Vec<TokenOutput>, String>>()
                    .map(|outputs| (txid, outputs))
            }),
            None => Err("broadcast response missing final_token_transaction".to_string()),
        };

        let (txid, outputs) = match final_outputs {
            Ok(value) => value,
            Err(e) => {
                error!(
                    "Failed to read final outputs for tx {partial_txid}: {e}; using the partial txid \
                     so the broadcast reservation is finalized rather than rolled back"
                );
                (partial_txid.clone(), Vec::new())
            }
        };

        let is_finalized = match commit_status {
            CommitStatus::CommitFinalized => true,
            CommitStatus::CommitProcessing => {
                warn!(
                    "broadcast_transaction for tx {txid} returned COMMIT_PROCESSING; \
                     committed operators: {:?}, uncommitted operators: {:?}",
                    commit_progress
                        .as_ref()
                        .map(|p| &p.committed_operator_public_keys),
                    commit_progress
                        .as_ref()
                        .map(|p| &p.uncommitted_operator_public_keys),
                );
                false
            }
            CommitStatus::CommitUnspecified => {
                warn!("broadcast_transaction for tx {txid} returned COMMIT_UNSPECIFIED");
                false
            }
        };

        // Operator outputs are authoritative post-broadcast; log mismatches rather than propagate.
        // The final transaction carries our receiver outputs at vouts 0..receiver_outputs.len(),
        // followed by any change output the builder appended, so it can be longer but never shorter.
        if outputs.len() < receiver_outputs.len() {
            warn!(
                "broadcast returned {} final outputs but expected at least {} for tx {txid}",
                outputs.len(),
                receiver_outputs.len()
            );
        }
        for (i, (output, expected)) in outputs.iter().zip(receiver_outputs.iter()).enumerate() {
            if output.token_amount != expected.amount {
                warn!(
                    "final output {i} for tx {txid} has amount {} but expected {}",
                    output.token_amount, expected.amount
                );
            }
            if output.token_identifier != expected.token_id {
                warn!(
                    "final output {i} for tx {txid} has token identifier {} but expected {}",
                    output.token_identifier, expected.token_id
                );
            }
        }

        if let Some(observer) = &self.transfer_observer
            && let Err(e) = observer
                .after_send_token(&partial_txid, &txid, receiver_outputs.len())
                .await
        {
            warn!("after_send_token observer failed for tx {txid}: {e:?}");
        }

        let outputs_to_spend = spent_outpoints
            .iter()
            .map(|o| TokenOutputToSpend {
                prev_token_tx_hash: o.prev_tx_hash.clone(),
                prev_token_tx_vout: o.prev_tx_vout,
            })
            .collect();

        Ok(TokenTransaction {
            hash: txid,
            inputs: TokenInputs::Transfer(TokenTransferInput { outputs_to_spend }),
            outputs,
            status: if is_finalized {
                TokenTransactionStatus::Finalized
            } else {
                TokenTransactionStatus::Unknown
            },
            created_timestamp,
            fulfilled_invoices: receiver_outputs
                .into_iter()
                .filter_map(|o| o.spark_invoice)
                .collect(),
        })
    }

    pub async fn prepare_token_transfer(
        &self,
        receiver_outputs: Vec<TransferTokenOutput>,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
        execute_before_unix_micros: Option<i64>,
    ) -> Result<PreparedTokenPackage, ServiceError> {
        let targets = reservation_targets(&receiver_outputs)?;
        let identity_public_key = self.spark_signer.get_identity_public_key().await?;

        let inputs = self
            .token_output_service
            .select_token_outputs(&targets, preferred_outputs, selection_strategy)
            .await?
            .outputs;

        // The cap is on the transaction's total inputs, so it is checked against every
        // token's selection combined. Consolidating the token that contributed the most
        // inputs is what brings the next attempt back under the cap.
        if inputs.len() > self.tokens_config.max_tx_inputs {
            let token_id = most_common_token(&inputs).ok_or_else(|| {
                ServiceError::Generic("No inputs selected for transfer".to_string())
            })?;
            let prepared = self
                .prepare_token_consolidation(identity_public_key, &token_id)
                .await?;
            return Ok(PreparedTokenPackage::Consolidation(prepared));
        }

        let prepared = self
            .build_partial_token_transfer(
                identity_public_key,
                inputs,
                receiver_outputs,
                execute_before_unix_micros,
            )
            .await?;
        Ok(PreparedTokenPackage::Ready(prepared))
    }

    /// Builds a self-transfer that collapses the wallet's smallest token outputs
    /// into one, without signing or broadcasting. This is the client-signing
    /// counterpart to one `optimize_token_outputs` iteration: the client signs
    /// the returned transfer, publishes it, then rebuilds the original send.
    async fn prepare_token_consolidation(
        &self,
        identity_public_key: PublicKey,
        token_id: &str,
    ) -> Result<PreparedTokenTransfer, ServiceError> {
        let selected = self
            .token_output_service
            .select_token_outputs(
                &[(
                    token_id.to_string(),
                    ReservationTarget::MaxOutputCount(self.tokens_config.max_tx_inputs),
                )],
                None,
                Some(SelectionStrategy::SmallestFirst),
            )
            .await?;
        let amount = selected
            .outputs
            .iter()
            .map(|o| o.output.token_amount)
            .sum::<u128>();
        let receiver_address = SparkAddress::new(identity_public_key, self.network, None);
        let receiver_outputs = build_consolidation_outputs(token_id, amount, 1, &receiver_address);
        self.build_partial_token_transfer(
            identity_public_key,
            selected.outputs,
            receiver_outputs,
            None,
        )
        .await
    }

    pub async fn submit_token_transfer(
        &self,
        prepared: PreparedTokenTransfer,
        signature: Vec<u8>,
    ) -> Result<TokenTransaction, ServiceError> {
        let identity_public_key = self.spark_signer.get_identity_public_key().await?;

        // The package may spend outpoints of several tokens; one reservation covers
        // all of them.
        let outpoints: Vec<(String, u32)> = prepared
            .spent_outpoints
            .iter()
            .map(|o| (o.prev_tx_hash.clone(), o.prev_tx_vout))
            .collect();

        let reservation = self
            .token_output_service
            .reserve_token_outputs_by_outpoints(&outpoints, ReservationPurpose::Payment)
            .await?;

        let token_transaction = with_reserved_token_outputs(
            self.token_output_service.as_ref(),
            self.broadcast_token_transfer(identity_public_key, prepared, signature),
            &reservation,
        )
        .await?;

        if let Err(e) = self
            .update_token_outputs_for_transaction(&token_transaction, &identity_public_key)
            .await
        {
            error!("Failed to update token outputs after broadcast: {e:?}");
        }
        Ok(token_transaction)
    }

    /// Optimizes token outputs by consolidating them when there are more than the configured threshold.
    /// Processes one token at a time. Token identifier can be provided, otherwise one is automatically selected.
    /// Only optimizes if the number of outputs is greater than the provided `min_outputs_threshold` (min 2).
    ///
    /// `target_output_count` controls how many outputs the consolidation transfer produces.
    /// The summed input amount is split into roughly-equal outputs to the wallet's own
    /// spark address; any integer-division remainder becomes the change output (also
    /// self-addressed), so the resulting output count is exactly `target_output_count`.
    /// If the per-output amount would round down to zero, the consolidation falls back
    /// to a single output to avoid creating zero-value outputs.
    pub async fn optimize_token_outputs(
        &self,
        token_identifier: Option<&str>,
        min_outputs_threshold: u32,
        target_output_count: u32,
    ) -> Result<(), ServiceError> {
        if min_outputs_threshold <= 1 {
            return Err(ServiceError::ValidationError(
                "min_outputs_threshold must be greater than 1".to_string(),
            ));
        }
        if target_output_count < 1 {
            return Err(ServiceError::ValidationError(
                "target_output_count must be at least 1".to_string(),
            ));
        }
        if target_output_count >= min_outputs_threshold {
            return Err(ServiceError::ValidationError(
                "target_output_count must be strictly less than min_outputs_threshold".to_string(),
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
                    &[(
                        output.metadata.identifier.clone(),
                        ReservationTarget::MaxOutputCount(self.tokens_config.max_tx_inputs),
                    )],
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

            let identity_public_key = self.spark_signer.get_identity_public_key().await?;
            let receiver_address = SparkAddress::new(identity_public_key, self.network, None);
            let receiver_outputs = build_consolidation_outputs(
                &output.metadata.identifier,
                amount,
                target_output_count,
                &receiver_address,
            );

            let token_transaction = with_reserved_token_outputs(
                self.token_output_service.as_ref(),
                self.transfer_tokens_inner(
                    identity_public_key,
                    reservation.token_outputs.outputs.clone(),
                    receiver_outputs,
                    None,
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
                .update_token_outputs(
                    &[],
                    &TokenOutputs {
                        metadata: reservation.token_outputs.metadata,
                        outputs,
                    },
                )
                .await?;
        }

        if !did_optimize {
            info!("No token outputs to optimize");
        }

        Ok(())
    }

    fn get_operator_identity_public_keys(&self) -> Result<Vec<Vec<u8>>, ServiceError> {
        let operators = self.operator_pool.get_all_operators();
        let keys = operators
            .map(|o| o.identity_public_key.serialize().to_vec())
            .collect();
        Ok(keys)
    }
}

pub(crate) fn unix_micros(time: SystemTime) -> Result<i64, ServiceError> {
    i64::try_from(
        time.duration_since(UNIX_EPOCH)
            .map_err(|_| {
                ServiceError::Generic("client_created_timestamp is before UNIX_EPOCH".to_string())
            })?
            .as_micros(),
    )
    .map_err(|_| ServiceError::Generic("client_created_timestamp overflows i64".to_string()))
}

fn validate_create_token_params(
    name: &str,
    ticker: &str,
    decimals: u32,
    extra_metadata: Option<&[u8]>,
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
    if extra_metadata.is_some_and(|m| m.len() > MAX_TOKEN_EXTRA_METADATA_BYTES) {
        return Err(ServiceError::Generic(format!(
            "Token extra metadata must be at most {MAX_TOKEN_EXTRA_METADATA_BYTES} bytes"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::PublicKey;
    use macros::test_all;

    use super::{
        ReservationTarget, TransferTokenOutput, build_consolidation_outputs, most_common_token,
        reservation_targets, validate_create_token_params,
    };
    use crate::{Network, address::SparkAddress};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_validate_create_token_params_valid() {
        // Valid parameters
        assert!(validate_create_token_params("Bitcoin", "BTC", 8, None).is_ok());
        assert!(validate_create_token_params("ABC", "ABC", 0, None).is_ok());
        assert!(validate_create_token_params("12345678901234567890", "ABCDEF", 255, None).is_ok());
    }

    #[test_all]
    fn test_validate_create_token_params_name_too_short() {
        let result = validate_create_token_params("AB", "BTC", 8, None);
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
        let result = validate_create_token_params("123456789012345678901", "BTC", 8, None);
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
        let result = validate_create_token_params("Bitcoin", "AB", 8, None);
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
        let result = validate_create_token_params("Bitcoin", "ABCDEFG", 8, None);
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
        let result = validate_create_token_params("Bitcoin", "BTC", 256, None);
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
        let result = validate_create_token_params(non_nfc_name, "BTC", 8, None);
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
        let result = validate_create_token_params("Bitcoin", non_nfc_ticker, 8, None);
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
        assert!(validate_create_token_params("ABC", "ABC", 0, None).is_ok());

        // Maximum valid values
        assert!(validate_create_token_params("12345678901234567890", "ABCDEF", 255, None).is_ok());

        // Exactly at boundaries
        assert!(validate_create_token_params("123", "ABC", 0, None).is_ok());
        assert!(validate_create_token_params("12345678901234567890", "ABC", 0, None).is_ok());
        assert!(validate_create_token_params("Bitcoin", "ABC", 0, None).is_ok());
        assert!(validate_create_token_params("Bitcoin", "ABCDEF", 0, None).is_ok());
    }

    #[test_all]
    fn test_validate_create_token_params_extra_metadata() {
        let max = vec![0_u8; super::MAX_TOKEN_EXTRA_METADATA_BYTES];
        assert!(validate_create_token_params("Bitcoin", "BTC", 8, Some(&max)).is_ok());

        let too_long = vec![0_u8; super::MAX_TOKEN_EXTRA_METADATA_BYTES + 1];
        assert!(validate_create_token_params("Bitcoin", "BTC", 8, Some(&too_long)).is_err());
    }

    fn test_receiver_address() -> SparkAddress {
        const TEST_PUBKEY_BYTES: [u8; 33] = [
            3, 141, 37, 201, 160, 148, 226, 93, 184, 201, 131, 47, 222, 91, 55, 171, 38, 95, 13,
            248, 175, 190, 44, 132, 189, 75, 131, 204, 215, 82, 93, 167, 177,
        ];
        SparkAddress {
            identity_public_key: PublicKey::from_slice(&TEST_PUBKEY_BYTES).unwrap(),
            network: Network::Mainnet,
            spark_invoice_fields: None,
        }
    }

    const TEST_TOKEN_ID: &str = "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87";
    const TEST_TOKEN_ID_B: &str =
        "btkn1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";

    fn transfer_output(token_id: &str, amount: u128) -> TransferTokenOutput {
        TransferTokenOutput {
            token_id: token_id.to_string(),
            amount,
            receiver_address: test_receiver_address(),
            spark_invoice: None,
        }
    }

    fn input_of(token_id: &str, amount: u128, vout: u32) -> crate::token::TokenOutputWithPrevOut {
        crate::token::TokenOutputWithPrevOut {
            output: crate::token::TokenOutput {
                owner_public_key: test_receiver_address().identity_public_key,
                revocation_commitment: "commitment".to_string(),
                withdraw_bond_sats: 1,
                withdraw_relative_block_locktime: 1,
                token_public_key: None,
                token_identifier: token_id.to_string(),
                token_amount: amount,
            },
            prev_tx_hash: format!("hash-{vout}"),
            prev_tx_vout: vout,
        }
    }

    fn target_amount(targets: &[(String, ReservationTarget)], token_id: &str) -> Option<u128> {
        targets
            .iter()
            .find(|(id, _)| id == token_id)
            .map(|(_, t)| match t {
                ReservationTarget::MinTotalValue(amount) => *amount,
                ReservationTarget::MaxOutputCount(count) => *count as u128,
            })
    }

    #[test_all]
    fn test_reservation_targets_sums_per_token() {
        let targets = reservation_targets(&[
            transfer_output(TEST_TOKEN_ID, 10),
            transfer_output(TEST_TOKEN_ID_B, 5),
            transfer_output(TEST_TOKEN_ID, 7),
        ])
        .unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(target_amount(&targets, TEST_TOKEN_ID), Some(17));
        assert_eq!(target_amount(&targets, TEST_TOKEN_ID_B), Some(5));
    }

    #[test_all]
    fn test_reservation_targets_rejects_empty() {
        assert!(reservation_targets(&[]).is_err());
    }

    #[test_all]
    fn test_reservation_targets_detects_overflow() {
        assert!(
            reservation_targets(&[
                transfer_output(TEST_TOKEN_ID, u128::MAX),
                transfer_output(TEST_TOKEN_ID, 1),
            ])
            .is_err()
        );
    }

    #[test_all]
    fn test_most_common_token_picks_largest_contributor() {
        let inputs = vec![
            input_of(TEST_TOKEN_ID_B, 1, 0),
            input_of(TEST_TOKEN_ID, 1, 1),
            input_of(TEST_TOKEN_ID, 1, 2),
        ];
        assert_eq!(most_common_token(&inputs).as_deref(), Some(TEST_TOKEN_ID));
        assert_eq!(most_common_token(&[]), None);
    }

    #[test_all]
    fn test_build_consolidation_outputs_target_zero_returns_single_output() {
        let receiver = test_receiver_address();
        let outputs = build_consolidation_outputs(TEST_TOKEN_ID, 100, 0, &receiver);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].amount, 100);
        assert_eq!(outputs[0].token_id, TEST_TOKEN_ID);
        assert_eq!(outputs[0].receiver_address, receiver);
        assert!(outputs[0].spark_invoice.is_none());
    }

    #[test_all]
    fn test_build_consolidation_outputs_target_one_returns_single_output() {
        let receiver = test_receiver_address();
        let outputs = build_consolidation_outputs(TEST_TOKEN_ID, 100, 1, &receiver);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].amount, 100);
    }

    #[test_all]
    fn test_build_consolidation_outputs_amount_less_than_target_falls_back() {
        let receiver = test_receiver_address();
        // amount=3, target=5: cannot split into 5 non-zero outputs, fall back to one.
        let outputs = build_consolidation_outputs(TEST_TOKEN_ID, 3, 5, &receiver);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].amount, 3);
    }

    #[test_all]
    fn test_build_consolidation_outputs_amount_equal_to_target_splits_into_ones() {
        let receiver = test_receiver_address();
        // amount=5, target=5: per_output=1, emit 4 outputs of 1; change of 1 is
        // added by the transfer builder, yielding 5 outputs total.
        let outputs = build_consolidation_outputs(TEST_TOKEN_ID, 5, 5, &receiver);
        assert_eq!(outputs.len(), 4);
        for out in &outputs {
            assert_eq!(out.amount, 1);
        }
    }

    #[test_all]
    fn test_build_consolidation_outputs_even_split() {
        let receiver = test_receiver_address();
        // amount=1000, target=4: per_output=250, emit 3 explicit outputs of 250;
        // change of 250 lands as the 4th output.
        let outputs = build_consolidation_outputs(TEST_TOKEN_ID, 1000, 4, &receiver);
        assert_eq!(outputs.len(), 3);
        for out in &outputs {
            assert_eq!(out.amount, 250);
            assert_eq!(out.token_id, TEST_TOKEN_ID);
            assert_eq!(out.receiver_address, receiver);
        }
    }

    #[test_all]
    fn test_build_consolidation_outputs_uneven_split_remainder_goes_to_change() {
        let receiver = test_receiver_address();
        // amount=1003, target=4: per_output=250, emit 3 explicit outputs of 250.
        // Change = 1003 - 750 = 253 will be added by the transfer builder.
        let outputs = build_consolidation_outputs(TEST_TOKEN_ID, 1003, 4, &receiver);
        assert_eq!(outputs.len(), 3);
        let explicit_total: u128 = outputs.iter().map(|o| o.amount).sum();
        assert_eq!(explicit_total, 750);
        for out in &outputs {
            assert_eq!(out.amount, 250);
        }
    }

    #[test_all]
    fn test_build_consolidation_outputs_large_u128_amount() {
        let receiver = test_receiver_address();
        // Sanity-check that the math does not overflow at the u128 ceiling.
        let outputs = build_consolidation_outputs(TEST_TOKEN_ID, u128::MAX, 10, &receiver);
        assert_eq!(outputs.len(), 9);
        let per_output = u128::MAX / 10;
        for out in &outputs {
            assert_eq!(out.amount, per_output);
        }
    }
}
