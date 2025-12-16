use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::Hash;
use bitcoin::{Address, OutPoint, Transaction, Txid};
use serde::Serialize;
use tracing::{debug, trace};
use web_time::SystemTime;

use crate::core::Network;
use crate::operator::OperatorPool;
use crate::operator::rpc as operator_rpc;
use crate::services::{
    ExitSpeed, LeafKeyTweak, LeafRefundSigningData, Transfer, TransferId, TransferService,
};
use crate::services::{ServiceError, TransferObserver};
use crate::ssp::RequestCoopExitInput;
use crate::ssp::ServiceProvider;
use crate::tree::TreeNodeId;
use crate::utils::leaf_key_tweak::prepare_leaf_key_tweaks_to_send;
use crate::utils::refund::{
    RefundSignatures, map_refund_signatures, prepare_leaf_refund_signing_data,
    prepare_refund_so_signing_jobs_with_tx_constructor, sign_aggregate_refunds,
};
use crate::utils::time::web_time_to_prost_timestamp;
use crate::utils::transactions::{ConnectorRefundTxsParams, create_connector_refund_txs};
use crate::{signer::Signer, tree::TreeNode};

const COOP_EXIT_EXPIRY_DURATION_MAINNET: Duration = Duration::from_secs(7 * 24 * 60 * 60 + 5 * 60); // 1 week + 5 minutes
const COOP_EXIT_EXPIRY_DURATION: Duration = Duration::from_secs(35 * 60); // 35 minutes

#[derive(Debug, Clone, Serialize)]
pub struct CoopExitSpeedFeeQuote {
    pub user_fee_sat: u64,
    pub l1_broadcast_fee_sat: u64,
}

#[derive(Debug)]
struct CoopExitRefundSignatures {
    pub transfer: Transfer,
    pub refund_signatures: RefundSignatures,
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
    pub leaves: Vec<TreeNode>,
    pub withdrawal_address: &'a Address,
    pub withdraw_all: bool,
    pub exit_speed: ExitSpeed,
    pub fee_quote_id: Option<String>,
    pub fee_leaves: Option<Vec<TreeNode>>,
    pub transfer_id: Option<TransferId>,
}

pub struct CoopExitService {
    operator_pool: Arc<OperatorPool>,
    ssp_client: Arc<ServiceProvider>,
    transfer_service: Arc<TransferService>,
    network: Network,
    signer: Arc<dyn Signer>,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
}

impl CoopExitService {
    pub fn new(
        operator_pool: Arc<OperatorPool>,
        ssp_client: Arc<ServiceProvider>,
        transfer_service: Arc<TransferService>,
        network: Network,
        signer: Arc<dyn Signer>,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        CoopExitService {
            operator_pool,
            ssp_client,
            transfer_service,
            network,
            signer,
            transfer_observer,
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

    pub async fn coop_exit(&self, params: CoopExitParams<'_>) -> Result<Transfer, ServiceError> {
        let CoopExitParams {
            leaves,
            withdrawal_address,
            withdraw_all,
            exit_speed,
            fee_quote_id,
            fee_leaves,
            transfer_id,
        } = params;
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

        let unwrapped_transfer_id = match &transfer_id {
            Some(transfer_id) => transfer_id.clone(),
            None => TransferId::generate(),
        };
        if let Some(transfer_observer) = &self.transfer_observer {
            let amount_sats: u64 = leaves.iter().map(|l| l.value).sum();
            transfer_observer
                .before_coop_exit(&unwrapped_transfer_id, withdrawal_address, amount_sats)
                .await?;
        }

        // Build leaf key tweaks for all leaves with new signing keys
        let all_leaves = [leaves, fee_leaves.unwrap_or_default()].concat();
        let leaf_key_tweaks = prepare_leaf_key_tweaks_to_send(&self.signer, all_leaves, None).await?;

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

        let res = self
            .get_connector_refund_signatures(
                leaf_key_tweaks,
                connector_txid,
                coop_exit_input,
                unwrapped_transfer_id,
            )
            .await;
        let coop_exit_refund_signatures = match (&transfer_id, res) {
            (_, Ok(s)) => s,
            (Some(transfer_id), Err(e)) => {
                return self
                    .transfer_service
                    .recover_transfer_on_rpc_connection_error(transfer_id, e)
                    .await;
            }
            (None, Err(e)) => return Err(e),
        };
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
        transfer_id: TransferId,
    ) -> Result<CoopExitRefundSignatures, ServiceError> {
        debug!(
            "Getting connector refund signatures for connector_txid: {connector_txid}, exit_txid: {exit_txid}",
        );
        let coop_exit_refund_signatures = self
            .sign_coop_exit_refunds(&leaf_key_tweaks, connector_txid, exit_txid, transfer_id)
            .await?;

        trace!("Delivering transfer package for cooperative exit refund signatures");
        let transfer_tweaked = self
            .transfer_service
            .deliver_transfer_package(
                &coop_exit_refund_signatures.transfer,
                &leaf_key_tweaks,
                coop_exit_refund_signatures.refund_signatures.clone(),
            )
            .await?;

        Ok(CoopExitRefundSignatures {
            transfer: transfer_tweaked,
            refund_signatures: coop_exit_refund_signatures.refund_signatures,
        })
    }

    async fn sign_coop_exit_refunds(
        &self,
        leaf_key_tweaks: &[LeafKeyTweak],
        connector_txid: Txid,
        exit_txid: Txid,
        transfer_id: TransferId,
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

        let expiry_time = SystemTime::now()
            + if self.network == Network::Mainnet {
                COOP_EXIT_EXPIRY_DURATION_MAINNET
            } else {
                COOP_EXIT_EXPIRY_DURATION
            };

        // Call the coordinator to get signing results
        // TODO: Use `transfer_package` as `leaves_to_send` is deprecated
        trace!("Calling coordinator for cooperative exit signing results");
        let response =
            self.operator_pool
                .get_coordinator()
                .client
                .cooperative_exit_v2(operator_rpc::spark::CooperativeExitRequest {
                    transfer: Some(operator_rpc::spark::StartTransferRequest {
                        transfer_id: transfer_id.to_string(),
                        #[allow(deprecated)]
                        leaves_to_send: signing_jobs,
                        owner_identity_public_key: self
                            .signer
                            .get_identity_public_key().await?
                            .serialize()
                            .to_vec(),
                        receiver_identity_public_key: self
                            .ssp_client
                            .identity_public_key()
                            .serialize()
                            .to_vec(),
                        expiry_time: Some(web_time_to_prost_timestamp(&expiry_time).map_err(
                            |_| ServiceError::Generic("Invalid expiry time".to_string()),
                        )?),
                        transfer_package: None,
                        ..Default::default()
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
            None,
            None,
        )
        .await?;

        trace!("Converting signed refunds to map");
        let refund_signatures = map_refund_signatures(signed_refunds)?;

        Ok(CoopExitRefundSignatures {
            transfer,
            refund_signatures,
        })
    }

    fn prepare_refund_so_signing_jobs(
        &self,
        leaf_key_tweaks: &[LeafKeyTweak],
        connector_txid: Txid,
        leaf_data_map: &mut HashMap<TreeNodeId, LeafRefundSigningData>,
    ) -> Result<Vec<operator_rpc::spark::LeafRefundTxSigningJob>, ServiceError> {
        prepare_refund_so_signing_jobs_with_tx_constructor(
            leaf_key_tweaks,
            leaf_data_map,
            false,
            |refund_tx_constructor| {
                create_connector_refund_txs(ConnectorRefundTxsParams {
                    cpfp_sequence: refund_tx_constructor.cpfp_sequence,
                    direct_sequence: refund_tx_constructor.direct_sequence,
                    cpfp_outpoint: refund_tx_constructor.refund_tx.input[0].previous_output,
                    direct_outpoint: refund_tx_constructor
                        .node
                        .direct_refund_tx
                        .as_ref()
                        .map(|tx| tx.input[0].previous_output),
                    connector_outpoint: OutPoint {
                        txid: connector_txid,
                        vout: refund_tx_constructor.vout,
                    },
                    amount_sats: refund_tx_constructor.node.value,
                    receiving_pubkey: refund_tx_constructor.receiving_pubkey,
                    network: self.network,
                })
            },
        )
    }
}
