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
use crate::signer::{FrostJob, LeafSigningKey, SparkSigner};
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
    signing_key: &LeafSigningKey,
    signing_jobs: Vec<SigningJob>,
    signing_commitments: Vec<BTreeMap<Identifier, SigningCommitments>>,
    network: Network,
) -> Result<Vec<SignedJob>, SignerError> {
    // Build every renewal-tx FROST job up front, then sign the whole batch in one
    // call.
    let mut jobs = Vec::with_capacity(signing_jobs.len());
    for (i, signing_job) in signing_jobs.iter().enumerate() {
        let sighash = sighash_from_tx(&signing_job.tx, 0, &signing_job.parent_tx_out)
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        jobs.push(FrostJob {
            derivation: signing_key.into(),
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, absolute::LockTime,
        transaction::Version,
    };
    use macros::async_test_all;

    use super::{SigningJob, SigningJobType, sign_signing_jobs};
    use crate::Network;
    use crate::signer::testing::{RecordingSparkSigner, operator_commitments};
    use crate::signer::{FrostDerivation, LeafSigningKey, SparkSigner};
    use crate::tree::TreeNodeId;

    fn spend() -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                sequence: Sequence::ZERO,
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    /// Every transaction of a renewal is signed with the key the leaf is held
    /// under, whatever id that key derives from, while the signed transactions
    /// keep naming the leaf by its node id.
    #[async_test_all]
    async fn every_renewal_transaction_is_signed_with_the_key_the_leaf_is_held_under() {
        let node_id: TreeNodeId = "leaf".parse().unwrap();
        for held_under in [TreeNodeId::generate(), node_id.clone()] {
            let recorder = Arc::new(RecordingSparkSigner::new());
            let signer: Arc<dyn SparkSigner> = recorder.clone();
            let signing_public_key = signer.get_public_key_for_leaf(&held_under).await.unwrap();
            let job_types = [
                SigningJobType::CpfpNode,
                SigningJobType::DirectNode,
                SigningJobType::CpfpRefund,
            ];
            let jobs = job_types
                .iter()
                .map(|job_type| SigningJob {
                    job_type: *job_type,
                    node_id: node_id.clone(),
                    tx: spend(),
                    parent_tx_out: spend().output[0].clone(),
                    signing_public_key,
                    verifying_public_key: signing_public_key,
                })
                .collect();
            let mut commitments = Vec::new();
            for _ in job_types {
                commitments.push(operator_commitments(3).await);
            }

            let signed = sign_signing_jobs(
                &signer,
                &LeafSigningKey {
                    derived_from: held_under.clone(),
                },
                jobs,
                commitments,
                Network::Regtest,
            )
            .await
            .unwrap();

            assert_eq!(
                recorder.frost_derivations(),
                vec![
                    FrostDerivation::SigningLeaf {
                        leaf_id: held_under.clone()
                    };
                    job_types.len()
                ]
            );
            assert_eq!(signed.len(), job_types.len());
            for (signed, job_type) in signed.iter().zip(job_types) {
                assert_eq!(signed.job_type, job_type);
                assert_eq!(signed.signed_tx.node_id, node_id);
                assert_eq!(signed.signed_tx.signing_public_key, signing_public_key);
            }
        }
    }
}
