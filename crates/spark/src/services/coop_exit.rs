use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::PublicKey;
use bitcoin::{Address, OutPoint, Transaction, Txid};
use frost_secp256k1_tr::Identifier;
use platform_utils::time::SystemTime;
use serde::Serialize;
use tracing::{debug, trace};

use crate::bitcoin::{sighash_from_multi_input_tx, sighash_from_tx};
use crate::core::Network;
use crate::operator::OperatorPool;
use crate::operator::rpc as operator_rpc;
use crate::services::models::{
    SignedTx, map_signing_nonce_commitments, split_signing_commitments_by_variant,
};
use crate::services::{
    ExitSpeed, LeafKeyTweak, LeafRefundSigningData, Transfer, TransferId, TransferService,
};
use crate::services::{ServiceError, TransferObserver};
use crate::signer::{FrostSigningCommitmentsWithNonces, SecretSource, SignFrostRequest};
use crate::ssp::RequestCoopExitInput;
use crate::ssp::ServiceProvider;
use crate::tree::TreeNodeId;
use crate::utils::leaf_key_tweak::prepare_leaf_key_tweaks_to_send;
use crate::utils::refund::{
    RefundSignatures, prepare_leaf_refund_signing_data,
    prepare_refund_so_signing_jobs_with_tx_constructor,
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
        let leaf_key_tweaks =
            prepare_leaf_key_tweaks_to_send(&self.signer, all_leaves, None).await?;

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
            .submit_coop_exit_transfer(
                leaf_key_tweaks,
                connector_txid,
                coop_exit_input,
                unwrapped_transfer_id,
                raw_connector_transaction_bytes.clone(),
            )
            .await;
        let transfer = match (&transfer_id, res) {
            (_, Ok(t)) => t,
            (Some(transfer_id), Err(e)) => {
                return self
                    .transfer_service
                    .recover_transfer_on_rpc_connection_error(transfer_id, e)
                    .await;
            }
            (None, Err(e)) => return Err(e),
        };
        trace!("Submitted cooperative exit transfer: {transfer:?}");

        let complete_response = self
            .ssp_client
            .complete_coop_exit(&transfer.id.to_string(), &coop_exit_request.id)
            .await?;
        trace!("Completed cooperative exit: {complete_response:?}",);

        Ok(transfer)
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

        // 1. Prepare key tweaks (empty signature maps; the SO hasn't signed yet).
        let key_tweak_input_map = self
            .transfer_service
            .prepare_send_transfer_key_tweaks(
                &transfer_id,
                &receiver_public_key,
                &leaf_key_tweaks,
                RefundSignatures::default(),
            )
            .await?;

        // 2. ECIES-encrypt the key tweaks per operator.
        let encrypted_key_tweaks = self
            .transfer_service
            .encrypt_key_tweaks(&key_tweak_input_map)?;
        let key_tweak_package: HashMap<String, Vec<u8>> = encrypted_key_tweaks
            .into_iter()
            .map(|(k, v)| (hex::encode(k.serialize()), v))
            .collect();

        // 3. Fetch operator signing commitments (3 per leaf: cpfp, direct, direct-from-cpfp).
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

        // 4. Sign coop-exit refunds (connector refunds, decremented timelock)
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

        // 5. Assemble + sign the transfer package.
        let unsigned_package = operator_rpc::spark::TransferPackage {
            leaves_to_send: cpfp_jobs,
            direct_leaves_to_send: direct_jobs,
            direct_from_cpfp_leaves_to_send: direct_from_cpfp_jobs,
            key_tweak_package,
            user_signature: Vec::new(),
            hash_variant: operator_rpc::spark::HashVariant::V2.into(),
        };
        let signed_package = self
            .transfer_service
            .sign_transfer_package(&transfer_id, unsigned_package)
            .await?;

        let expiry_time = SystemTime::now()
            + if self.network == Network::Mainnet {
                COOP_EXIT_EXPIRY_DURATION_MAINNET
            } else {
                COOP_EXIT_EXPIRY_DURATION
            };

        // 6. Single cooperative_exit_v2 call with the full transfer_package.
        let response =
            self.operator_pool
                .get_coordinator()
                .client
                .cooperative_exit_v2(operator_rpc::spark::CooperativeExitRequest {
                    transfer: Some(operator_rpc::spark::StartTransferRequest {
                        transfer_id: transfer_id.to_string(),
                        owner_identity_public_key: self
                            .signer
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
        // Build leaf data map (with the single SSP receiver) + connector prev_out
        // per leaf.
        let mut leaf_data_map =
            prepare_leaf_refund_signing_data(&self.signer, leaves, *receiving_public_key).await?;
        for (i, leaf) in leaves.iter().enumerate() {
            if let Some(leaf_data) = leaf_data_map.get_mut(&leaf.node.id)
                && i < connector_tx_parsed.output.len()
            {
                leaf_data.connector_prev_out = Some(connector_tx_parsed.output[i].clone());
            }
        }

        // Build the connector refund transactions (decremented timelock) into
        // `leaf_data_map`. The (fused-form) signing jobs returned here are
        // discarded — we sign operator-commits-first below.
        prepare_refund_so_signing_jobs_with_tx_constructor(
            leaves,
            &mut leaf_data_map,
            false,
            |c| {
                create_connector_refund_txs(ConnectorRefundTxsParams {
                    cpfp_sequence: c.cpfp_sequence,
                    direct_sequence: c.direct_sequence,
                    node_tx: &c.node.node_tx,
                    direct_tx: c.node.direct_refund_tx(),
                    connector_outpoint: OutPoint {
                        txid: connector_txid,
                        vout: c.vout,
                    },
                    receiving_pubkey: c.receiving_pubkey,
                    network: self.network,
                })
            },
        )?;

        let mut cpfp_jobs = Vec::new();
        let mut direct_jobs = Vec::new();
        let mut direct_from_cpfp_jobs = Vec::new();
        for (i, leaf) in leaves.iter().enumerate() {
            let data = leaf_data_map.remove(&leaf.node.id).ok_or_else(|| {
                ServiceError::Generic(format!("Leaf data not found for leaf {}", leaf.node.id))
            })?;
            let verifying_key = leaf.node.verifying_public_key;
            let signing_private_key = leaf.signing_key.clone();

            let LeafRefundSigningData {
                signing_public_key,
                tx: node_tx,
                direct_tx,
                refund_tx,
                direct_refund_tx,
                direct_from_cpfp_refund_tx,
                signing_nonce_commitment,
                direct_signing_nonce_commitment,
                direct_from_cpfp_signing_nonce_commitment,
                connector_prev_out,
                ..
            } = data;

            // Coop-exit refunds spend multiple inputs (node_tx + connector); BIP-341
            // sighash requires all prev outs.
            let node_all_prev_outs = connector_prev_out
                .as_ref()
                .map(|c| vec![node_tx.output[0].clone(), c.clone()]);

            let cpfp_refund_tx = refund_tx
                .ok_or_else(|| ServiceError::Generic("Missing cpfp refund tx".to_string()))?;
            let cpfp_sighash = if let Some(prev_outs) = node_all_prev_outs
                .as_ref()
                .filter(|_| cpfp_refund_tx.input.len() > 1)
            {
                sighash_from_multi_input_tx(&cpfp_refund_tx, 0, prev_outs)
            } else {
                sighash_from_tx(&cpfp_refund_tx, 0, &node_tx.output[0])
            }?;
            cpfp_jobs.push(
                self.sign_coop_exit_refund_job(
                    &leaf.node.id,
                    cpfp_refund_tx,
                    cpfp_sighash.as_byte_array(),
                    &signing_public_key,
                    &signing_private_key,
                    signing_nonce_commitment,
                    cpfp_commitments[i].clone(),
                    &verifying_key,
                )
                .await?,
            );

            if let (Some(direct_tx), Some(direct_refund_tx)) = (direct_tx, direct_refund_tx) {
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
                direct_jobs.push(
                    self.sign_coop_exit_refund_job(
                        &leaf.node.id,
                        direct_refund_tx,
                        sighash.as_byte_array(),
                        &signing_public_key,
                        &signing_private_key,
                        direct_signing_nonce_commitment,
                        direct_commitments[i].clone(),
                        &verifying_key,
                    )
                    .await?,
                );
            }

            if let Some(dfc_refund_tx) = direct_from_cpfp_refund_tx {
                let sighash = if let Some(prev_outs) = node_all_prev_outs
                    .as_ref()
                    .filter(|_| dfc_refund_tx.input.len() > 1)
                {
                    sighash_from_multi_input_tx(&dfc_refund_tx, 0, prev_outs)
                } else {
                    sighash_from_tx(&dfc_refund_tx, 0, &node_tx.output[0])
                }?;
                direct_from_cpfp_jobs.push(
                    self.sign_coop_exit_refund_job(
                        &leaf.node.id,
                        dfc_refund_tx,
                        sighash.as_byte_array(),
                        &signing_public_key,
                        &signing_private_key,
                        direct_from_cpfp_signing_nonce_commitment,
                        direct_from_cpfp_commitments[i].clone(),
                        &verifying_key,
                    )
                    .await?,
                );
            }
        }

        Ok((cpfp_jobs, direct_jobs, direct_from_cpfp_jobs))
    }

    #[allow(clippy::too_many_arguments)]
    async fn sign_coop_exit_refund_job(
        &self,
        node_id: &TreeNodeId,
        refund_tx: Transaction,
        sighash_bytes: &[u8],
        signing_public_key: &PublicKey,
        signing_private_key: &SecretSource,
        self_nonce_commitment: FrostSigningCommitmentsWithNonces,
        operator_commitments: std::collections::BTreeMap<
            Identifier,
            frost_secp256k1_tr::round1::SigningCommitments,
        >,
        verifying_key: &PublicKey,
    ) -> Result<operator_rpc::spark::UserSignedTxSigningJob, ServiceError> {
        let user_signature = self
            .signer
            .sign_frost(SignFrostRequest {
                message: sighash_bytes,
                public_key: signing_public_key,
                private_key: signing_private_key,
                verifying_key,
                self_nonce_commitment: &self_nonce_commitment,
                statechain_commitments: operator_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let signed_tx = SignedTx {
            node_id: node_id.clone(),
            signing_public_key: *signing_public_key,
            tx: refund_tx,
            user_signature,
            signing_commitments: operator_commitments,
            self_nonce_commitment,
            network: self.network,
        };
        (&signed_tx).try_into()
    }
}
