use bitcoin::hashes::Hash;
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::{Address, OutPoint, Transaction, Txid};
use prost_types::Timestamp;
use serde::Serialize;
use tracing::{debug, trace};

use crate::core::Network;
use crate::operator::OperatorPool;
use crate::services::ServiceError;
use crate::ssp::ServiceProvider;
use crate::utils::leaf_key_tweak::prepare_leaf_key_tweaks_to_send;
use crate::utils::transactions::create_coop_exit_refund_tx;
use crate::{signer::Signer, tree::TreeNode};
use std::sync::Arc;

use crate::address::SparkAddress;
use crate::core::next_sequence;
use crate::operator::rpc as operator_rpc;
use crate::services::{
    ExitSpeed, LeafKeyTweak, LeafRefundSigningData, Transfer, TransferId, TransferService,
};
use crate::ssp::RequestCoopExitInput;
use crate::tree::TreeNodeId;
use crate::utils::refund::{
    node_signatures_to_map, prepare_leaf_refund_signing_data, sign_aggregate_refunds,
};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

const COOP_EXIT_EXPIRY_DURATION_MAINNET: Duration = Duration::from_secs(24 * 60 * 60 * 2); // 48 hours
const COOP_EXIT_EXPIRY_DURATION: Duration = Duration::from_secs(60 * 5); // 5 minutes

#[derive(Debug, Clone, Serialize)]
pub struct CoopExitSpeedFeeQuote {
    pub user_fee_sat: u64,
    pub l1_broadcast_fee_sat: u64,
}

#[derive(Debug)]
struct CoopExitRefundSignatures {
    pub transfer: Transfer,
    pub refund_signature_map: HashMap<TreeNodeId, Signature>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoopExitFeeQuote {
    pub id: String,
    pub expires_at: i64,
    pub speed_fast: CoopExitSpeedFeeQuote,
    pub speed_medium: CoopExitSpeedFeeQuote,
    pub speed_slow: CoopExitSpeedFeeQuote,
}

impl TryFrom<crate::ssp::CoopExitFeeQuote> for CoopExitFeeQuote {
    type Error = ServiceError;

    fn try_from(quote: crate::ssp::CoopExitFeeQuote) -> Result<Self, Self::Error> {
        Ok(Self {
            id: quote.id,
            expires_at: quote.expires_at.timestamp(),
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

pub struct CoopExitService<S> {
    operator_pool: Arc<OperatorPool<S>>,
    ssp_client: Arc<ServiceProvider<S>>,
    transfer_service: Arc<TransferService<S>>,
    network: Network,
    signer: S,
}

impl<S> CoopExitService<S>
where
    S: Signer,
{
    pub fn new(
        operator_pool: Arc<OperatorPool<S>>,
        ssp_client: Arc<ServiceProvider<S>>,
        transfer_service: Arc<TransferService<S>>,
        network: Network,
        signer: S,
    ) -> Self {
        CoopExitService {
            operator_pool,
            ssp_client,
            transfer_service,
            network,
            signer,
        }
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

    pub async fn coop_exit(
        &self,
        leaves: Vec<TreeNode>,
        withdrawal_address: &Address,
        withdraw_all: bool,
        exit_speed: ExitSpeed,
        fee_quote_id: Option<String>,
        fee_leaves: Option<Vec<TreeNode>>,
    ) -> Result<Transfer, ServiceError> {
        debug!("Starting cooperative exit with leaves");
        let leaf_external_ids = leaves.iter().map(|l| l.id.clone().to_string()).collect();
        let fee_leaf_external_ids = fee_leaves.as_ref().map(|fee_leaves| {
            fee_leaves
                .iter()
                .map(|l| l.id.clone().to_string())
                .collect()
        });
        trace!("Leaf external IDs for cooperative exit: {leaf_external_ids:?}");
        trace!("Fee leaf external IDs for cooperative exit: {fee_leaf_external_ids:?}");

        // Build leaf key tweaks for all leaves with new signing keys
        let all_leaves = [leaves, fee_leaves.unwrap_or_default()].concat();
        let leaf_key_tweaks = prepare_leaf_key_tweaks_to_send(&self.signer, all_leaves)?;

        // Request cooperative exit from the SSP
        trace!("Requesting cooperative exit");
        let coop_exit_request = self
            .ssp_client
            .request_coop_exit(RequestCoopExitInput {
                leaf_external_ids,
                withdrawal_address: withdrawal_address.to_string(),
                idempotency_key: uuid::Uuid::now_v7().to_string(),
                exit_speed: exit_speed.into(),
                withdraw_all,
                fee_leaf_external_ids,
                fee_quote_id,
            })
            .await?;

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

        let coop_exit_refund_signatures = self
            .get_connector_refund_signatures(leaf_key_tweaks, connector_txid, coop_exit_input)
            .await?;
        trace!("Got connector refund signatures: {coop_exit_refund_signatures:?}",);
        let transfer = coop_exit_refund_signatures.transfer;

        let complete_response = self
            .ssp_client
            .complete_coop_exit(&transfer.id.to_string(), &coop_exit_request.id)
            .await?;
        trace!("Completed cooperative exit: {complete_response:?}",);

        Ok(transfer)
    }

    async fn get_connector_refund_signatures(
        &self,
        leaf_key_tweaks: Vec<LeafKeyTweak>,
        connector_txid: Txid,
        exit_txid: Txid,
    ) -> Result<CoopExitRefundSignatures, ServiceError> {
        debug!(
            "Getting connector refund signatures for connector_txid: {connector_txid}, exit_txid: {exit_txid}",
        );
        let coop_exit_refund_signatures = self
            .sign_coop_exit_refunds(&leaf_key_tweaks, connector_txid, exit_txid)
            .await?;

        trace!("Delivering transfer package for cooperative exit refund signatures");
        let transfer_tweaked = self
            .transfer_service
            .deliver_transfer_package(
                &coop_exit_refund_signatures.transfer,
                &leaf_key_tweaks,
                coop_exit_refund_signatures.refund_signature_map.clone(),
            )
            .await?;

        Ok(CoopExitRefundSignatures {
            transfer: transfer_tweaked,
            refund_signature_map: coop_exit_refund_signatures.refund_signature_map,
        })
    }

    async fn sign_coop_exit_refunds(
        &self,
        leaf_key_tweaks: &[LeafKeyTweak],
        connector_txid: Txid,
        exit_txid: Txid,
    ) -> Result<CoopExitRefundSignatures, ServiceError> {
        debug!(
            "Signing cooperative exit refunds for connector_txid: {connector_txid}, exit_txid: {exit_txid}",
        );
        // Prepare leaf data map with refund signing information
        let receiving_public_key = self.ssp_client.identity_public_key();
        let mut leaf_data_map =
            prepare_leaf_refund_signing_data(&self.signer, leaf_key_tweaks, receiving_public_key)
                .await?;

        // Prepare refund signing jobs for the coordinator
        trace!("Preparing refund signing jobs for cooperative exit");
        let signing_jobs = self.prepare_refund_so_signing_jobs(
            leaf_key_tweaks,
            connector_txid,
            &mut leaf_data_map,
        )?;

        // Create the Spark payment intent
        // TODO: Is the amount needed here?
        trace!("Creating Spark payment intent for cooperative exit");
        let spark_payment_intent =
            SparkAddress::new(self.signer.get_identity_public_key()?, self.network, None)
                .to_address_string()
                .map_err(|e| {
                    ServiceError::Generic(format!("error creating spark payment intent: {e}"))
                })?;

        let transfer_id = TransferId::generate();
        let expiry_time = if self.network == Network::Mainnet {
            COOP_EXIT_EXPIRY_DURATION_MAINNET
        } else {
            COOP_EXIT_EXPIRY_DURATION
        };

        // Call the coordinator to get signing results
        // TODO: Use `transfer_package` as `leaves_to_send` is deprecated
        trace!("Calling coordinator for cooperative exit signing results");
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .cooperative_exit(operator_rpc::spark::CooperativeExitRequest {
                transfer: Some(operator_rpc::spark::StartTransferRequest {
                    transfer_id: transfer_id.to_string(),
                    #[allow(deprecated)]
                    leaves_to_send: signing_jobs,
                    owner_identity_public_key: self
                        .signer
                        .get_identity_public_key()?
                        .serialize()
                        .to_vec(),
                    receiver_identity_public_key: self
                        .ssp_client
                        .identity_public_key()
                        .serialize()
                        .to_vec(),
                    expiry_time: Some(Timestamp::from(SystemTime::now() + expiry_time)),
                    transfer_package: None,
                    spark_payment_intent,
                }),
                exit_id: uuid::Uuid::now_v7().to_string(),
                exit_txid: exit_txid.as_byte_array().to_vec(),
            })
            .await?;
        let transfer = response
            .transfer
            .ok_or(ServiceError::Generic("No transfer in response".to_string()))?
            .try_into()?;

        // Sign the refunds using FROST
        trace!("Signing aggregate refunds for cooperative exit");
        let signed_refunds = sign_aggregate_refunds(
            &self.signer,
            &leaf_data_map,
            &response.signing_results,
            None,
        )
        .await?;

        trace!("Converting signed refunds to map");
        let refund_signature_map = node_signatures_to_map(signed_refunds)?;

        Ok(CoopExitRefundSignatures {
            transfer,
            refund_signature_map,
        })
    }

    fn prepare_refund_so_signing_jobs(
        &self,
        leaf_key_tweaks: &[LeafKeyTweak],
        connector_txid: Txid,
        leaf_data_map: &mut HashMap<TreeNodeId, LeafRefundSigningData>,
    ) -> Result<Vec<operator_rpc::spark::LeafRefundTxSigningJob>, ServiceError> {
        debug!("Preparing refund signing jobs for connector_txid: {connector_txid}",);
        let mut signing_jobs = Vec::new();
        for (i, leaf_key_tweak) in leaf_key_tweaks.iter().enumerate() {
            let refund_signing_data: &mut LeafRefundSigningData = leaf_data_map
                .get_mut(&leaf_key_tweak.node.id)
                .ok_or_else(|| {
                    ServiceError::Generic(format!(
                        "Leaf data not found for leaf {}",
                        leaf_key_tweak.node.id
                    ))
                })?;

            let refund_tx = leaf_key_tweak
                .node
                .refund_tx
                .clone()
                .ok_or(ServiceError::Generic("No refund tx".to_string()))?;

            let sequence = next_sequence(refund_tx.input[0].sequence).ok_or(
                ServiceError::Generic("Failed to get next sequence".to_string()),
            )?;

            trace!(
                "Creating refund transaction for leaf {} with sequence {sequence} and connector vout {i}",
                leaf_key_tweak.node.id
            );
            let connector_refund_tx = create_coop_exit_refund_tx(
                sequence,
                refund_tx.input[0].previous_output,
                OutPoint {
                    txid: connector_txid,
                    vout: i as u32,
                },
                leaf_key_tweak.node.value,
                &refund_signing_data.receiving_public_key,
                self.network,
            );

            trace!(
                "Creating signing job for leaf {} with refund tx: {}",
                leaf_key_tweak.node.id,
                connector_refund_tx.compute_txid()
            );
            let signing_job = operator_rpc::spark::LeafRefundTxSigningJob {
                leaf_id: leaf_key_tweak.node.id.to_string(),
                refund_tx_signing_job: Some(operator_rpc::spark::SigningJob {
                    signing_public_key: refund_signing_data.signing_public_key.serialize().to_vec(),
                    raw_tx: bitcoin::consensus::serialize(&connector_refund_tx),
                    signing_nonce_commitment: Some(
                        refund_signing_data
                            .signing_nonce_commitment
                            .commitments
                            .try_into()?,
                    ),
                }),
            };

            refund_signing_data.refund_tx = Some(connector_refund_tx);
            signing_jobs.push(signing_job);
        }

        Ok(signing_jobs)
    }
}
