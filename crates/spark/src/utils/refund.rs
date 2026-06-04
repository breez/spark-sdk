use std::collections::BTreeMap;
use std::sync::Arc;

use bitcoin::Transaction;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::Identifier;
use frost_secp256k1_tr::round1::SigningCommitments;
use tracing::info;

use crate::core::next_lightning_htlc_sequence;
use crate::services::SignedTx;
use crate::signer::{FrostDerivation, FrostJob, SignerError, SparkSigner};
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

    let mut cpfp_signed_refunds = Vec::with_capacity(leaves.len());
    let mut direct_signed_refunds = Vec::with_capacity(leaves.len());
    let mut direct_from_cpfp_signed_refunds = Vec::with_capacity(leaves.len());

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

        let signing_public_key = spark_signer.get_public_key_for_leaf(&leaf.node.id).await?;

        let cpfp_signed_tx = sign_refund(
            spark_signer,
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

            let direct_refund_tx = sign_refund(
                spark_signer,
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
        }

        // direct_from_cpfp_refund_tx spends from the CPFP (node_tx) output, not from
        // direct_tx, so it must be signed regardless of whether direct_tx exists.
        if let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx {
            let direct_from_cpfp_signed_tx = sign_refund(
                spark_signer,
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
    spark_signer: &Arc<dyn SparkSigner>,
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

    // Refund signing always uses the leaf's own (derived) signing key, keyed by
    // the leaf's node id.
    let job = FrostJob {
        derivation: FrostDerivation::SigningLeaf {
            leaf_id: leaf.node.id.clone(),
        },
        sighash: sighash.to_raw_hash().to_byte_array(),
        verifying_key: leaf.node.verifying_public_key,
        operator_commitments: spark_commitments.clone(),
        adaptor_public_key: adaptor_public_key.copied(),
    };
    let share = spark_signer
        .sign_frost(vec![job])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| SignerError::Generic("sign_frost returned no share".to_string()))?;

    Ok(SignedTx {
        node_id: leaf.node.id.clone(),
        signing_public_key,
        tx: refund_tx,
        user_signature: share.signature_share,
        self_nonce_commitment: share.commitment,
        signing_commitments: spark_commitments,
        network,
    })
}
