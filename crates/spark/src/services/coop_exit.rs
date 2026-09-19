use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{All, PublicKey, Secp256k1};
use bitcoin::{Address, OutPoint, Transaction, Txid};
use frost_secp256k1_tr::Identifier;
use platform_utils::time::SystemTime;
use serde::Serialize;
use tracing::{debug, trace, warn};

use crate::bitcoin::{sighash_from_multi_input_tx, sighash_from_tx};
use crate::core::{Network, next_sequence};
use crate::operator::OperatorPool;
use crate::operator::rpc as operator_rpc;
use crate::services::models::{
    LeafRefundJobs, build_refund_signing_job, into_user_signed_job_groups,
    map_signing_nonce_commitments, sign_leaf_refunds, split_signing_commitments_by_variant,
};
use crate::services::{ExitSpeed, LeafKeyTweak, Transfer, TransferId, TransferService};
use crate::services::{ServiceError, TransferObserver};
use crate::signer::{
    OperatorRecipient, PrepareTransferRequest, PreparedTransfer, SparkSigner, TransferLeafInput,
};
use crate::ssp::RequestCoopExitInput;
use crate::ssp::ServiceProvider;
use crate::ssp::{CoopExitAcceptance, SparkCoopExitRequestStatus};
use crate::tree::TreeNode;
use crate::tree::TreeNodeId;
use crate::utils::frost::derive_leaf_signing_public_key;
use crate::utils::time::web_time_to_prost_timestamp;
use crate::utils::transactions::{
    ConnectorRefundTxsParams, RefundTransactions, create_connector_refund_txs,
};

const COOP_EXIT_EXPIRY_DURATION_MAINNET: Duration = Duration::from_secs(7 * 24 * 60 * 60 + 5 * 60); // 1 week + 5 minutes
const COOP_EXIT_EXPIRY_DURATION: Duration = Duration::from_secs(35 * 60); // 35 minutes

/// Attempts at the completion request that turns a committed transfer into an
/// on-chain payout.
const COMPLETE_EXIT_ATTEMPTS: u32 = 3;
const COMPLETE_EXIT_RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
pub struct CoopExitSpeedFeeQuote {
    pub user_fee_sat: u64,
    pub l1_broadcast_fee_sat: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoopExitFeeQuote {
    pub id: String,
    pub expires_at: u64,
    pub speed_fast: CoopExitSpeedFeeQuote,
    pub speed_medium: CoopExitSpeedFeeQuote,
    pub speed_slow: CoopExitSpeedFeeQuote,
}

impl CoopExitFeeQuote {
    pub fn fee_sats(&self, speed: &ExitSpeed) -> u64 {
        match speed {
            ExitSpeed::Fast => self.speed_fast.l1_broadcast_fee_sat + self.speed_fast.user_fee_sat,
            ExitSpeed::Medium => {
                self.speed_medium.l1_broadcast_fee_sat + self.speed_medium.user_fee_sat
            }
            ExitSpeed::Slow => self.speed_slow.l1_broadcast_fee_sat + self.speed_slow.user_fee_sat,
        }
    }
}

impl TryFrom<crate::ssp::CoopExitFeeQuote> for CoopExitFeeQuote {
    type Error = ServiceError;

    fn try_from(quote: crate::ssp::CoopExitFeeQuote) -> Result<Self, Self::Error> {
        Ok(Self {
            id: quote.id,
            expires_at: quote
                .expires_at
                .timestamp()
                .try_into()
                .map_err(|_| ServiceError::Generic("Failed to parse expires_at".to_string()))?,
            speed_fast: CoopExitSpeedFeeQuote {
                user_fee_sat: quote.user_fee_fast.as_sats()?,
                l1_broadcast_fee_sat: quote.l1_broadcast_fee_fast.as_sats()?,
            },
            speed_medium: CoopExitSpeedFeeQuote {
                user_fee_sat: quote.user_fee_medium.as_sats()?,
                l1_broadcast_fee_sat: quote.l1_broadcast_fee_medium.as_sats()?,
            },
            speed_slow: CoopExitSpeedFeeQuote {
                user_fee_sat: quote.user_fee_slow.as_sats()?,
                l1_broadcast_fee_sat: quote.l1_broadcast_fee_slow.as_sats()?,
            },
        })
    }
}

pub struct CoopExitParams<'a> {
    pub leaves: Vec<LeafKeyTweak>,
    pub withdrawal_address: &'a Address,
    pub withdraw_all: bool,
    pub exit_speed: ExitSpeed,
    pub fee_quote_id: Option<String>,
    pub fee_leaves: Option<Vec<LeafKeyTweak>>,
    pub fee_sats: u64,
    pub transfer_id: Option<TransferId>,
}

pub struct CoopExitService {
    operator_pool: Arc<OperatorPool>,
    ssp_client: Arc<ServiceProvider>,
    transfer_service: Arc<TransferService>,
    network: Network,
    spark_signer: Arc<dyn SparkSigner>,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
    secp: Secp256k1<All>,
}

impl CoopExitService {
    pub fn new(
        operator_pool: Arc<OperatorPool>,
        ssp_client: Arc<ServiceProvider>,
        transfer_service: Arc<TransferService>,
        network: Network,
        spark_signer: Arc<dyn SparkSigner>,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        CoopExitService {
            operator_pool,
            ssp_client,
            transfer_service,
            network,
            spark_signer,
            transfer_observer,
            secp: Secp256k1::new(),
        }
    }

    /// Builds the operator-recipient list for share-encryption from the pool.
    fn operator_recipients(&self) -> Vec<OperatorRecipient> {
        self.operator_pool
            .get_all_operators()
            .map(|op| OperatorRecipient {
                id: op.id,
                identifier: op.identifier,
                public_key: op.identity_public_key,
            })
            .collect()
    }

    pub async fn fetch_coop_exit_fee_quote(
        &self,
        leaves: Vec<TreeNode>,
        withdrawal_address: Address,
    ) -> Result<CoopExitFeeQuote, ServiceError> {
        let leaf_external_ids: Vec<String> =
            leaves.iter().map(|leaf| leaf.id.to_string()).collect();

        self.ssp_client
            .get_coop_exit_fee_quote(leaf_external_ids, &withdrawal_address.to_string())
            .await?
            .try_into()
    }

    pub fn prepare_coop_exit(
        &self,
        leaves: &[LeafKeyTweak],
        transfer_id: Option<TransferId>,
    ) -> PrepareTransferRequest {
        let ssp_identity_public_key = self.ssp_client.identity_public_key();
        let transfer_id = transfer_id.unwrap_or_else(TransferId::generate);
        self.transfer_service.build_transfer_approval_request(
            &transfer_id,
            leaves,
            &ssp_identity_public_key,
        )
    }

    pub async fn coop_exit(&self, params: CoopExitParams<'_>) -> Result<Transfer, ServiceError> {
        self.coop_exit_inner(params, None).await
    }

    pub async fn submit_coop_exit(
        &self,
        params: CoopExitParams<'_>,
        approved_transfer: PreparedTransfer,
    ) -> Result<Transfer, ServiceError> {
        self.coop_exit_inner(params, Some(approved_transfer)).await
    }

    async fn coop_exit_inner(
        &self,
        params: CoopExitParams<'_>,
        prepared: Option<PreparedTransfer>,
    ) -> Result<Transfer, ServiceError> {
        let CoopExitParams {
            leaves,
            withdrawal_address,
            withdraw_all,
            exit_speed,
            fee_quote_id,
            fee_leaves,
            fee_sats,
            transfer_id,
        } = params;
        debug!("Starting cooperative exit with leaves");
        let leaf_external_ids = leaves.iter().map(|l| l.node.id.to_string()).collect();
        let fee_leaf_external_ids = fee_leaves
            .as_ref()
            .map(|fee_leaves| fee_leaves.iter().map(|l| l.node.id.to_string()).collect());
        trace!("Leaf external IDs for cooperative exit: {leaf_external_ids:?}");
        trace!("Fee leaf external IDs for cooperative exit: {fee_leaf_external_ids:?}");

        let unwrapped_transfer_id = match &transfer_id {
            Some(transfer_id) => transfer_id.clone(),
            None => TransferId::generate(),
        };

        let leaves_sum: u64 = leaves.iter().map(|l| l.node.value).sum();
        let expected_payout_amount_sats = if withdraw_all {
            leaves_sum.saturating_sub(fee_sats)
        } else {
            leaves_sum
        };

        if let Some(transfer_observer) = &self.transfer_observer {
            transfer_observer
                .before_coop_exit(&unwrapped_transfer_id, withdrawal_address, leaves_sum)
                .await?;
        }

        let mut leaf_key_tweaks = leaves;
        leaf_key_tweaks.extend(fee_leaves.unwrap_or_default());

        // Request cooperative exit from the SSP
        trace!("Requesting cooperative exit");
        let coop_exit_request = self
            .ssp_client
            .request_coop_exit(RequestCoopExitInput {
                leaf_external_ids,
                withdrawal_address: withdrawal_address.to_string(),
                idempotency_key: None,
                exit_speed: exit_speed.into(),
                withdraw_all,
                fee_leaf_external_ids,
                fee_quote_id,
                user_outbound_transfer_external_id: Some(unwrapped_transfer_id.to_string()),
            })
            .await?;

        let expected_exit_txid = validate_coop_exit_payout_transaction(
            &coop_exit_request.raw_coop_exit_transaction,
            &coop_exit_request.coop_exit_txid,
            withdrawal_address,
            expected_payout_amount_sats,
        )?;

        // Convert the raw connector transaction to a Bitcoin Transaction
        trace!("Processing cooperative exit request: {coop_exit_request:?}",);
        let raw_connector_transaction_bytes =
            hex::decode(coop_exit_request.raw_connector_transaction).map_err(|_| {
                ServiceError::Generic("invalid raw_connector_transaction hex".to_string())
            })?;
        let connector_tx: Transaction =
            bitcoin::consensus::deserialize(&raw_connector_transaction_bytes).map_err(|_| {
                ServiceError::Generic("invalid raw_connector_transaction tx".to_string())
            })?;
        let connector_txid = connector_tx.compute_txid();
        let coop_exit_input = connector_tx.input[0].previous_output.txid;

        if coop_exit_input != expected_exit_txid {
            return Err(ServiceError::ValidationError(format!(
                "SSP connector transaction does not spend the verified exit transaction (spends {coop_exit_input}, expected {expected_exit_txid})"
            )));
        }

        let res = self
            .submit_coop_exit_transfer(
                leaf_key_tweaks,
                connector_txid,
                coop_exit_input,
                unwrapped_transfer_id.clone(),
                raw_connector_transaction_bytes.clone(),
                prepared,
            )
            .await;
        let transfer = match res {
            Ok(t) => t,
            // The transfer can commit server-side and still lose its response. Bind
            // the recovered transfer rather than returning it: the exit is only
            // finished once complete_coop_exit tells the SSP to broadcast the payout.
            Err(e) => {
                self.transfer_service
                    .recover_committed_transfer(&unwrapped_transfer_id, e)
                    .await?
            }
        };
        trace!("Submitted cooperative exit transfer: {transfer:?}");

        self.request_payout(&unwrapped_transfer_id, &coop_exit_request.id)
            .await?;

        Ok(transfer)
    }

    /// Asks the SSP to broadcast the payout for an exit whose transfer has
    /// already committed, retrying while the outcome is unknown.
    ///
    /// By this point the leaves belong to the SSP, so an unanswered request
    /// leaves the withdrawal with nowhere to go: re-reading the request between
    /// attempts settles whether a lost response was in fact received.
    async fn request_payout(
        &self,
        transfer_id: &TransferId,
        request_id: &str,
    ) -> Result<(), ServiceError> {
        let mut last_error = None;
        for attempt in 0..COMPLETE_EXIT_ATTEMPTS {
            if attempt > 0 {
                match self.exit_status(request_id).await {
                    Ok(status) => match status.acceptance() {
                        // A lost response that the SSP did in fact receive.
                        CoopExitAcceptance::Accepted => {
                            debug!("Cooperative exit {request_id} was accepted as {status:?}");
                            return Ok(());
                        }
                        // Asking again cannot revive it, and reporting success
                        // would claim a payout that will never be broadcast.
                        CoopExitAcceptance::Failed => {
                            return Err(ServiceError::Generic(format!(
                                "Cooperative exit {request_id} cannot pay out: {status:?}"
                            )));
                        }
                        CoopExitAcceptance::AwaitingCompletion => {}
                    },
                    Err(e) => debug!("Could not re-read cooperative exit {request_id}: {e:?}"),
                }
                platform_utils::tokio::time::sleep(COMPLETE_EXIT_RETRY_DELAY).await;
            }
            match self
                .ssp_client
                .complete_coop_exit(&transfer_id.to_string(), request_id)
                .await
            {
                Ok(response) => {
                    trace!("Completed cooperative exit: {response:?}");
                    return Ok(());
                }
                Err(e) => {
                    warn!(
                        "Failed to complete cooperative exit {request_id} \
                         (attempt {}/{COMPLETE_EXIT_ATTEMPTS}): {e:?}",
                        attempt.saturating_add(1),
                    );
                    last_error = Some(e);
                }
            }
        }
        Err(last_error.map_or_else(
            || ServiceError::Generic("Cooperative exit completion was never attempted".to_string()),
            ServiceError::from,
        ))
    }

    /// Reads how far the SSP has taken a cooperative exit request.
    async fn exit_status(
        &self,
        request_id: &str,
    ) -> Result<SparkCoopExitRequestStatus, ServiceError> {
        let request = self
            .ssp_client
            .get_coop_exit_request(request_id)
            .await?
            .ok_or_else(|| {
                ServiceError::Generic(format!("Cooperative exit request {request_id} not found"))
            })?;
        Ok(request.exit_status)
    }

    /// Submits the cooperative-exit transfer to the coordinator as a single
    /// `cooperative_exit_v2` call, packaging encrypted key tweaks and
    /// user-signed connector refunds together (operators aggregate and
    /// finalize server-side).
    async fn submit_coop_exit_transfer(
        &self,
        leaf_key_tweaks: Vec<LeafKeyTweak>,
        connector_txid: Txid,
        exit_txid: Txid,
        transfer_id: TransferId,
        connector_tx: Vec<u8>,
        prepared: Option<PreparedTransfer>,
    ) -> Result<Transfer, ServiceError> {
        debug!(
            "Submitting cooperative exit transfer for connector_txid: {connector_txid}, exit_txid: {exit_txid}",
        );
        if leaf_key_tweaks.is_empty() {
            return Err(ServiceError::InvalidInput(
                "submit_coop_exit_transfer requires at least one leaf".to_string(),
            ));
        }
        let receiver_public_key = self.ssp_client.identity_public_key();

        // 1. Fetch operator signing commitments (3 per leaf: cpfp, direct, direct-from-cpfp).
        let signing_commitments = self
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(operator_rpc::spark::GetSigningCommitmentsRequest {
                node_ids: leaf_key_tweaks
                    .iter()
                    .map(|l| l.node.id.to_string())
                    .collect(),
                count: 3,
                node_id_count: 0,
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        let [cpfp_chunk, direct_chunk, direct_from_cpfp_chunk] =
            split_signing_commitments_by_variant(&signing_commitments, leaf_key_tweaks.len())?;

        // 2. Sign coop-exit refunds (connector refunds, decremented timelock)
        //    operator-commits-first into UserSignedTxSigningJob's.
        let connector_tx_parsed: Transaction = bitcoin::consensus::deserialize(&connector_tx)
            .map_err(|_| {
                ServiceError::Generic("Failed to deserialize connector transaction".to_string())
            })?;
        let (cpfp_jobs, direct_jobs, direct_from_cpfp_jobs) = self
            .sign_coop_exit_refunds_into_jobs(
                &leaf_key_tweaks,
                &receiver_public_key,
                connector_txid,
                &connector_tx_parsed,
                cpfp_chunk,
                direct_chunk,
                direct_from_cpfp_chunk,
            )
            .await?;

        // 3. Key-tweak step (to the SSP receiver) + transfer-package signature
        //    via the high-level signer; assemble with the connector refunds.
        let prepared = match prepared {
            Some(prepared) => prepared,
            None => {
                self.spark_signer
                    .prepare_transfer(PrepareTransferRequest {
                        transfer_id: transfer_id.clone(),
                        receiver_public_key,
                        leaves: leaf_key_tweaks
                            .iter()
                            .map(|l| TransferLeafInput {
                                node: l.node.clone(),
                                new_leaf_id: TreeNodeId::generate(),
                                signing_key: l.signing_key.clone(),
                            })
                            .collect(),
                        operator_recipients: self.operator_recipients(),
                        threshold: self.transfer_service.split_secret_threshold(),
                    })
                    .await?
            }
        };

        let signed_package = operator_rpc::spark::TransferPackage {
            delegation_intent: None,
            leaves_to_send: cpfp_jobs,
            direct_leaves_to_send: direct_jobs,
            direct_from_cpfp_leaves_to_send: direct_from_cpfp_jobs,
            key_tweak_package: prepared
                .operator_packages
                .into_iter()
                .map(|p| {
                    (
                        hex::encode(p.operator_identifier.serialize()),
                        p.encrypted_package,
                    )
                })
                .collect(),
            user_signature: prepared.transfer_user_signature.serialize_der().to_vec(),
            hash_variant: operator_rpc::spark::HashVariant::V2.into(),
        };

        let expiry_time = SystemTime::now()
            + if self.network == Network::Mainnet {
                COOP_EXIT_EXPIRY_DURATION_MAINNET
            } else {
                COOP_EXIT_EXPIRY_DURATION
            };

        // 4. Single cooperative_exit_v2 call with the full transfer_package.
        let response =
            self.operator_pool
                .get_coordinator()
                .client
                .cooperative_exit_v2(operator_rpc::spark::CooperativeExitRequest {
                    transfer: Some(operator_rpc::spark::StartTransferRequest {
                        transfer_id: transfer_id.to_string(),
                        owner_identity_public_key: self
                            .spark_signer
                            .get_identity_public_key()
                            .await?
                            .serialize()
                            .to_vec(),
                        receiver_identity_public_key: receiver_public_key.serialize().to_vec(),
                        expiry_time: Some(web_time_to_prost_timestamp(&expiry_time).map_err(
                            |_| ServiceError::Generic("Invalid expiry time".to_string()),
                        )?),
                        transfer_package: Some(signed_package),
                        ..Default::default()
                    }),
                    exit_id: uuid::Uuid::now_v7().to_string(),
                    exit_txid: exit_txid.as_byte_array().to_vec(),
                    connector_tx,
                })
                .await?;

        response
            .transfer
            .ok_or(ServiceError::Generic("No transfer in response".to_string()))?
            .try_into()
    }

    /// Signs the coop-exit connector refund transactions operator-commits-first.
    /// The operators aggregate server-side during `cooperative_exit_v2`.
    #[allow(clippy::too_many_arguments)]
    async fn sign_coop_exit_refunds_into_jobs(
        &self,
        leaves: &[LeafKeyTweak],
        receiving_public_key: &PublicKey,
        connector_txid: Txid,
        connector_tx_parsed: &Transaction,
        cpfp_commitments: &[std::collections::BTreeMap<
            Identifier,
            frost_secp256k1_tr::round1::SigningCommitments,
        >],
        direct_commitments: &[std::collections::BTreeMap<
            Identifier,
            frost_secp256k1_tr::round1::SigningCommitments,
        >],
        direct_from_cpfp_commitments: &[std::collections::BTreeMap<
            Identifier,
            frost_secp256k1_tr::round1::SigningCommitments,
        >],
    ) -> Result<
        (
            Vec<operator_rpc::spark::UserSignedTxSigningJob>,
            Vec<operator_rpc::spark::UserSignedTxSigningJob>,
            Vec<operator_rpc::spark::UserSignedTxSigningJob>,
        ),
        ServiceError,
    > {
        // Build every leaf-variant connector-refund FROST job up front, then sign
        // the whole batch in one call.
        let mut leaf_jobs: Vec<LeafRefundJobs> = Vec::new();
        for (i, leaf) in leaves.iter().enumerate() {
            // The connector refund is signed with the leaf's current (owned)
            // derived key, recovered locally from the tree node instead of the
            // signer (see derive_leaf_signing_public_key). Safe here: the derived
            // value is the signing key, not the refund destination.
            let signing_public_key = derive_leaf_signing_public_key(&leaf.node, &self.secp)?;
            let verifying_key = leaf.node.verifying_public_key;
            let node_tx = &leaf.node.node_tx;
            let connector_prev_out = connector_tx_parsed.output.get(i).cloned();

            // Build the connector refund transactions at the decremented timelock.
            // The funds are refunded to the SSP receiver.
            let refund_tx = leaf
                .node
                .refund_tx
                .clone()
                .ok_or_else(|| ServiceError::Generic("No refund tx".to_string()))?;
            let old_sequence = refund_tx.input[0].sequence;
            let (cpfp_sequence, direct_sequence) = next_sequence(old_sequence)
                .ok_or_else(|| ServiceError::Generic("Failed to get next sequence".to_string()))?;
            let RefundTransactions {
                cpfp_tx: cpfp_refund_tx,
                direct_tx: direct_refund_tx,
                direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
            } = create_connector_refund_txs(ConnectorRefundTxsParams {
                cpfp_sequence,
                direct_sequence,
                node_tx,
                direct_tx: leaf.node.direct_refund_tx(),
                connector_outpoint: OutPoint {
                    txid: connector_txid,
                    vout: i as u32,
                },
                receiving_pubkey: receiving_public_key,
                network: self.network,
            });

            // Coop-exit refunds spend multiple inputs (node_tx + connector); BIP-341
            // sighash requires all prev outs.
            let node_all_prev_outs = connector_prev_out
                .as_ref()
                .map(|c| vec![node_tx.output[0].clone(), c.clone()]);

            let cpfp_sighash = if let Some(prev_outs) = node_all_prev_outs
                .as_ref()
                .filter(|_| cpfp_refund_tx.input.len() > 1)
            {
                sighash_from_multi_input_tx(&cpfp_refund_tx, 0, prev_outs)
            } else {
                sighash_from_tx(&cpfp_refund_tx, 0, &node_tx.output[0])
            }?;
            let cpfp = build_refund_signing_job(
                &leaf.node.id,
                &leaf.signing_key,
                &verifying_key,
                &signing_public_key,
                cpfp_refund_tx,
                *cpfp_sighash.as_byte_array(),
                cpfp_commitments[i].clone(),
                None, // connector refunds never use adaptor signatures
                self.network,
            );

            let direct = if let (Some(direct_tx), Some(direct_refund_tx)) =
                (leaf.node.direct_tx.as_ref(), direct_refund_tx)
            {
                let direct_all_prev_outs = connector_prev_out
                    .as_ref()
                    .map(|c| vec![direct_tx.output[0].clone(), c.clone()]);
                let sighash = if let Some(prev_outs) = direct_all_prev_outs
                    .as_ref()
                    .filter(|_| direct_refund_tx.input.len() > 1)
                {
                    sighash_from_multi_input_tx(&direct_refund_tx, 0, prev_outs)
                } else {
                    sighash_from_tx(&direct_refund_tx, 0, &direct_tx.output[0])
                }?;
                Some(build_refund_signing_job(
                    &leaf.node.id,
                    &leaf.signing_key,
                    &verifying_key,
                    &signing_public_key,
                    direct_refund_tx,
                    *sighash.as_byte_array(),
                    direct_commitments[i].clone(),
                    None, // connector refunds never use adaptor signatures
                    self.network,
                ))
            } else {
                None
            };

            let direct_from_cpfp = if let Some(dfc_refund_tx) = direct_from_cpfp_refund_tx {
                let sighash = if let Some(prev_outs) = node_all_prev_outs
                    .as_ref()
                    .filter(|_| dfc_refund_tx.input.len() > 1)
                {
                    sighash_from_multi_input_tx(&dfc_refund_tx, 0, prev_outs)
                } else {
                    sighash_from_tx(&dfc_refund_tx, 0, &node_tx.output[0])
                }?;
                Some(build_refund_signing_job(
                    &leaf.node.id,
                    &leaf.signing_key,
                    &verifying_key,
                    &signing_public_key,
                    dfc_refund_tx,
                    *sighash.as_byte_array(),
                    direct_from_cpfp_commitments[i].clone(),
                    None, // connector refunds never use adaptor signatures
                    self.network,
                ))
            } else {
                None
            };

            leaf_jobs.push(LeafRefundJobs {
                cpfp,
                direct,
                direct_from_cpfp,
            });
        }

        let signed = sign_leaf_refunds(&self.spark_signer, leaf_jobs).await?;
        into_user_signed_job_groups(signed)
    }
}

fn validate_coop_exit_payout_transaction(
    raw_coop_exit_transaction: &str,
    coop_exit_txid: &str,
    withdrawal_address: &Address,
    expected_payout_amount_sats: u64,
) -> Result<Txid, ServiceError> {
    if raw_coop_exit_transaction.is_empty() || coop_exit_txid.is_empty() {
        return Err(ServiceError::ValidationError(
            "SSP coop exit response missing L1 transaction fields".to_string(),
        ));
    }

    let exit_tx_bytes = hex::decode(raw_coop_exit_transaction).map_err(|_| {
        ServiceError::ValidationError("invalid raw_coop_exit_transaction hex".to_string())
    })?;
    let exit_tx: Transaction = bitcoin::consensus::deserialize(&exit_tx_bytes).map_err(|_| {
        ServiceError::ValidationError("invalid raw_coop_exit_transaction tx".to_string())
    })?;

    let expected_txid: Txid = coop_exit_txid
        .parse()
        .map_err(|_| ServiceError::ValidationError("invalid coop_exit_txid".to_string()))?;
    let actual_txid = exit_tx.compute_txid();
    if actual_txid != expected_txid {
        return Err(ServiceError::ValidationError(format!(
            "SSP coop exit response is inconsistent: coop_exit_txid {expected_txid} does not match raw_coop_exit_transaction {actual_txid}"
        )));
    }

    let expected_script = withdrawal_address.script_pubkey();
    let pays_user = exit_tx.output.iter().any(|out| {
        out.script_pubkey == expected_script && out.value.to_sat() >= expected_payout_amount_sats
    });
    if !pays_user {
        return Err(ServiceError::ValidationError(format!(
            "SSP cooperative exit transaction does not pay the requested withdrawal address for the expected amount (>= {expected_payout_amount_sats} sats)"
        )));
    }

    Ok(expected_txid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::absolute::LockTime;
    use bitcoin::consensus::encode::serialize_hex;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use bitcoin::transaction::Version;
    use bitcoin::{Amount, ScriptBuf, Sequence, TxIn, TxOut, Witness};
    use macros::{async_test_all, test_all};

    use crate::operator::testing::unroutable_operator_pool;
    use crate::session_store::InMemorySessionStore;
    use crate::signer::testing::{RecordingSparkSigner, operator_commitments};
    use crate::signer::{FrostDerivation, LeafSigningKey};
    use crate::ssp::{RetryConfig, ServiceProviderConfig};
    use crate::tree::tests::create_test_leaf_held_under;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn test_address(seed: u8) -> Address {
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(&[seed; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret);
        Address::p2tr(
            &secp,
            pubkey.x_only_public_key().0,
            None,
            bitcoin::Network::Regtest,
        )
    }

    fn exit_tx(outputs: Vec<TxOut>) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: outputs,
        }
    }

    fn paying(addr: &Address, sats: u64) -> (String, String) {
        let tx = exit_tx(vec![TxOut {
            value: Amount::from_sat(sats),
            script_pubkey: addr.script_pubkey(),
        }]);
        (serialize_hex(&tx), tx.compute_txid().to_string())
    }

    #[test_all]
    fn accepts_payout_to_requested_address() {
        let addr = test_address(1);
        let (raw, txid) = paying(&addr, 1000);

        assert!(validate_coop_exit_payout_transaction(&raw, &txid, &addr, 1000).is_ok());
        assert!(validate_coop_exit_payout_transaction(&raw, &txid, &addr, 900).is_ok());
    }

    #[test_all]
    fn rejects_short_amount() {
        let addr = test_address(1);
        let (raw, txid) = paying(&addr, 1000);

        assert!(validate_coop_exit_payout_transaction(&raw, &txid, &addr, 1001).is_err());
    }

    #[test_all]
    fn rejects_wrong_address() {
        let addr = test_address(1);
        let attacker = test_address(2);
        let (raw, txid) = paying(&attacker, 1000);

        assert!(validate_coop_exit_payout_transaction(&raw, &txid, &addr, 1000).is_err());
    }

    #[test_all]
    fn rejects_mismatched_txid() {
        let addr = test_address(1);
        let (raw, _) = paying(&addr, 1000);
        let wrong_txid = Txid::all_zeros().to_string();

        assert!(validate_coop_exit_payout_transaction(&raw, &wrong_txid, &addr, 1000).is_err());
    }

    #[test_all]
    fn rejects_missing_fields() {
        let addr = test_address(1);
        let (raw, txid) = paying(&addr, 1000);

        assert!(validate_coop_exit_payout_transaction("", &txid, &addr, 1000).is_err());
        assert!(validate_coop_exit_payout_transaction(&raw, "", &addr, 1000).is_err());
    }

    /// A coop exit signs every connector refund with the key the leaf is held
    /// under, and records them under that key, whatever id the key derives from.
    #[async_test_all]
    async fn a_coop_exit_signs_its_refunds_with_the_key_the_leaf_is_held_under() {
        for held_under in [TreeNodeId::generate(), "leaf".parse().unwrap()] {
            let recorder = Arc::new(RecordingSparkSigner::new());
            let signer: Arc<dyn SparkSigner> = recorder.clone();
            let identity = signer.get_identity_public_key().await.unwrap();
            let operator_pool = unroutable_operator_pool(&signer).await;
            let ssp_client = Arc::new(
                ServiceProvider::new(
                    ServiceProviderConfig {
                        base_url: "http://127.0.0.1:1".to_string(),
                        schema_endpoint: None,
                        identity_public_key: identity,
                        user_agent: None,
                        retry_config: RetryConfig::default(),
                    },
                    Arc::clone(&signer),
                    Arc::new(InMemorySessionStore::default()),
                    None,
                )
                .unwrap(),
            );
            let transfer_service = Arc::new(TransferService::new(
                Arc::clone(&signer),
                Network::Regtest,
                2,
                Arc::clone(&operator_pool),
                None,
            ));
            let service = CoopExitService::new(
                operator_pool,
                ssp_client,
                transfer_service,
                Network::Regtest,
                Arc::clone(&signer),
                None,
            );
            let held_key = signer.get_public_key_for_leaf(&held_under).await.unwrap();
            let leaf = LeafKeyTweak {
                node: create_test_leaf_held_under("leaf", held_key),
                signing_key: LeafSigningKey {
                    derived_from: held_under.clone(),
                },
            };
            let connector_tx = exit_tx(vec![TxOut {
                value: Amount::from_sat(330),
                script_pubkey: ScriptBuf::new(),
            }]);

            let (cpfp, direct, direct_from_cpfp) = service
                .sign_coop_exit_refunds_into_jobs(
                    std::slice::from_ref(&leaf),
                    &identity,
                    connector_tx.compute_txid(),
                    &connector_tx,
                    &[operator_commitments(3).await],
                    &[operator_commitments(3).await],
                    &[operator_commitments(3).await],
                )
                .await
                .unwrap();

            let derivations = recorder.frost_derivations();
            assert!(!derivations.is_empty());
            assert!(
                derivations.iter().all(|d| *d
                    == FrostDerivation::SigningLeaf {
                        leaf_id: held_under.clone()
                    }),
                "{derivations:?}"
            );
            let jobs: Vec<_> = cpfp
                .iter()
                .chain(&direct)
                .chain(&direct_from_cpfp)
                .collect();
            assert_eq!(jobs.len(), derivations.len());
            for job in jobs {
                assert_eq!(job.leaf_id, "leaf");
                assert_eq!(job.signing_public_key, held_key.serialize().to_vec());
            }
        }
    }
}
