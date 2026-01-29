use std::collections::BTreeMap;
use std::sync::Arc;

use bitcoin::TxOut;
use bitcoin::hashes::Hash;
use bitcoin::{Transaction, secp256k1::PublicKey};
use frost_secp256k1_tr::Identifier;
use frost_secp256k1_tr::round1::SigningCommitments;

use crate::Network;
use crate::bitcoin::sighash_from_tx;
use crate::operator::rpc as operator_rpc;
use crate::services::{ServiceError, SignedTx};
use crate::signer::{FrostSigningCommitmentsWithNonces, SecretSource, SignFrostRequest};
use crate::{
    signer::{Signer, SignerError},
    tree::TreeNodeId,
};

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
    pub signing_commitments: FrostSigningCommitmentsWithNonces,
    pub verifying_public_key: PublicKey,
}

impl AsRef<SigningJob> for SigningJob {
    fn as_ref(&self) -> &SigningJob {
        self
    }
}

impl TryFrom<&SigningJob> for operator_rpc::spark::SigningJob {
    type Error = ServiceError;

    fn try_from(signing_job: &SigningJob) -> Result<Self, Self::Error> {
        Ok(operator_rpc::spark::SigningJob {
            raw_tx: bitcoin::consensus::serialize(&signing_job.tx),
            signing_public_key: signing_job.signing_public_key.serialize().to_vec(),
            signing_nonce_commitment: Some(signing_job.signing_commitments.commitments.try_into()?),
        })
    }
}

pub struct SignedJob {
    pub job_type: SigningJobType,
    pub signed_tx: SignedTx,
}

pub async fn sign_signing_jobs(
    signer: &Arc<dyn Signer>,
    signing_jobs: Vec<SigningJob>,
    signing_commitments: Vec<BTreeMap<Identifier, SigningCommitments>>,
    network: Network,
) -> Result<Vec<SignedJob>, SignerError> {
    let mut signed_txs = Vec::new();

    for (i, signing_job) in signing_jobs.iter().enumerate() {
        let sighash = sighash_from_tx(&signing_job.tx, 0, &signing_job.parent_tx_out)
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        let private_key = SecretSource::Derived(signing_job.node_id.clone());
        let user_signature = signer
            .sign_frost(SignFrostRequest {
                message: sighash.to_raw_hash().to_byte_array().as_ref(),
                public_key: &signing_job.signing_public_key,
                private_key: &private_key,
                verifying_key: &signing_job.verifying_public_key,
                self_nonce_commitment: &signing_job.signing_commitments,
                statechain_commitments: signing_commitments[i].clone(),
                adaptor_public_key: None,
            })
            .await?;
        signed_txs.push(SignedJob {
            job_type: signing_job.job_type,
            signed_tx: SignedTx {
                node_id: signing_job.node_id.clone(),
                signing_public_key: signing_job.signing_public_key,
                tx: signing_job.tx.clone(),
                user_signature,
                self_nonce_commitment: signing_job.signing_commitments.clone(),
                signing_commitments: signing_commitments[i].clone(),
                network,
            },
        })
    }

    Ok(signed_txs)
}
