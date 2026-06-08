use std::time::Duration;
use std::{collections::HashMap, sync::Arc};

use crate::Network;
use crate::address::SparkAddress;
use crate::operator::OperatorPool;
use crate::operator::rpc::spark::transfer_filter::Participant;
use crate::operator::rpc::spark::{HashVariant, StartTransferRequest, TransferFilter};
use crate::operator::rpc::{self as operator_rpc, OperatorRpcError};
use crate::services::models::{
    LeafKeyTweak, Transfer, map_signing_nonce_commitments, split_signing_commitments_by_variant,
};
use crate::services::{TransferId, TransferObserver, TransferStatus};
use crate::signer::EncryptedSecret;
use crate::utils::leaf_key_tweak::prepare_leaf_key_tweaks_to_send;
use crate::utils::paging::{PagingFilter, PagingResult, pager};
use crate::utils::refund::{SignRefundsParams, SignedRefundTransactions, sign_refunds};
use crate::utils::tagged_hasher::TaggedHasher;
use crate::utils::time::web_time_to_prost_timestamp;

use bitcoin::Transaction;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::Identifier;
use platform_utils::time::SystemTime;
use platform_utils::tokio;
use tracing::{debug, error, trace, warn};

use crate::{
    bitcoin::sighash_from_tx,
    core::{current_sequence, enforce_timelock},
    signer::{
        ClaimLeafInput, FrostDerivation, FrostJob, OperatorRecipient, PrepareClaimRequest,
        PrepareTransferRequest, SparkSigner, TransferLeafInput,
    },
    tree::{TreeNode, TreeNodeId},
    utils::transactions::{RefundTransactions, create_refund_txs},
};

use super::ServiceError;
use super::models::{PreparedTransferRequest, SignedTx};

/// Result of preparing a transfer package, containing both the package and the signed transaction data
pub(crate) struct PreparedTransferPackage {
    pub transfer_package: operator_rpc::spark::TransferPackage,
    pub cpfp_signed_txs: Vec<SignedTx>,
}

/// Configuration for claiming transfers
pub struct ClaimTransferConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for ClaimTransferConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 1000,
            max_delay_ms: 10000,
        }
    }
}

pub struct TransferService {
    spark_signer: Arc<dyn SparkSigner>,
    network: Network,
    split_secret_threshold: u32,
    operator_pool: Arc<OperatorPool>,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
}

impl TransferService {
    pub fn new(
        spark_signer: Arc<dyn SparkSigner>,
        network: Network,
        split_secret_threshold: u32,
        operator_pool: Arc<OperatorPool>,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        Self {
            spark_signer,
            network,
            split_secret_threshold,
            operator_pool,
            transfer_observer,
        }
    }

    /// Creates and initiates a new transfer with the given leaves.
    ///
    /// Generates a transfer package containing encrypted key material, refund signatures,
    /// and proofs that are distributed to the statechain operators.
    pub async fn transfer_leaves_to(
        &self,
        leaves: Vec<TreeNode>,
        receiver_id: &PublicKey,
        transfer_id: Option<TransferId>,
        spark_invoice: Option<String>,
    ) -> Result<Transfer, ServiceError> {
        let unwrapped_transfer_id = match &transfer_id {
            Some(transfer_id) => transfer_id.clone(),
            None => TransferId::generate(),
        };

        if let Some(transfer_observer) = &self.transfer_observer {
            let identity_public_key = &self.spark_signer.get_identity_public_key().await?;
            if identity_public_key != receiver_id {
                let receiver_address = SparkAddress::new(*receiver_id, self.network, None);
                let amount_sats: u64 = leaves.iter().map(|l| l.value).sum();
                transfer_observer
                    .before_send_transfer(
                        &unwrapped_transfer_id,
                        &spark_invoice
                            .clone()
                            .or(receiver_address.to_address_string().ok())
                            .ok_or(ServiceError::Generic(
                                "No pay request available".to_string(),
                            ))?,
                        amount_sats,
                    )
                    .await?;
            }
        }

        // build leaf key tweaks with new signing keys that we will send to the receiver
        let leaf_key_tweaks = prepare_leaf_key_tweaks_to_send(leaves);
        let transfer_res = self
            .send_transfer_with_key_tweaks(
                &unwrapped_transfer_id,
                &leaf_key_tweaks,
                receiver_id,
                spark_invoice,
            )
            .await;
        let transfer = match (&transfer_id, transfer_res) {
            (_, Ok(t)) => t,
            (Some(transfer_id), Err(e)) => {
                return self
                    .recover_transfer_on_rpc_connection_error(transfer_id, e)
                    .await;
            }
            (None, Err(e)) => return Err(e),
        };

        Ok(transfer)
    }

    pub(crate) async fn recover_transfer_on_rpc_connection_error(
        &self,
        transfer_id: &TransferId,
        error: ServiceError,
    ) -> Result<Transfer, ServiceError> {
        if let ServiceError::ServiceConnectionError(operator_rpc_error) = &error
            && let OperatorRpcError::Connection(status) = operator_rpc_error.as_ref()
            && matches!(
                status.code(),
                tonic::Code::Internal | tonic::Code::AlreadyExists
            )
        {
            // There was an RPC connection error. Check if the transfer already exists remotely.
            let operator_transfers = self
                .operator_pool
                .get_coordinator()
                .client
                .query_all_transfers(TransferFilter {
                    transfer_ids: vec![transfer_id.to_string()],
                    network: self.network.to_proto_network() as i32,
                    participant: Some(Participant::SenderIdentityPublicKey(
                        self.spark_signer
                            .get_identity_public_key()
                            .await?
                            .serialize()
                            .to_vec(),
                    )),
                    ..Default::default()
                })
                .await?;
            if let Some(transfer) = operator_transfers.transfers.into_iter().nth(0) {
                debug!("Recovered transfer {} after connection error", transfer.id);

                return transfer.try_into();
            }
        }

        Err(error)
    }

    pub async fn send_transfer_with_key_tweaks(
        &self,
        transfer_id: &TransferId,
        leaf_key_tweaks: &[LeafKeyTweak],
        receiver_id: &PublicKey,
        spark_invoice: Option<String>,
    ) -> Result<Transfer, ServiceError> {
        let prepared_package = self
            .prepare_transfer_package(
                transfer_id,
                leaf_key_tweaks,
                receiver_id,
                None,
                None, // No adaptor public key for regular transfers
            )
            .await?;

        // Make request to start transfer
        let start_transfer_request = operator_rpc::spark::StartTransferRequest {
            transfer_id: transfer_id.to_string(),
            owner_identity_public_key: self
                .spark_signer
                .get_identity_public_key()
                .await?
                .serialize()
                .to_vec(),
            receiver_identity_public_key: receiver_id.serialize().to_vec(),
            transfer_package: Some(prepared_package.transfer_package),
            spark_invoice: spark_invoice.unwrap_or_default(),
            ..Default::default()
        };
        trace!(
            "About to send start_transfer_request: {:?}",
            start_transfer_request
        );
        let transfer = self
            .operator_pool
            .get_coordinator()
            .client
            .start_transfer_v2(start_transfer_request)
            .await?
            .transfer
            .ok_or(ServiceError::Generic(
                "No transfer from operator".to_string(),
            ))?;

        transfer.try_into()
    }

    pub(crate) async fn prepare_transfer_package(
        &self,
        transfer_id: &TransferId,
        leaf_key_tweaks: &[LeafKeyTweak],
        receiver_public_key: &PublicKey,
        payment_hash: Option<&sha256::Hash>,
        cpfp_adaptor_public_key: Option<&PublicKey>,
    ) -> Result<PreparedTransferPackage, ServiceError> {
        if leaf_key_tweaks.is_empty() {
            return Err(ServiceError::InvalidInput(
                "prepare_transfer_package requires at least one leaf".to_string(),
            ));
        }

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
        let cpfp_signing_commitments = cpfp_chunk.to_vec();
        let direct_signing_commitments = direct_chunk.to_vec();
        let direct_from_cpfp_signing_commitments = direct_from_cpfp_chunk.to_vec();

        // Refund signing (operator-commits-first) with the old leaf key.
        let SignedRefundTransactions {
            cpfp_signed_tx,
            direct_signed_tx,
            direct_from_cpfp_signed_tx,
        } = sign_refunds(SignRefundsParams {
            spark_signer: &self.spark_signer,
            leaves: leaf_key_tweaks,
            cpfp_signing_commitments,
            direct_signing_commitments,
            direct_from_cpfp_signing_commitments,
            receiver_pubkey: receiver_public_key,
            payment_hash,
            network: self.network,
            cpfp_adaptor_public_key,
        })
        .await?;

        // Key-tweak / Feldman-split / ECIES / transfer-payload signing. The
        // signer generates the new receiver key and produces the per-operator
        // encrypted key-tweak packages plus the transfer-package signature.
        let prepared = self
            .spark_signer
            .prepare_transfer(PrepareTransferRequest {
                transfer_id: transfer_id.clone(),
                receiver_public_key: *receiver_public_key,
                leaves: leaf_key_tweaks
                    .iter()
                    .map(|l| TransferLeafInput {
                        node: l.node.clone(),
                        new_leaf_id: TreeNodeId::generate(),
                    })
                    .collect(),
                operator_recipients: self.operator_recipients(),
                threshold: self.split_secret_threshold,
            })
            .await?;

        let transfer_package = operator_rpc::spark::TransferPackage {
            leaves_to_send: cpfp_signed_tx
                .iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            direct_leaves_to_send: direct_signed_tx
                .iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            direct_from_cpfp_leaves_to_send: direct_from_cpfp_signed_tx
                .iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
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
            hash_variant: HashVariant::V2.into(),
        };

        Ok(PreparedTransferPackage {
            transfer_package,
            cpfp_signed_txs: cpfp_signed_tx,
        })
    }

    /// The Feldman-split threshold this service is configured with.
    pub(crate) fn split_secret_threshold(&self) -> u32 {
        self.split_secret_threshold
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

    pub async fn prepare_transfer_request(
        &self,
        transfer_id: &TransferId,
        leaves: &[LeafKeyTweak],
        receiver_public_key: &PublicKey,
        payment_hash: Option<&sha256::Hash>,
        expiry_time: Option<SystemTime>,
        cpfp_adaptor_public_key: Option<&PublicKey>,
    ) -> Result<PreparedTransferRequest, ServiceError> {
        let prepared_package = self
            .prepare_transfer_package(
                transfer_id,
                leaves,
                receiver_public_key,
                payment_hash,
                cpfp_adaptor_public_key,
            )
            .await?;

        Ok(PreparedTransferRequest {
            transfer_request: StartTransferRequest {
                transfer_id: transfer_id.to_string(),
                owner_identity_public_key: self
                    .spark_signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
                receiver_identity_public_key: receiver_public_key.serialize().to_vec(),
                expiry_time: expiry_time
                    .map(|t| web_time_to_prost_timestamp(&t))
                    .transpose()
                    .map_err(|_| ServiceError::Generic("Invalid expiry time".to_string()))?,
                transfer_package: Some(prepared_package.transfer_package),
                ..Default::default()
            },
            cpfp_signed_txs: prepared_package.cpfp_signed_txs,
        })
    }

    /// Claims a transfer with retry logic and automatic leaf preparation.
    ///
    /// Returns the claimed leaves on success. If a concurrent instance of this
    /// wallet wins the race and finalizes the transfer, the coordinator's finalized
    /// leaves are returned uniformly — callers do not need to distinguish this case.
    pub async fn claim_transfer(
        &self,
        transfer: &Transfer,
        config: Option<ClaimTransferConfig>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let config = config.unwrap_or_default();

        let mut retry_count = 0;
        loop {
            if retry_count >= config.max_retries {
                return Err(ServiceError::MaxRetriesExceeded);
            }

            // Introduce an exponential backoff delay before retrying.
            if retry_count > 0 {
                let delay_ms =
                    (config.base_delay_ms * 2u64.pow(retry_count - 1)).min(config.max_delay_ms);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }

            match self.try_claim_transfer(transfer).await {
                Ok(nodes) => return Ok(nodes),
                Err(ServiceError::NoLeavesToClaim) => {
                    debug!("There are no leaves to claim for this transfer");
                    return Ok(Vec::new());
                }
                Err(e) => {
                    error!("Failed to claim transfer: {}", e);
                    // A concurrent instance of this wallet may have finalized the
                    // transfer — if so, use its leaves instead of retrying.
                    if let Some(leaves) =
                        self.finalized_leaves_if_already_claimed(&transfer.id).await
                    {
                        return Ok(leaves);
                    }
                    retry_count += 1;
                }
            }
        }
    }

    /// Single claim attempt: verify, prepare leaves, and run the 3-step claim flow.
    async fn try_claim_transfer(&self, transfer: &Transfer) -> Result<Vec<TreeNode>, ServiceError> {
        let leaf_key_map = self.verify_pending_transfer(transfer).await?;
        let leaves_to_claim = self
            .prepare_leaves_for_claiming(transfer, &leaf_key_map)
            .await?;
        self.claim_transfer_with_leaves(transfer, leaves_to_claim)
            .await
    }

    /// If the transfer is `Completed` on the coordinator, returns its finalized
    /// leaves. Used after a failed claim attempt to detect that another instance
    /// of this wallet already finalized the claim concurrently.
    ///
    /// A failed coordinator query is non-fatal here — it's logged and treated as
    /// "not completed" so the caller falls through to its normal error handling.
    async fn finalized_leaves_if_already_claimed(
        &self,
        transfer_id: &TransferId,
    ) -> Option<Vec<TreeNode>> {
        let completed = match self.query_transfer(transfer_id).await {
            Ok(Some(t)) if t.status == TransferStatus::Completed => t,
            Ok(_) => return None,
            Err(e) => {
                warn!("Failed to check if transfer {transfer_id} was claimed concurrently: {e:?}");
                return None;
            }
        };
        debug!(
            "Transfer {transfer_id} already claimed by another instance; using coordinator's finalized leaves"
        );
        let our_pubkey = self.spark_signer.get_identity_public_key().await.ok()?;
        let leaves: Vec<TreeNode> = completed
            .leaves
            .into_iter()
            .map(|l| l.leaf)
            .filter(|leaf| {
                let is_ours = leaf.owner_identity_public_key == Some(our_pubkey);
                if !is_ours {
                    debug!(
                        "Dropping leaf {} from already-claimed transfer {transfer_id} — \
                         owner {:?} is no longer us",
                        leaf.id, leaf.owner_identity_public_key
                    );
                }
                is_ours
            })
            .collect();
        Some(leaves)
    }

    /// Prepares leaves for claiming by creating LeafKeyTweak structs
    async fn prepare_leaves_for_claiming(
        &self,
        transfer: &Transfer,
        leaf_key_map: &HashMap<TreeNodeId, EncryptedSecret>,
    ) -> Result<Vec<LeafKeyTweak>, ServiceError> {
        let mut leaves_to_claim = Vec::new();

        for leaf in &transfer.leaves {
            let Some(leaf_key) = leaf_key_map.get(&leaf.leaf.id) else {
                continue;
            };

            leaves_to_claim.push(LeafKeyTweak {
                node: leaf.leaf_with_intermediate_txs(),
                incoming_key: Some(leaf_key.clone()),
            });
        }

        if leaves_to_claim.is_empty() {
            return Err(ServiceError::NoLeavesToClaim);
        }

        Ok(leaves_to_claim)
    }

    /// Low-level claim transfer operation
    async fn claim_transfer_with_leaves(
        &self,
        transfer: &Transfer,
        leaves_to_claim: Vec<LeafKeyTweak>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        trace!("Claiming transfer with leaves: {:?}", leaves_to_claim);

        let claim_package = self
            .prepare_claim_package(transfer, &leaves_to_claim)
            .await?;

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .claim_transfer(operator_rpc::spark::ClaimTransferRequest {
                transfer_id: transfer.id.to_string(),
                owner_identity_public_key: self
                    .spark_signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
                claim_package: Some(claim_package),
            })
            .await?;

        let claimed_transfer: Transfer = response
            .transfer
            .ok_or_else(|| {
                ServiceError::Generic("claim_transfer returned no transfer".to_string())
            })?
            .try_into()?;

        Ok(claimed_transfer
            .leaves
            .into_iter()
            .map(|l| l.leaf)
            .collect())
    }

    /// Assembles a signed `ClaimPackage` for the coordinator: per-operator
    /// ECIES-encrypted key tweaks, user-signed refund jobs, and an
    /// identity-key signature over the package payload.
    async fn prepare_claim_package(
        &self,
        transfer: &Transfer,
        leaves: &[LeafKeyTweak],
    ) -> Result<operator_rpc::spark::ClaimPackage, ServiceError> {
        if leaves.is_empty() {
            return Err(ServiceError::NoLeavesToClaim);
        }
        let node_id_count: u32 = leaves
            .len()
            .try_into()
            .map_err(|_| ServiceError::InvalidInput("too many leaves to claim".to_string()))?;

        // Fetch operator signing commitments. The receiver does not yet own the
        // leaves, so request commitments by count rather than by node id.
        let signing_commitments = self
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(operator_rpc::spark::GetSigningCommitmentsRequest {
                node_ids: Vec::new(),
                count: 3,
                node_id_count,
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        let [cpfp_chunk, direct_chunk, direct_from_cpfp_chunk] =
            split_signing_commitments_by_variant(&signing_commitments, leaves.len())?;

        // Sign the claim refunds (current timelock) operator-commits-first via
        // sign_frost; the operators aggregate server-side during `claim_transfer`.
        let (cpfp_jobs, direct_jobs, direct_from_cpfp_jobs) = self
            .sign_claim_refunds(leaves, cpfp_chunk, direct_chunk, direct_from_cpfp_chunk)
            .await?;

        // Key-tweak step (decrypt incoming key, derive new key, compute tweak,
        // Feldman-split, ECIES per operator, sign the claim-package payload).
        let claim_leaves = leaves
            .iter()
            .map(|leaf| {
                let Some(cipher) = &leaf.incoming_key else {
                    return Err(ServiceError::InvalidInput(
                        "claim leaf must carry the encrypted incoming key".to_string(),
                    ));
                };
                let sender_signature = transfer
                    .leaves
                    .iter()
                    .find(|tl| tl.leaf.id == leaf.node.id)
                    .and_then(|tl| tl.signature)
                    .map(|s| s.serialize_compact().to_vec())
                    .unwrap_or_default();
                Ok(ClaimLeafInput {
                    node: leaf.node.clone(),
                    sender_signature,
                    leaf_key_ciphertext: cipher.as_slice().to_vec(),
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let prepared = self
            .spark_signer
            .prepare_claim(PrepareClaimRequest {
                transfer_id: transfer.id.clone(),
                sender_identity_public_key: transfer.sender_identity_public_key,
                leaves: claim_leaves,
                operator_recipients: self.operator_recipients(),
                threshold: self.split_secret_threshold,
            })
            .await?;

        let key_tweak_package: std::collections::BTreeMap<String, Vec<u8>> = prepared
            .operator_packages
            .into_iter()
            .map(|p| {
                (
                    hex::encode(p.operator_identifier.serialize()),
                    p.encrypted_package,
                )
            })
            .collect();

        // Claim-package user signature: identity-key ECDSA over the tagged
        // payload (tag || transfer_id || tweak map). Done here, not in the
        // signer, so signers stay free of claim-payload construction.
        let transfer_id_bytes = hex::decode(transfer.id.to_string().replace('-', ""))
            .map_err(|e| ServiceError::Generic(format!("invalid transfer id: {e}")))?;
        let signing_payload = TaggedHasher::new(&["spark", "claim", "signing payload"])
            .add_bytes(&transfer_id_bytes)
            .add_map_string_to_bytes(&key_tweak_package)
            .signable_message();
        let user_signature = self.spark_signer.sign_message(&signing_payload).await?;

        Ok(operator_rpc::spark::ClaimPackage {
            leaves_to_claim: cpfp_jobs,
            direct_leaves_to_claim: direct_jobs,
            direct_from_cpfp_leaves_to_claim: direct_from_cpfp_jobs,
            key_tweak_package: key_tweak_package.into_iter().collect(),
            user_signature: user_signature.serialize_der().to_vec(),
            hash_variant: HashVariant::V2.into(),
        })
    }

    /// Signs claim refund transactions operator-commits-first. The operator
    /// aggregates server-side during `claim_transfer`.
    async fn sign_claim_refunds(
        &self,
        leaves: &[LeafKeyTweak],
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
        let mut cpfp_jobs = Vec::new();
        let mut direct_jobs = Vec::new();
        let mut direct_from_cpfp_jobs = Vec::new();
        for (i, leaf) in leaves.iter().enumerate() {
            // The claim refund is signed with the receiver's new leaf key, which
            // is the derived key for this node id.
            let signing_public_key = self
                .spark_signer
                .get_public_key_for_leaf(&leaf.node.id)
                .await?;
            let verifying_key = leaf.node.verifying_public_key;
            let node_tx = &leaf.node.node_tx;

            // Build the claim refund transactions at the current (enforced)
            // timelock. The receiver receives the funds, so it is also the
            // receiving pubkey.
            let refund_tx = leaf
                .node
                .refund_tx
                .clone()
                .ok_or_else(|| ServiceError::Generic("No refund tx".to_string()))?;
            let old_sequence = refund_tx.input[0].sequence;
            let (cpfp_sequence, direct_sequence) = current_sequence(enforce_timelock(old_sequence));
            let RefundTransactions {
                cpfp_tx: cpfp_refund_tx,
                direct_tx: direct_refund_tx,
                direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
            } = create_refund_txs(
                node_tx,
                leaf.node.direct_refund_tx(),
                cpfp_sequence,
                direct_sequence,
                &signing_public_key,
                self.network,
            );

            let cpfp_sighash = sighash_from_tx(&cpfp_refund_tx, 0, &node_tx.output[0])?;
            cpfp_jobs.push(
                self.sign_claim_refund_job(
                    &leaf.node.id,
                    cpfp_refund_tx,
                    cpfp_sighash.as_byte_array(),
                    &signing_public_key,
                    cpfp_commitments[i].clone(),
                    &verifying_key,
                )
                .await?,
            );

            if let (Some(direct_tx), Some(direct_refund_tx)) =
                (leaf.node.direct_tx.as_ref(), direct_refund_tx)
            {
                let sighash = sighash_from_tx(&direct_refund_tx, 0, &direct_tx.output[0])?;
                direct_jobs.push(
                    self.sign_claim_refund_job(
                        &leaf.node.id,
                        direct_refund_tx,
                        sighash.as_byte_array(),
                        &signing_public_key,
                        direct_commitments[i].clone(),
                        &verifying_key,
                    )
                    .await?,
                );
            }

            if let Some(dfc_refund_tx) = direct_from_cpfp_refund_tx {
                let sighash = sighash_from_tx(&dfc_refund_tx, 0, &node_tx.output[0])?;
                direct_from_cpfp_jobs.push(
                    self.sign_claim_refund_job(
                        &leaf.node.id,
                        dfc_refund_tx,
                        sighash.as_byte_array(),
                        &signing_public_key,
                        direct_from_cpfp_commitments[i].clone(),
                        &verifying_key,
                    )
                    .await?,
                );
            }
        }

        Ok((cpfp_jobs, direct_jobs, direct_from_cpfp_jobs))
    }

    async fn sign_claim_refund_job(
        &self,
        node_id: &TreeNodeId,
        refund_tx: Transaction,
        sighash_bytes: &[u8; 32],
        signing_public_key: &PublicKey,
        operator_commitments: std::collections::BTreeMap<
            Identifier,
            frost_secp256k1_tr::round1::SigningCommitments,
        >,
        verifying_key: &PublicKey,
    ) -> Result<operator_rpc::spark::UserSignedTxSigningJob, ServiceError> {
        // The claim refund is signed with the receiver's new leaf key, which is
        // the derived key for this node id.
        let share = self
            .spark_signer
            .sign_frost(vec![FrostJob {
                derivation: FrostDerivation::SigningLeaf {
                    leaf_id: node_id.clone(),
                },
                sighash: *sighash_bytes,
                verifying_key: *verifying_key,
                operator_commitments: operator_commitments.clone(),
                adaptor_public_key: None,
            }])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| ServiceError::Generic("sign_frost returned no share".to_string()))?;

        let signed_tx = SignedTx {
            node_id: node_id.clone(),
            signing_public_key: *signing_public_key,
            tx: refund_tx,
            user_signature: share.signature_share,
            signing_commitments: operator_commitments,
            self_nonce_commitment: share.commitment,
            network: self.network,
        };
        (&signed_tx).try_into()
    }

    pub async fn verify_pending_transfer(
        &self,
        transfer: &Transfer,
    ) -> Result<HashMap<TreeNodeId, EncryptedSecret>, ServiceError> {
        let mut leaf_key_map = HashMap::new();
        let secp = bitcoin::secp256k1::Secp256k1::new();

        for transfer_leaf in &transfer.leaves {
            // Build the payload: leaf_id + transfer_id + secret_cipher
            let leaf_id_string = transfer_leaf.leaf.id.to_string();
            let transfer_id_string = transfer.id.to_string();

            let mut payload = Vec::new();
            payload.extend_from_slice(leaf_id_string.as_bytes());
            payload.extend_from_slice(transfer_id_string.as_bytes());
            payload.extend_from_slice(&transfer_leaf.secret_cipher);

            // Hash the payload
            let digest = sha256::Hash::hash(&payload);
            let message = bitcoin::secp256k1::Message::from_digest(digest.to_byte_array());

            let signature = match transfer_leaf.signature {
                Some(signature) => signature,
                None => {
                    return Err(ServiceError::Generic(
                        "Transfer leaf signature is missing".to_string(),
                    ));
                }
            };
            // Verify the signature (signature is already a Signature type in TransferLeaf)
            secp.verify_ecdsa(&message, &signature, &transfer.sender_identity_public_key)
                .map_err(|e| ServiceError::SignatureVerificationFailed(e.to_string()))?;

            // Decrypt the secret cipher and get the corresponding public key
            // The signer persists the private key internally and returns the public key
            let private_key = EncryptedSecret::new(transfer_leaf.secret_cipher.clone());

            leaf_key_map.insert(transfer_leaf.leaf.id.clone(), private_key);
        }

        Ok(leaf_key_map)
    }

    async fn query_transfers_inner(
        &self,
        transfer_ids: &[TransferId],
        paging: PagingFilter,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        trace!(
            "Querying transfers with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let order: crate::operator::rpc::spark::Order = paging.order.into();
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_transfers(TransferFilter {
                order: order.into(),
                transfer_ids: transfer_ids.iter().map(|id| id.to_string()).collect(),
                participant: Some(Participant::SenderOrReceiverIdentityPublicKey(
                    self.spark_signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
                )),
                network: self.network.to_proto_network() as i32,
                limit: paging.limit as i64,
                offset: paging.offset as i64,
                types: vec![
                    operator_rpc::spark::TransferType::Transfer.into(),
                    operator_rpc::spark::TransferType::PreimageSwap.into(),
                    operator_rpc::spark::TransferType::CooperativeExit.into(),
                    operator_rpc::spark::TransferType::UtxoSwap.into(),
                ],
                ..Default::default()
            })
            .await?;

        Ok(PagingResult {
            items: resp
                .transfers
                .into_iter()
                .map(|t| t.try_into())
                .collect::<Result<Vec<Transfer>, _>>()?,
            next: paging.next_from_offset(resp.offset),
        })
    }

    /// Queries transfers for the current identity
    pub async fn query_transfers(
        &self,
        transfer_ids: &[TransferId],
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        let transfers = match paging {
            Some(paging) => self.query_transfers_inner(transfer_ids, paging).await?,
            None => {
                pager(
                    |f| self.query_transfers_inner(transfer_ids, f),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(transfers)
    }

    async fn query_pending_transfers_inner(
        &self,
        paging: PagingFilter,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        trace!(
            "Querying pending transfers with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .query_pending_transfers(operator_rpc::spark::TransferFilter {
                participant: Some(Participant::SenderOrReceiverIdentityPublicKey(
                    self.spark_signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
                )),
                offset: paging.offset as i64,
                limit: paging.limit as i64,
                network: self.network.to_proto_network() as i32,
                ..Default::default()
            })
            .await?;

        Ok(PagingResult {
            items: resp
                .transfers
                .into_iter()
                .map(|t| t.try_into())
                .collect::<Result<Vec<Transfer>, _>>()?,
            next: paging.next_from_offset(resp.offset),
        })
    }

    /// Queries pending transfers from the operator
    pub async fn query_pending_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        let transfers = match paging {
            Some(paging) => self.query_pending_transfers_inner(paging).await?,
            None => {
                pager(
                    |f| self.query_pending_transfers_inner(f),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(transfers)
    }

    async fn query_claimable_receiver_transfers_inner(
        &self,
        paging: PagingFilter,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        trace!(
            "Querying pending (claimable) receiver transfers with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .query_pending_transfers(operator_rpc::spark::TransferFilter {
                network: self.network.to_proto_network() as i32,
                participant: Some(Participant::ReceiverIdentityPublicKey(
                    self.spark_signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
                )),
                offset: paging.offset as i64,
                limit: paging.limit as i64,
                ..Default::default()
            })
            .await?;

        Ok(PagingResult {
            items: resp
                .transfers
                .into_iter()
                .map(|t| t.try_into())
                .collect::<Result<Vec<Transfer>, _>>()?,
            next: paging.next_from_offset(resp.offset),
        })
    }

    /// Queries pending transfers from the operator
    pub async fn query_claimable_receiver_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        let transfers = match paging {
            Some(paging) => {
                self.query_claimable_receiver_transfers_inner(paging)
                    .await?
            }
            None => {
                pager(
                    |f| self.query_claimable_receiver_transfers_inner(f),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(transfers)
    }

    pub async fn query_transfer(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<Transfer>, ServiceError> {
        trace!("Querying transfer with id: {}", transfer_id);
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_transfers(TransferFilter {
                participant: Some(Participant::SenderOrReceiverIdentityPublicKey(
                    self.spark_signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
                )),
                transfer_ids: vec![transfer_id.to_string()],
                network: self.network.to_proto_network() as i32,
                ..Default::default()
            })
            .await?;

        match response.transfers.first() {
            Some(transfer) => {
                let transfer = transfer.clone().try_into()?;
                Ok(Some(transfer))
            }
            None => Ok(None),
        }
    }
}
