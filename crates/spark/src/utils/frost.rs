use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bitcoin::secp256k1::{All, PublicKey, Secp256k1};
use frost_secp256k1_tr::keys::{PublicKeyPackage, VerifyingShare};
use frost_secp256k1_tr::round1::SigningCommitments;
use frost_secp256k1_tr::{Identifier, SigningPackage, VerifyingKey};

use crate::signer::{AggregateFrostRequest, FrostJob, FrostShareResult, SignerError, SparkSigner};
use crate::tree::TreeNode;

/// The user's own signing public key for an OWNED leaf, derived from persisted
/// tree data instead of asking the signer: `verifying_public_key -
/// signing_keyshare.public_key`. FROST composes the group verifying key as the
/// user's verifying share plus the operators' aggregate share, and
/// `refresh_leaves` validates this relation for every Available leaf, so the
/// user's share is recoverable locally. This avoids a per-leaf signer round-trip
/// on the send/coop-exit/timelock hot paths (for a remote signer with a cold
/// in-memory cache, e.g. a per-request server instance, that would otherwise be
/// one network call per leaf). Only valid for owned leaves: an incoming (claim)
/// leaf is mid-transfer, so its stored SE share need not pair with the new key.
pub(crate) fn derive_leaf_signing_public_key(
    node: &TreeNode,
    secp: &Secp256k1<All>,
) -> Result<PublicKey, SignerError> {
    let se_share = node.signing_keyshare.public_key.negate(secp);
    node.verifying_public_key
        .combine(&se_share)
        .map_err(|e| SignerError::Generic(format!("failed to derive leaf signing key: {e}")))
}

/// Signs a batch of FROST jobs in a single `sign_frost` call, pairing each
/// returned share with its caller-side metadata (order preserved). Remote signer
/// backends (e.g. Turnkey) collapse this into one round-trip instead of one per
/// job, so callers should build all jobs up front rather than signing per item.
pub(crate) async fn sign_frost_batch<T>(
    spark_signer: &Arc<dyn SparkSigner>,
    jobs: Vec<FrostJob>,
    pending: Vec<T>,
) -> Result<Vec<(T, FrostShareResult)>, SignerError> {
    let shares = spark_signer.sign_frost(jobs).await?;
    if shares.len() != pending.len() {
        return Err(SignerError::Generic(format!(
            "sign_frost returned {} shares, expected {}",
            shares.len(),
            pending.len()
        )));
    }
    Ok(pending.into_iter().zip(shares).collect())
}

/// Builds the FROST [`SigningPackage`] for a user + statechain signing round.
///
/// Adds the user's commitment to the statechain commitments and splits the
/// participants into two groups (statechain, user), optionally binding an
/// adaptor public key for adaptor-signature flows (atomic swaps).
pub(crate) fn frost_signing_package(
    user_identifier: Identifier,
    message: &[u8],
    statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
    self_commitment: &SigningCommitments,
    adaptor_public_key: Option<&PublicKey>,
) -> Result<SigningPackage, SignerError> {
    // Clone statechain commitments to add our own commitment
    let mut signing_commitments = statechain_commitments.clone();

    // Create participant groups for the signing operation.
    // First group is all statechain participants.
    let mut signing_participants_groups = Vec::new();
    signing_participants_groups.push(
        statechain_commitments
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
    );

    // Add the user's commitment to the signing commitments
    signing_commitments.insert(user_identifier, *self_commitment);
    // Add a second participant group containing only the user
    signing_participants_groups.push(BTreeSet::from([user_identifier]));

    // Convert the adaptor public key format if provided
    let adaptor = match adaptor_public_key {
        Some(pk) => {
            let adaptor = VerifyingKey::deserialize(pk.serialize().as_slice()).map_err(|e| {
                SignerError::SerializationError(format!(
                    "Failed to deserialize adaptor public key: {e}"
                ))
            })?;
            Some(adaptor)
        }
        None => None,
    };

    // Create a signing package containing commitments, participant groups, message and adaptor
    Ok(SigningPackage::new_with_adaptor(
        signing_commitments,
        Some(signing_participants_groups),
        message,
        adaptor,
    ))
}

/// Aggregates FROST signature shares (user + statechain) into a complete
/// Schnorr signature.
///
/// This is **pure public math** (no private key is involved), so it lives as a
/// free function callable without a [`Signer`](crate::signer::Signer). Flows
/// that already hold a valid user signature share (e.g. the atomic-swap adaptor
/// step) can aggregate it directly rather than re-signing.
pub fn aggregate_frost(
    request: AggregateFrostRequest<'_>,
) -> Result<frost_secp256k1_tr::Signature, SignerError> {
    // Derive an identifier for the local user
    let user_identifier =
        Identifier::derive("user".as_bytes()).map_err(|_| SignerError::IdentifierError)?;

    // Create a signing package containing commitments, participant groups, message and adaptor
    let signing_package = frost_signing_package(
        user_identifier,
        request.message,
        request.statechain_commitments,
        request.self_commitment,
        request.adaptor_public_key,
    )?;

    // Combine all signature shares (statechain + user)
    let mut signature_shares = request.statechain_signatures.clone();
    signature_shares.insert(user_identifier, *request.self_signature);

    // Build a map of verifying shares for all participants
    let mut verifying_shares = BTreeMap::new();
    // Convert statechain public keys to verifying shares
    for (id, pk) in request.statechain_public_keys.iter() {
        let verifying_key =
            VerifyingShare::deserialize(pk.serialize().as_slice()).map_err(|e| {
                SignerError::SerializationError(format!(
                    "Failed to deserialize public key for participant {id:?}: {e} (culprit: {:?})",
                    e.culprit()
                ))
            })?;
        verifying_shares.insert(*id, verifying_key);
    }

    // Add the user's public key as a verifying share
    verifying_shares.insert(
        user_identifier,
        VerifyingShare::deserialize(request.public_key.serialize().as_slice()).map_err(|e| {
            SignerError::SerializationError(format!(
                "Failed to deserialize user public key: {e} (culprit: {:?})",
                e.culprit()
            ))
        })?,
    );

    let verifying_key = VerifyingKey::deserialize(request.verifying_key.serialize().as_slice())
        .map_err(|e| {
            SignerError::SerializationError(format!(
                "Failed to deserialize group verifying key: {e} (culprit: {:?})",
                e.culprit()
            ))
        })?;

    // Create a public key package with all verifying shares and the group's verifying key
    let public_key_package = PublicKeyPackage::new(verifying_shares, verifying_key);

    // For taproot signatures, we provide an empty merkle root
    let merkle_root = Vec::new();

    // Aggregate all signature shares into a final signature
    frost_secp256k1_tr::aggregate_with_tweak(
        &signing_package,
        &signature_shares,
        &public_key_package,
        Some(&merkle_root),
    )
    .map_err(|e| {
        SignerError::FrostError(format!(
            "Failed to aggregate signatures: {e} (culprit: {:?})",
            e.culprit()
        ))
    })
}
