//! Live coverage that a batched `sign_frost` survives past Turnkey's single-
//! activity ceiling.
//!
//! The batched send/coop-exit/claim/timelock paths pack one to three FROST jobs
//! per leaf into a single `sign_frost` call, and nothing upstream caps the leaf
//! count (a full-balance coop-exit signs every wallet leaf). A single Turnkey
//! `SPARK_SIGN_FROST` activity fails with HTTP 500 (an internal ~15s timeout)
//! past roughly 250 to 300 jobs, well below its ~11 MB request-body cap, so
//! `TurnkeySparkSigner` chunks its submissions (`MAX_FROST_JOBS_PER_ACTIVITY`).
//! This signs a batch above that ceiling and checks one share returns per job,
//! which only holds if the chunking splits the batch across activities.
#![cfg(feature = "turnkey")]

use anyhow::Result;
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use breez_sdk_itest::turnkey::provision_turnkey_wallet;
use breez_sdk_spark::signer::{
    ExternalFrostDerivation, ExternalFrostJob, ExternalIdentifier, ExternalSigningCommitments,
    ExternalTreeNodeId, IdentifierCommitmentPair,
};
use tracing::info;

/// FROST jobs carry one commitment per signing operator; the production pool has
/// three, so each synthetic job is representative in size.
const OPERATORS: u8 = 3;

/// Comfortably above Turnkey's ~250 to 300 job single-activity ceiling, so a
/// success proves the signer split the batch across activities rather than
/// submitting it as one (which would fail).
const BATCH_JOBS: usize = 300;

/// Builds `n` synthetic FROST jobs. Every field is a real, deserializable value
/// (curve points from throwaway keys, 32-byte scalars for identifiers), so
/// Turnkey signs each against a locally derived key and returns a real share;
/// only the leaf id varies per job, matching a real batch's shape.
fn build_jobs(n: usize) -> Vec<ExternalFrostJob> {
    let secp = Secp256k1::new();
    let point = |b: u8| {
        SecretKey::from_slice(&[b.max(1); 32])
            .unwrap()
            .public_key(&secp)
            .serialize()
            .to_vec()
    };
    let verifying_key = point(7);
    let operator_commitments: Vec<IdentifierCommitmentPair> = (0..OPERATORS)
        .map(|k| {
            // 32-byte big-endian scalar (k+1): a valid, canonical FROST identifier.
            let mut id = [0u8; 32];
            id[31] = k + 1;
            IdentifierCommitmentPair {
                identifier: ExternalIdentifier { bytes: id.to_vec() },
                commitment: ExternalSigningCommitments {
                    hiding: point(10 + k),
                    binding: point(20 + k),
                },
            }
        })
        .collect();

    (0..n)
        .map(|i| ExternalFrostJob {
            derivation: ExternalFrostDerivation::SigningLeaf {
                leaf_id: ExternalTreeNodeId {
                    id: format!("00000000-0000-7000-8000-{i:012x}"),
                },
            },
            sighash: vec![0u8; 32],
            verifying_key: verifying_key.clone(),
            operator_commitments: operator_commitments.clone(),
            adaptor_public_key: None,
        })
        .collect()
}

#[test_log::test(tokio::test)]
async fn sign_frost_chunks_batch_above_single_activity_limit() -> Result<()> {
    let (config, _guard) = provision_turnkey_wallet().await?;
    let signers = breez_sdk_spark::turnkey::create_turnkey_signer(config)
        .await
        .map_err(|e| anyhow::anyhow!("create_turnkey_signer failed: {e}"))?;

    let jobs = build_jobs(BATCH_JOBS);
    let shares = signers
        .spark_signer
        .sign_frost(jobs)
        .await
        .map_err(|e| anyhow::anyhow!("sign_frost({BATCH_JOBS} jobs) failed: {e}"))?;

    assert_eq!(
        shares.len(),
        BATCH_JOBS,
        "chunked sign_frost must return one share per job"
    );
    info!("[Turnkey] sign_frost chunked {BATCH_JOBS} jobs into shares");
    Ok(())
}
