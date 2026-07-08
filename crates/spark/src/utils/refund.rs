use std::collections::BTreeMap;
use std::sync::Arc;

use bitcoin::Transaction;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{PublicKey, Secp256k1};
use frost_secp256k1_tr::Identifier;
use frost_secp256k1_tr::round1::SigningCommitments;
use tracing::info;

use crate::core::next_lightning_htlc_sequence;
use crate::services::{
    LeafRefundJobs, RefundJob, SignedTx, build_refund_signing_job, into_signed_tx_groups,
    sign_leaf_refunds,
};
use crate::signer::{SignerError, SparkSigner};
use crate::utils::frost::derive_leaf_signing_public_key;
use crate::utils::htlc_transactions::{
    CreateLightningHtlcRefundTxsParams, create_lightning_htlc_refund_txs,
};
use crate::utils::transactions::{RefundTransactions, create_refund_txs};
use crate::{Network, bitcoin::sighash_from_tx, core::next_sequence, services::LeafKeyTweak};

pub struct SignRefundsParams<'a> {
    pub spark_signer: &'a Arc<dyn SparkSigner>,
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

pub async fn sign_refunds(
    params: SignRefundsParams<'_>,
) -> Result<SignedRefundTransactions, SignerError> {
    let SignRefundsParams {
        spark_signer,
        leaves,
        cpfp_signing_commitments,
        direct_signing_commitments,
        direct_from_cpfp_signing_commitments,
        receiver_pubkey,
        payment_hash,
        network,
        cpfp_adaptor_public_key,
    } = params;
    let identity_pubkey = spark_signer.get_identity_public_key().await?;

    // Build every FROST job up front (all local work), tagging each with what's
    // needed to rebuild its SignedTx once the batched shares return. A leaf
    // contributes up to three jobs: cpfp, direct, direct-from-cpfp.
    let mut leaf_jobs: Vec<LeafRefundJobs> = Vec::new();
    let secp = Secp256k1::new();

    for (i, leaf) in leaves.iter().enumerate() {
        let node_tx = &leaf.node.node_tx;
        let direct_tx = leaf.node.direct_refund_tx();

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

        let signing_public_key = derive_leaf_signing_public_key(&leaf.node, &secp)?;

        let cpfp = build_refund_job(
            leaf,
            node_tx,
            cpfp_refund_tx,
            signing_public_key,
            cpfp_signing_commitments[i].clone(),
            network,
            cpfp_adaptor_public_key,
        )?;

        let direct = if let Some(direct_tx) = direct_tx {
            let Some(direct_refund_tx) = direct_refund_tx else {
                return Err(SignerError::Generic(
                    "Direct refund transaction is missing".to_string(),
                ));
            };

            Some(build_refund_job(
                leaf,
                direct_tx,
                direct_refund_tx,
                signing_public_key,
                direct_signing_commitments[i].clone(),
                network,
                None, // Direct transactions don't use adaptor signatures
            )?)
        } else {
            None
        };

        // direct_from_cpfp_refund_tx spends from the CPFP (node_tx) output, not from
        // direct_tx, so it must be signed regardless of whether direct_tx exists.
        let direct_from_cpfp = if let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx {
            Some(build_refund_job(
                leaf,
                node_tx,
                direct_from_cpfp_refund_tx,
                signing_public_key,
                direct_from_cpfp_signing_commitments[i].clone(),
                network,
                None, // Direct transactions don't use adaptor signatures
            )?)
        } else {
            None
        };

        leaf_jobs.push(LeafRefundJobs {
            cpfp,
            direct,
            direct_from_cpfp,
        });
    }

    // Sign every leaf's jobs in one batched call, then split by variant. Remote
    // signer backends (e.g. Turnkey) collapse this into one round-trip instead of
    // one per leaf-variant.
    let signed = sign_leaf_refunds(spark_signer, leaf_jobs).await?;
    let (cpfp_signed_tx, direct_signed_tx, direct_from_cpfp_signed_tx) =
        into_signed_tx_groups(signed);

    Ok(SignedRefundTransactions {
        cpfp_signed_tx,
        direct_signed_tx,
        direct_from_cpfp_signed_tx,
    })
}

/// Builds the shared refund FROST job, computing the sighash from the parent
/// output first. A thin wrapper over `build_refund_signing_job` that keeps the
/// send-path call sites terse (they pass the leaf and its parent tx rather than
/// a precomputed sighash).
fn build_refund_job(
    leaf: &LeafKeyTweak,
    tx: &Transaction,
    refund_tx: Transaction,
    signing_public_key: PublicKey,
    spark_commitments: BTreeMap<Identifier, SigningCommitments>,
    network: Network,
    adaptor_public_key: Option<&PublicKey>,
) -> Result<RefundJob, SignerError> {
    let sighash = sighash_from_tx(&refund_tx, 0, &tx.output[0])
        .map_err(|e| SignerError::Generic(e.to_string()))?;
    Ok(build_refund_signing_job(
        &leaf.node.id,
        &leaf.node.verifying_public_key,
        &signing_public_key,
        refund_tx,
        sighash.to_raw_hash().to_byte_array(),
        spark_commitments,
        adaptor_public_key,
        network,
    ))
}
