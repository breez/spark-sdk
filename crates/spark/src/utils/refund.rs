use std::collections::{BTreeMap, HashMap};

use crate::services::{LeafRefundSigningData, ServiceError};
use crate::tree::TreeNodeId;
use crate::utils::anchor::ephemeral_anchor_output;
use crate::{Network, bitcoin::sighash_from_tx, core::next_sequence, services::LeafKeyTweak};
use bitcoin::absolute::LockTime;
use bitcoin::blockdata::transaction::Version;
use bitcoin::hashes::Hash;
use bitcoin::{OutPoint, Sequence, Transaction};
use bitcoin::{key::Secp256k1, secp256k1::PublicKey};
use frost_core::round2::SignatureShare;
use frost_secp256k1_tr::round1::SigningCommitments;

use frost_secp256k1_tr::{Identifier, Secp256K1Sha256TR};

use crate::signer::SignerError;
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
) -> Result<bitcoin::Transaction, SignerError> {
    let mut new_refund_tx = bitcoin::Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![],
        output: vec![],
    };

    new_refund_tx.input.push(bitcoin::TxIn {
        previous_output: node_outpoint,
        script_sig: bitcoin::ScriptBuf::default(),
        sequence,
        witness: bitcoin::Witness::default(),
    });

    let secp = Secp256k1::new();
    let network: bitcoin::Network = network.into();
    let addr = bitcoin::Address::p2tr(&secp, receiving_pubkey.x_only_public_key().0, None, network);

    new_refund_tx.output.push(bitcoin::TxOut {
        value: bitcoin::Amount::from_sat(amount_sat),
        script_pubkey: addr.script_pubkey(),
    });
    new_refund_tx.output.push(ephemeral_anchor_output());

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
