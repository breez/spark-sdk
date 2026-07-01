use std::collections::BTreeMap;
use std::sync::Arc;

use bitcoin::TxOut;
use bitcoin::hashes::Hash;
use bitcoin::{Transaction, secp256k1::PublicKey};
use frost_secp256k1_tr::Identifier;
use frost_secp256k1_tr::round1::SigningCommitments;

use crate::Network;
use crate::bitcoin::sighash_from_tx;
use crate::services::SignedTx;
use crate::signer::{FrostDerivation, FrostJob, SparkSigner};
use crate::utils::frost::sign_frost_batch;
use crate::{signer::SignerError, tree::TreeNodeId};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum SigningJobType {
    CpfpNode,
    CpfpSplitNode,
    CpfpRefund,
    DirectNode,
    DirectSplitNode,
    DirectRefund,
    DirectFromCpfpRefund,
}

#[derive(Clone)]
pub(crate) struct SigningJob {
    pub job_type: SigningJobType,
    pub node_id: TreeNodeId,
    pub tx: Transaction,
    pub parent_tx_out: TxOut,
    pub signing_public_key: PublicKey,
    pub verifying_public_key: PublicKey,
}

pub struct SignedJob {
    pub job_type: SigningJobType,
    pub signed_tx: SignedTx,
}

pub async fn sign_signing_jobs(
    spark_signer: &Arc<dyn SparkSigner>,
    signing_jobs: Vec<SigningJob>,
    signing_commitments: Vec<BTreeMap<Identifier, SigningCommitments>>,
    network: Network,
) -> Result<Vec<SignedJob>, SignerError> {
    // Build every renewal-tx FROST job up front, then sign the whole batch in one
    // call: remote signers turn each sign_frost into a network round-trip, so a
    // per-job loop would cost one round-trip per renewal.
    let mut jobs = Vec::with_capacity(signing_jobs.len());
    for (i, signing_job) in signing_jobs.iter().enumerate() {
        let sighash = sighash_from_tx(&signing_job.tx, 0, &signing_job.parent_tx_out)
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        // Each renewal tx is signed with the node's derived leaf key.
        jobs.push(FrostJob {
            derivation: FrostDerivation::SigningLeaf {
                leaf_id: signing_job.node_id.clone(),
            },
            sighash: sighash.to_raw_hash().to_byte_array(),
            verifying_key: signing_job.verifying_public_key,
            operator_commitments: signing_commitments[i].clone(),
            adaptor_public_key: None,
        });
    }

    let signed = sign_frost_batch(spark_signer, jobs, signing_jobs).await?;
    let signed_txs = signed
        .into_iter()
        .zip(signing_commitments)
        .map(|((signing_job, share), commitments)| SignedJob {
            job_type: signing_job.job_type,
            signed_tx: SignedTx {
                node_id: signing_job.node_id,
                signing_public_key: signing_job.signing_public_key,
                tx: signing_job.tx,
                user_signature: share.signature_share,
                self_nonce_commitment: share.commitment,
                signing_commitments: commitments,
                network,
            },
        })
        .collect();

    Ok(signed_txs)
}
