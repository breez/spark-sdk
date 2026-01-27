use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::Arc;

use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{PublicKey, ecdsa::Signature};
use bitcoin::{Sequence, Transaction};
use frost_secp256k1_tr::Identifier;
use frost_secp256k1_tr::round1::SigningCommitments;
use tracing::{info, trace};

use crate::core::{current_sequence, enforce_timelock, next_lightning_htlc_sequence};
use crate::services::{LeafRefundSigningData, ServiceError, SignedTx};
use crate::signer::{SignFrostRequest, Signer, SignerError};
use crate::tree::{TreeNode, TreeNodeId};
use crate::utils::frost::{SignAggregateFrostParams, sign_aggregate_frost};
use crate::utils::htlc_transactions::{
    CreateLightningHtlcRefundTxsParams, create_lightning_htlc_refund_txs,
};
use crate::utils::transactions::{RefundTransactions, create_refund_txs};
use crate::{Network, bitcoin::sighash_from_tx, core::next_sequence, services::LeafKeyTweak};

#[derive(Clone, Debug, Default)]
pub struct RefundSignatures {
    pub cpfp_signatures: HashMap<TreeNodeId, Signature>,
    pub direct_signatures: HashMap<TreeNodeId, Signature>,
    pub direct_from_cpfp_signatures: HashMap<TreeNodeId, Signature>,
}

pub struct RefundTxConstructor<'a> {
    pub node: &'a TreeNode,
    pub vout: u32,
    pub cpfp_sequence: Sequence,
    pub direct_sequence: Sequence,
    pub receiving_pubkey: &'a PublicKey,
}

pub struct SignRefundsParams<'a> {
    pub signer: &'a Arc<dyn Signer>,
    pub leaves: &'a [LeafKeyTweak],
    pub cpfp_signing_commitments: Vec<BTreeMap<Identifier, SigningCommitments>>,
    pub direct_signing_commitments: Vec<BTreeMap<Identifier, SigningCommitments>>,
    pub direct_from_cpfp_signing_commitments: Vec<BTreeMap<Identifier, SigningCommitments>>,
    pub receiver_pubkey: &'a PublicKey,
    pub payment_hash: Option<&'a sha256::Hash>,
    pub network: Network,
    /// Optional adaptor public key for creating adaptor signatures (used in swap v3)
    pub cpfp_adaptor_public_key: Option<&'a PublicKey>,
}

pub struct SignedRefundTransactions {
    pub cpfp_signed_tx: Vec<SignedTx>,
    pub direct_signed_tx: Vec<SignedTx>,
    pub direct_from_cpfp_signed_tx: Vec<SignedTx>,
}

pub async fn prepare_leaf_refund_signing_data(
    signer: &Arc<dyn Signer>,
    leaf_key_tweaks: &[LeafKeyTweak],
    receiving_public_key: PublicKey,
) -> Result<HashMap<TreeNodeId, LeafRefundSigningData>, SignerError> {
    let mut leaf_data_map = HashMap::new();
    for leaf_key in leaf_key_tweaks.iter() {
        let signing_nonce_commitment = signer.generate_random_signing_commitment().await?;
        let direct_signing_nonce_commitment = signer.generate_random_signing_commitment().await?;
        let direct_from_cpfp_signing_nonce_commitment =
            signer.generate_random_signing_commitment().await?;

        leaf_data_map.insert(
            leaf_key.node.id.clone(),
            LeafRefundSigningData {
                signing_public_key: signer.public_key_from_secret(&leaf_key.signing_key).await?,
                signing_private_key: leaf_key.signing_key.clone(),
                receiving_public_key,
                tx: leaf_key.node.node_tx.clone(),
                direct_tx: leaf_key.node.direct_tx.clone(),
                refund_tx: leaf_key.node.refund_tx.clone(),
                direct_refund_tx: leaf_key.node.direct_refund_tx.clone(),
                direct_from_cpfp_refund_tx: leaf_key.node.direct_from_cpfp_refund_tx.clone(),
                signing_nonce_commitment,
                direct_signing_nonce_commitment,
                direct_from_cpfp_signing_nonce_commitment,
                vout: leaf_key.node.vout,
            },
        );
    }

    Ok(leaf_data_map)
}

pub async fn sign_refunds(
    params: SignRefundsParams<'_>,
) -> Result<SignedRefundTransactions, SignerError> {
    let SignRefundsParams {
        signer,
        leaves,
        cpfp_signing_commitments,
        direct_signing_commitments,
        direct_from_cpfp_signing_commitments,
        receiver_pubkey,
        payment_hash,
        network,
        cpfp_adaptor_public_key,
    } = params;
    let identity_pubkey = signer.get_identity_public_key().await?;

    let mut cpfp_signed_refunds = Vec::with_capacity(leaves.len());
    let mut direct_signed_refunds = Vec::with_capacity(leaves.len());
    let mut direct_from_cpfp_signed_refunds = Vec::with_capacity(leaves.len());

    for (i, leaf) in leaves.iter().enumerate() {
        let node_tx = &leaf.node.node_tx;
        let direct_tx = leaf.node.direct_tx.as_ref();

        let old_sequence = leaf
            .node
            .refund_tx
            .as_ref()
            .ok_or(SignerError::Generic("No refund transaction".to_string()))?
            .input[0]
            .sequence;

        let RefundTransactions {
            cpfp_tx: cpfp_refund_tx,
            direct_tx: direct_refund_tx,
            direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
        } = match payment_hash {
            Some(payment_hash) => {
                let (cpfp_sequence, direct_sequence) = next_lightning_htlc_sequence(old_sequence)
                    .ok_or(SignerError::Generic(
                    "Failed to get next lightning HTLC sequences".to_string(),
                ))?;
                create_lightning_htlc_refund_txs(CreateLightningHtlcRefundTxsParams {
                    node_tx,
                    direct_tx,
                    cpfp_sequence,
                    direct_sequence,
                    hash: payment_hash,
                    hash_lock_pubkey: receiver_pubkey,
                    sequence_lock_pubkey: &identity_pubkey,
                    network,
                })?
            }
            None => {
                let (cpfp_sequence, direct_sequence) = next_sequence(old_sequence).ok_or(
                    SignerError::Generic("Failed to get next sequence".to_string()),
                )?;
                create_refund_txs(
                    node_tx,
                    direct_tx,
                    cpfp_sequence,
                    direct_sequence,
                    receiver_pubkey,
                    network,
                )
            }
        };

        info!(
            "sign_refunds for leaf {}: Current sequence: {old_sequence}, next sequence: {}",
            leaf.node.id, cpfp_refund_tx.input[0].sequence
        );

        let signing_public_key = signer.public_key_from_secret(&leaf.signing_key).await?;

        let cpfp_signed_tx = sign_refund(
            signer,
            leaf,
            node_tx,
            cpfp_refund_tx,
            signing_public_key,
            cpfp_signing_commitments[i].clone(),
            network,
            cpfp_adaptor_public_key,
        )
        .await?;
        cpfp_signed_refunds.push(cpfp_signed_tx);

        if let Some(direct_tx) = direct_tx {
            let Some(direct_refund_tx) = direct_refund_tx else {
                return Err(SignerError::Generic(
                    "Direct refund transaction is missing".to_string(),
                ));
            };
            let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx else {
                return Err(SignerError::Generic(
                    "Direct from CPFP refund transaction is missing".to_string(),
                ));
            };

            let direct_refund_tx = sign_refund(
                signer,
                leaf,
                direct_tx,
                direct_refund_tx,
                signing_public_key,
                direct_signing_commitments[i].clone(),
                network,
                None, // Direct transactions don't use adaptor signatures
            )
            .await?;
            direct_signed_refunds.push(direct_refund_tx);

            let direct_from_cpfp_signed_tx = sign_refund(
                signer,
                leaf,
                node_tx,
                direct_from_cpfp_refund_tx,
                signing_public_key,
                direct_from_cpfp_signing_commitments[i].clone(),
                network,
                None, // Direct transactions don't use adaptor signatures
            )
            .await?;
            direct_from_cpfp_signed_refunds.push(direct_from_cpfp_signed_tx);
        }
    }

    Ok(SignedRefundTransactions {
        cpfp_signed_tx: cpfp_signed_refunds,
        direct_signed_tx: direct_signed_refunds,
        direct_from_cpfp_signed_tx: direct_from_cpfp_signed_refunds,
    })
}

/// Signs a refund transaction using FROST threshold signatures.
///
/// This function performs the client-side portion of the FROST signing protocol for a refund transaction:
/// 1. Calculates the transaction sighash
/// 2. Generates new nonce commitments for signing
/// 3. Signs the transaction using the FROST protocol
/// 4. Returns a structured `SignedTx` object with all data needed for later aggregation
///
/// The function does not perform signature aggregation - it only creates the user's signature share.
/// Aggregation happens later when combined with operator signatures.
///
/// # Arguments
///
/// * `signer` - Reference to the signer implementation
/// * `leaf` - The leaf key data containing node info and signing key
/// * `tx` - The original transaction being spent by the refund transaction
/// * `refund_tx` - The refund transaction to sign
/// * `signing_public_key` - The public key corresponding to the user's signing key
/// * `spark_commitments` - The FROST signing commitments from the Spark operators
/// * `network` - The Bitcoin network being used
///
/// # Returns
///
/// * `Ok(SignedTx)` - A structure containing the signed transaction and signing metadata
/// * `Err(SignerError)` - If the signing process fails
#[allow(clippy::too_many_arguments)]
async fn sign_refund(
    signer: &Arc<dyn Signer>,
    leaf: &LeafKeyTweak,
    tx: &Transaction,
    refund_tx: Transaction,
    signing_public_key: PublicKey,
    spark_commitments: BTreeMap<Identifier, SigningCommitments>,
    network: Network,
    adaptor_public_key: Option<&PublicKey>,
) -> Result<SignedTx, SignerError> {
    let sighash = sighash_from_tx(&refund_tx, 0, &tx.output[0])
        .map_err(|e| SignerError::Generic(e.to_string()))?;
    let self_nonce_commitment = signer.generate_random_signing_commitment().await?;
    let user_signature = signer
        .sign_frost(SignFrostRequest {
            message: sighash.to_raw_hash().to_byte_array().as_ref(),
            public_key: &signing_public_key,
            private_key: &leaf.signing_key,
            verifying_key: &leaf.node.verifying_public_key,
            self_nonce_commitment: &self_nonce_commitment,
            statechain_commitments: spark_commitments.clone(),
            adaptor_public_key,
        })
        .await?;

    Ok(SignedTx {
        node_id: leaf.node.id.clone(),
        signing_public_key,
        tx: refund_tx,
        user_signature,
        self_nonce_commitment,
        signing_commitments: spark_commitments,
        network,
    })
}

/// Signs refund transactions using FROST threshold signatures
pub async fn sign_aggregate_refunds(
    signer: &Arc<dyn Signer>,
    leaf_data_map: &HashMap<TreeNodeId, LeafRefundSigningData>,
    operator_signing_results: &[crate::operator::rpc::spark::LeafRefundTxSigningResult],
    cpfp_adaptor_pubkey: Option<&PublicKey>,
    direct_adaptor_pubkey: Option<&PublicKey>,
    direct_from_cpfp_adaptor_pubkey: Option<&PublicKey>,
) -> Result<Vec<crate::operator::rpc::spark::NodeSignatures>, ServiceError> {
    let mut node_signatures = Vec::new();

    for operator_signing_result in operator_signing_results {
        let leaf_id = TreeNodeId::from_str(&operator_signing_result.leaf_id)
            .map_err(ServiceError::ValidationError)?;

        let leaf_data = leaf_data_map.get(&leaf_id).ok_or_else(|| {
            ServiceError::Generic(format!(
                "Leaf data not found for leaf {}",
                operator_signing_result.leaf_id
            ))
        })?;

        let tx = &leaf_data.tx;
        let refund_tx = leaf_data
            .refund_tx
            .as_ref()
            .ok_or_else(|| ServiceError::Generic("Missing refund transaction".to_string()))?;

        let verifying_key = PublicKey::from_slice(&operator_signing_result.verifying_key)
            .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;

        let refund_tx_signing_result = operator_signing_result
            .refund_tx_signing_result
            .as_ref()
            .map(|sr| sr.try_into())
            .transpose()?
            .ok_or(ServiceError::ValidationError(
                "Missing refund tx signing result".to_string(),
            ))?;

        let refund_tx_signature = sign_aggregate_frost(SignAggregateFrostParams {
            signer,
            tx: refund_tx,
            prev_out: &tx.output[0],
            signing_public_key: &leaf_data.signing_public_key,
            aggregating_public_key: &leaf_data.signing_public_key,
            signing_private_key: &leaf_data.signing_private_key,
            self_nonce_commitment: &leaf_data.signing_nonce_commitment,
            adaptor_public_key: cpfp_adaptor_pubkey,
            verifying_key: &verifying_key,
            signing_result: refund_tx_signing_result,
        })
        .await?;

        let mut direct_refund_tx_signature = Vec::new();
        let mut direct_from_cpfp_refund_tx_signature = Vec::new();

        if let Some(direct_tx) = &leaf_data.direct_tx {
            if let Some(direct_refund_tx) = &leaf_data.direct_refund_tx {
                trace!("Signing direct refund tx for leaf");
                let direct_refund_tx_signing_result = operator_signing_result
                    .direct_refund_tx_signing_result
                    .as_ref()
                    .map(|sr| sr.try_into())
                    .transpose()?
                    .ok_or(ServiceError::ValidationError(
                        "Missing direct refund tx signing result".to_string(),
                    ))?;
                let signature = sign_aggregate_frost(SignAggregateFrostParams {
                    signer,
                    tx: direct_refund_tx,
                    prev_out: &direct_tx.output[0],
                    signing_public_key: &leaf_data.signing_public_key,
                    aggregating_public_key: &leaf_data.signing_public_key,
                    signing_private_key: &leaf_data.signing_private_key,
                    self_nonce_commitment: &leaf_data.direct_signing_nonce_commitment,
                    adaptor_public_key: direct_adaptor_pubkey,
                    verifying_key: &verifying_key,
                    signing_result: direct_refund_tx_signing_result,
                })
                .await?;
                direct_refund_tx_signature = signature.serialize()?.to_vec();
            }

            if let Some(direct_from_cpfp_refund_tx) = &leaf_data.direct_from_cpfp_refund_tx {
                trace!("Signing direct from CPFP refund tx for leaf");
                let direct_from_cpfp_refund_tx_signing_result = operator_signing_result
                    .direct_from_cpfp_refund_tx_signing_result
                    .as_ref()
                    .map(|sr| sr.try_into())
                    .transpose()?
                    .ok_or(ServiceError::ValidationError(
                        "Missing direct from CPFP refund tx signing result".to_string(),
                    ))?;
                let signature = sign_aggregate_frost(SignAggregateFrostParams {
                    signer,
                    tx: direct_from_cpfp_refund_tx,
                    prev_out: &tx.output[0],
                    signing_public_key: &leaf_data.signing_public_key,
                    aggregating_public_key: &leaf_data.signing_public_key,
                    signing_private_key: &leaf_data.signing_private_key,
                    self_nonce_commitment: &leaf_data.direct_from_cpfp_signing_nonce_commitment,
                    adaptor_public_key: direct_from_cpfp_adaptor_pubkey,
                    verifying_key: &verifying_key,
                    signing_result: direct_from_cpfp_refund_tx_signing_result,
                })
                .await?;
                direct_from_cpfp_refund_tx_signature = signature.serialize()?.to_vec();
            }
        }

        node_signatures.push(crate::operator::rpc::spark::NodeSignatures {
            node_id: operator_signing_result.leaf_id.clone(),
            node_tx_signature: Vec::new(),
            refund_tx_signature: refund_tx_signature.serialize()?.to_vec(),
            direct_node_tx_signature: Vec::new(),
            direct_refund_tx_signature,
            direct_from_cpfp_refund_tx_signature,
        });
    }

    Ok(node_signatures)
}

/// Prepares refund signing jobs for claim operations
pub fn prepare_refund_so_signing_jobs(
    network: Network,
    leaves: &[LeafKeyTweak],
    leaf_data_map: &mut HashMap<TreeNodeId, LeafRefundSigningData>,
    is_for_claim: bool,
) -> Result<Vec<crate::operator::rpc::spark::LeafRefundTxSigningJob>, ServiceError> {
    prepare_refund_so_signing_jobs_with_tx_constructor(
        leaves,
        leaf_data_map,
        is_for_claim,
        |refund_tx_constructor| {
            let RefundTxConstructor {
                node,
                cpfp_sequence,
                direct_sequence,
                receiving_pubkey,
                ..
            } = refund_tx_constructor;
            create_refund_txs(
                &node.node_tx,
                node.direct_tx.as_ref(),
                cpfp_sequence,
                direct_sequence,
                receiving_pubkey,
                network,
            )
        },
    )
}

/// Prepares refund signing jobs for claim operations with a custom transaction constructor
pub fn prepare_refund_so_signing_jobs_with_tx_constructor<F>(
    leaves: &[LeafKeyTweak],
    leaf_data_map: &mut HashMap<TreeNodeId, LeafRefundSigningData>,
    is_for_claim: bool,
    refund_tx_constructor: F,
) -> Result<Vec<crate::operator::rpc::spark::LeafRefundTxSigningJob>, ServiceError>
where
    F: Fn(RefundTxConstructor) -> RefundTransactions,
{
    let mut signing_jobs = Vec::new();

    for (i, leaf) in leaves.iter().enumerate() {
        let refund_signing_data: &mut LeafRefundSigningData =
            leaf_data_map.get_mut(&leaf.node.id).ok_or_else(|| {
                ServiceError::Generic(format!("Leaf data not found for leaf {}", leaf.node.id))
            })?;

        let refund_tx = leaf
            .node
            .refund_tx
            .clone()
            .ok_or(ServiceError::Generic("No refund tx".to_string()))?;
        let old_sequence = refund_tx.input[0].sequence;
        let (cpfp_sequence, direct_sequence) = if is_for_claim {
            let enforced = enforce_timelock(old_sequence);
            current_sequence(enforced)
        } else {
            next_sequence(old_sequence).ok_or(ServiceError::Generic(
                "Failed to get next sequence".to_string(),
            ))?
        };

        let RefundTransactions {
            cpfp_tx: cpfp_refund_tx,
            direct_tx: direct_refund_tx,
            direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
        } = refund_tx_constructor(RefundTxConstructor {
            node: &leaf.node,
            vout: i as u32,
            cpfp_sequence,
            direct_sequence,
            receiving_pubkey: &refund_signing_data.receiving_public_key,
        });

        info!(
            "prepare_refund_so_signing_jobs_with_tx_constructor for leaf {}: Current sequence: {old_sequence}, next sequence: {}",
            leaf.node.id, cpfp_refund_tx.input[0].sequence
        );

        let direct_refund_tx_signing_job = if let Some(direct_refund_tx) = &direct_refund_tx {
            Some(crate::operator::rpc::spark::SigningJob {
                signing_public_key: refund_signing_data.signing_public_key.serialize().to_vec(),
                raw_tx: bitcoin::consensus::serialize(direct_refund_tx),
                signing_nonce_commitment: Some(
                    refund_signing_data
                        .direct_signing_nonce_commitment
                        .commitments
                        .try_into()?,
                ),
            })
        } else {
            None
        };
        let direct_from_cpfp_refund_tx_signing_job =
            if let Some(direct_from_cpfp_refund_tx) = &direct_from_cpfp_refund_tx {
                Some(crate::operator::rpc::spark::SigningJob {
                    signing_public_key: refund_signing_data.signing_public_key.serialize().to_vec(),
                    raw_tx: bitcoin::consensus::serialize(direct_from_cpfp_refund_tx),
                    signing_nonce_commitment: Some(
                        refund_signing_data
                            .direct_from_cpfp_signing_nonce_commitment
                            .commitments
                            .try_into()?,
                    ),
                })
            } else {
                None
            };

        let signing_job = crate::operator::rpc::spark::LeafRefundTxSigningJob {
            leaf_id: leaf.node.id.to_string(),
            refund_tx_signing_job: Some(crate::operator::rpc::spark::SigningJob {
                signing_public_key: refund_signing_data.signing_public_key.serialize().to_vec(),
                raw_tx: bitcoin::consensus::serialize(&cpfp_refund_tx),
                signing_nonce_commitment: Some(
                    refund_signing_data
                        .signing_nonce_commitment
                        .commitments
                        .try_into()?,
                ),
            }),
            direct_refund_tx_signing_job,
            direct_from_cpfp_refund_tx_signing_job,
        };

        refund_signing_data.refund_tx = Some(cpfp_refund_tx);
        refund_signing_data.direct_refund_tx = direct_refund_tx;
        refund_signing_data.direct_from_cpfp_refund_tx = direct_from_cpfp_refund_tx;

        signing_jobs.push(signing_job);
    }

    Ok(signing_jobs)
}

/// Converts operator-provided node signatures into a structured `RefundSignatures` object.
///
/// This function processes an array of `NodeSignatures` (typically from operators) and maps them
/// into HashMaps keyed by `TreeNodeId` for easier access. It handles three types of signatures:
///
/// 1. CPFP refund transaction signatures (always present)
/// 2. Direct refund transaction signatures (optional)
/// 3. Direct-from-CPFP refund transaction signatures (optional)
///
/// The function validates each signature by attempting to parse it from its compact representation.
///
/// # Arguments
///
/// * `node_signatures` - A vector of `NodeSignatures` from operators containing serialized signatures
///
/// # Returns
///
/// * `Ok(RefundSignatures)` - A structured object containing HashMaps of signatures organized by type and node ID
/// * `Err(ServiceError)` - If any node ID or signature fails validation
pub fn map_refund_signatures(
    node_signatures: Vec<crate::operator::rpc::spark::NodeSignatures>,
) -> Result<RefundSignatures, ServiceError> {
    let mut cpfp_signatures = HashMap::new();
    let mut direct_signatures = HashMap::new();
    let mut direct_from_cpfp_signatures = HashMap::new();

    for ns in node_signatures {
        let node_id: TreeNodeId = match ns.node_id.parse() {
            Ok(id) => id,
            Err(_) => return Err(ServiceError::Generic("invalid node_id".to_string())),
        };

        cpfp_signatures.insert(
            node_id.clone(),
            Signature::from_compact(&ns.refund_tx_signature)
                .map_err(|_| ServiceError::Generic("invalid refund tx signature".to_string()))?,
        );

        if !ns.direct_refund_tx_signature.is_empty() {
            direct_signatures.insert(
                node_id.clone(),
                Signature::from_compact(&ns.direct_refund_tx_signature).map_err(|_| {
                    ServiceError::Generic("invalid direct refund tx signature".to_string())
                })?,
            );
        }

        if !ns.direct_from_cpfp_refund_tx_signature.is_empty() {
            direct_from_cpfp_signatures.insert(
                node_id,
                Signature::from_compact(&ns.direct_from_cpfp_refund_tx_signature).map_err(
                    |_| {
                        ServiceError::Generic(
                            "invalid direct from CPFP refund tx signature".to_string(),
                        )
                    },
                )?,
            );
        }
    }

    Ok(RefundSignatures {
        cpfp_signatures,
        direct_signatures,
        direct_from_cpfp_signatures,
    })
}
