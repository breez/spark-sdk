use std::collections::{BTreeMap, BTreeSet};

use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::keys::{PublicKeyPackage, VerifyingShare};
use frost_secp256k1_tr::round1::SigningCommitments;
use frost_secp256k1_tr::{Identifier, SigningPackage, VerifyingKey};

use crate::signer::{AggregateFrostRequest, SignerError};

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
