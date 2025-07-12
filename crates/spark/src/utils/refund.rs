use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use crate::services::{
    LeafRefundSigningData, ServiceError, map_public_keys, map_signature_shares,
    map_signing_nonce_commitments,
};
use crate::tree::TreeNodeId;
use crate::utils::anchor::ephemeral_anchor_output;
use crate::{Network, bitcoin::sighash_from_tx, core::next_sequence, services::LeafKeyTweak};
use bitcoin::absolute::LockTime;
use bitcoin::blockdata::transaction::Version;
use bitcoin::hashes::Hash;
use bitcoin::{OutPoint, Sequence, Transaction, TxIn, TxOut};
use bitcoin::{key::Secp256k1, secp256k1::PublicKey};
use frost_core::round2::SignatureShare;
use frost_secp256k1_tr::round1::SigningCommitments;

use frost_secp256k1_tr::{Identifier, Secp256K1Sha256TR};

use crate::signer::{AggregateFrostRequest, SignerError};
use crate::signer::{SignFrostRequest, Signer};

pub struct SignedTx {
    pub node_id: TreeNodeId,
    pub signing_public_key: PublicKey,
    pub tx: Transaction,
    pub user_signature: SignatureShare<Secp256K1Sha256TR>,
    pub signing_commitments: BTreeMap<Identifier, SigningCommitments>,
    pub user_signature_commitment: SigningCommitments,
    pub network: Network,
}

pub fn create_refund_tx(
    sequence: Sequence,
    node_outpoint: OutPoint,
    amount_sat: u64,
    receiving_pubkey: &PublicKey,
    network: Network,
) -> Result<Transaction, SignerError> {
    // TODO: Isolate secp256k1 initialization to avoid multiple initializations
    let secp = Secp256k1::new();
    let network: bitcoin::Network = network.into();
    let addr = bitcoin::Address::p2tr(&secp, receiving_pubkey.x_only_public_key().0, None, network);

    let new_refund_tx = Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: node_outpoint,
            script_sig: bitcoin::ScriptBuf::default(),
            sequence,
            witness: bitcoin::Witness::default(),
        }],
        output: vec![
            TxOut {
                value: bitcoin::Amount::from_sat(amount_sat),
                script_pubkey: addr.script_pubkey(),
            },
            ephemeral_anchor_output(),
        ],
    };

    Ok(new_refund_tx)
}

pub async fn sign_refunds<S: Signer>(
    signer: &S,
    leaves: &[LeafKeyTweak],
    spark_commitments: Vec<BTreeMap<Identifier, SigningCommitments>>,
    receiver_pubkey: &PublicKey,
    network: Network,
) -> Result<Vec<SignedTx>, SignerError> {
    // sign refunds. TODO: In JS SDK, this is the `sign_refunds` function
    let mut signed_refunds = Vec::with_capacity(leaves.len());

    for (i, leaf) in leaves.iter().enumerate() {
        let node_tx = leaf.node.node_tx.clone();

        let old_sequence = leaf
            .node
            .refund_tx
            .as_ref()
            .ok_or(SignerError::Generic("No refund transaction".to_string()))?
            .input[0]
            .sequence;
        let sequence = next_sequence(old_sequence).ok_or(SignerError::Generic(
            "Failed to get next sequence".to_string(),
        ))?;

        let new_refund_tx = create_refund_tx(
            sequence,
            OutPoint {
                txid: node_tx.compute_txid(),
                vout: 0,
            },
            leaf.node.value,
            receiver_pubkey,
            network,
        )?;

        let sighash = sighash_from_tx(&new_refund_tx, 0, &node_tx.output[0])
            .map_err(|e| SignerError::Generic(e.to_string()))?;

        let self_commitment = signer.generate_frost_signing_commitments().await?;
        let spark_commitment = spark_commitments[i].clone();

        let signing_public_key =
            signer.get_public_key_from_private_key_source(&leaf.signing_key)?;

        let user_signature_share = signer
            .sign_frost(SignFrostRequest {
                message: sighash.to_raw_hash().to_byte_array().as_ref(),
                public_key: &signing_public_key,
                private_key: &leaf.signing_key,
                verifying_key: &leaf.node.verifying_public_key,
                self_commitment: &self_commitment,
                statechain_commitments: spark_commitment.clone(),
                adaptor_public_key: None,
            })
            .await?;

        signed_refunds.push(SignedTx {
            node_id: leaf.node.id.clone(),
            signing_public_key,
            tx: new_refund_tx,
            user_signature: user_signature_share,
            user_signature_commitment: self_commitment,
            signing_commitments: spark_commitment,
            network,
        });
    }

    Ok(signed_refunds)
}

/// Signs refund transactions using FROST threshold signatures
pub async fn sign_aggregate_refunds<S: Signer>(
    signer: &S,
    leaf_data_map: &HashMap<TreeNodeId, LeafRefundSigningData>,
    operator_signing_results: &[crate::operator::rpc::spark::LeafRefundTxSigningResult],
    adaptor_pubkey: Option<&PublicKey>,
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

        let refund_tx_signing_result = operator_signing_result
            .refund_tx_signing_result
            .as_ref()
            .ok_or_else(|| {
                ServiceError::ValidationError("Missing refund tx signing result".to_string())
            })?;

        let refund_tx = leaf_data
            .refund_tx
            .as_ref()
            .ok_or_else(|| ServiceError::Generic("Missing refund transaction".to_string()))?;

        let refund_tx_sighash = sighash_from_tx(refund_tx, 0, &leaf_data.tx.output[0])?;

        // Map operator signing commitments and signature shares
        let signing_nonce_commitments = map_signing_nonce_commitments(
            refund_tx_signing_result.signing_nonce_commitments.clone(),
        )?;
        let signature_shares =
            map_signature_shares(refund_tx_signing_result.signature_shares.clone())?;
        let public_keys = map_public_keys(refund_tx_signing_result.public_keys.clone())?;

        let verifying_key = PublicKey::from_slice(&operator_signing_result.verifying_key)
            .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;

        // Sign with FROST
        let user_signature = signer
            .sign_frost(SignFrostRequest {
                message: refund_tx_sighash.as_byte_array(),
                public_key: &leaf_data.signing_public_key,
                private_key: &leaf_data.signing_private_key,
                verifying_key: &verifying_key,
                self_commitment: &leaf_data.signing_nonce_commitment,
                statechain_commitments: signing_nonce_commitments.clone(),
                adaptor_public_key: adaptor_pubkey,
            })
            .await?;

        // Aggregate FROST signatures
        let refund_aggregate = signer
            .aggregate_frost(AggregateFrostRequest {
                message: refund_tx_sighash.as_byte_array(),
                statechain_signatures: signature_shares,
                statechain_public_keys: public_keys,
                verifying_key: &verifying_key,
                statechain_commitments: signing_nonce_commitments,
                self_commitment: &leaf_data.signing_nonce_commitment,
                public_key: &leaf_data.signing_public_key,
                self_signature: &user_signature,
                adaptor_public_key: adaptor_pubkey,
            })
            .await?;

        node_signatures.push(crate::operator::rpc::spark::NodeSignatures {
            node_id: operator_signing_result.leaf_id.clone(),
            refund_tx_signature: refund_aggregate.serialize()?.to_vec(),
            node_tx_signature: Vec::new(),
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
    let mut signing_jobs = Vec::new();

    for leaf in leaves {
        let refund_signing_data: &mut LeafRefundSigningData =
            leaf_data_map.get_mut(&leaf.node.id).ok_or_else(|| {
                ServiceError::Generic(format!("Leaf data not found for leaf {}", leaf.node.id))
            })?;

        let old_sequence = leaf
            .node
            .refund_tx
            .as_ref()
            .ok_or(ServiceError::Generic("No refund transaction".to_string()))?
            .input[0]
            .sequence;
        let sequence = if is_for_claim {
            old_sequence // TODO: is this correct?
        } else {
            next_sequence(old_sequence).ok_or(ServiceError::Generic(
                "Failed to get next sequence".to_string(),
            ))?
        };

        let refund_tx = create_refund_tx(
            sequence,
            bitcoin::OutPoint {
                txid: leaf.node.node_tx.compute_txid(),
                vout: 0,
            },
            leaf.node.value,
            &refund_signing_data.receiving_public_key,
            network,
        )?;

        let signing_job = crate::operator::rpc::spark::LeafRefundTxSigningJob {
            leaf_id: leaf.node.id.to_string(),
            refund_tx_signing_job: Some(crate::operator::rpc::spark::SigningJob {
                signing_public_key: refund_signing_data.signing_public_key.serialize().to_vec(),
                raw_tx: bitcoin::consensus::serialize(&refund_tx),
                signing_nonce_commitment: Some(
                    refund_signing_data.signing_nonce_commitment.try_into()?,
                ),
            }),
        };

        refund_signing_data.refund_tx = Some(refund_tx);

        signing_jobs.push(signing_job);
    }

    Ok(signing_jobs)
}
