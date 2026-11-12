use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bitcoin::secp256k1::{All, PublicKey, Secp256k1};
use frost_secp256k1_tr::keys::{PublicKeyPackage, VerifyingShare};
use frost_secp256k1_tr::round1::SigningCommitments;
use frost_secp256k1_tr::{Identifier, SigningPackage, VerifyingKey};

use crate::signer::{
    AggregateFrostRequest, FrostAdaptor, FrostJob, FrostShareResult, SignerError, SparkSigner,
};
use crate::tree::TreeNode;

/// The user's own signing public key for an OWNED leaf, recovered from persisted
/// tree data as `verifying_public_key - signing_keyshare.public_key` rather than
/// via a signer round-trip: FROST composes the group verifying key as the user's
/// share plus the operators' aggregate share.
///
/// The result is only as trustworthy as the persisted operands: a poisoned
/// `signing_keyshare.public_key` steers it, so use it only where a wrong value
/// fails closed (an invalid signature), never to choose a payout destination.
/// Valid only for owned leaves: an incoming (claim) leaf is mid-transfer, so its
/// stored SE share need not pair with the new key.
pub(crate) fn derive_leaf_signing_public_key(
    node: &TreeNode,
    secp: &Secp256k1<All>,
) -> Result<PublicKey, SignerError> {
    let se_share = node.signing_keyshare.public_key.negate(secp);
    node.verifying_public_key
        .combine(&se_share)
        .map_err(|e| SignerError::Generic(format!("failed to derive leaf signing key: {e}")))
}

/// Zips signer shares back onto their caller-side metadata one-to-one, erroring
/// on a count mismatch instead of silently dropping the tail (a plain `zip`
/// truncates to the shorter side, misrouting every share past the gap). This is
/// the single length guard behind the batched signing paths, so the reliance on
/// `sign_frost` returning one share per job in order is enforced in one place.
pub(crate) fn zip_checked<S, T>(
    shares: Vec<S>,
    pending: Vec<T>,
) -> Result<Vec<(T, S)>, SignerError> {
    if shares.len() != pending.len() {
        return Err(SignerError::Generic(format!(
            "sign_frost returned {} shares, expected {}",
            shares.len(),
            pending.len()
        )));
    }
    Ok(pending.into_iter().zip(shares).collect())
}

/// Signs a batch of FROST jobs in a single `sign_frost` call, pairing each
/// returned share with its caller-side metadata (order preserved).
pub(crate) async fn sign_frost_batch<T>(
    spark_signer: &Arc<dyn SparkSigner>,
    jobs: Vec<FrostJob>,
    pending: Vec<T>,
) -> Result<Vec<(T, FrostShareResult)>, SignerError> {
    let shares = spark_signer.sign_frost(jobs).await?;
    zip_checked(shares, pending)
}

/// Builds the FROST [`SigningPackage`] for a user + statechain signing round.
///
/// Adds the user's commitment to the statechain commitments and splits the
/// participants into two groups (statechain, user), optionally binding an
/// adaptor for adaptor-signature flows (atomic swaps).
pub(crate) fn frost_signing_package(
    user_identifier: Identifier,
    message: &[u8],
    statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
    self_commitment: &SigningCommitments,
    adaptor: Option<FrostAdaptor>,
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

    let Some(adaptor) = adaptor else {
        return Ok(SigningPackage::new_with_adaptor(
            signing_commitments,
            Some(signing_participants_groups),
            message,
            None,
        ));
    };
    let adaptor_key = VerifyingKey::deserialize(adaptor.public_key.serialize().as_slice())
        .map_err(|e| {
            SignerError::SerializationError(format!(
                "Failed to deserialize adaptor public key: {e}"
            ))
        })?;
    let new_signing_package = if adaptor.v2 {
        SigningPackage::new_with_adaptor_v2
    } else {
        SigningPackage::new_with_adaptor
    };
    Ok(new_signing_package(
        signing_commitments,
        Some(signing_participants_groups),
        message,
        Some(adaptor_key),
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
        request.adaptor,
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
                    "Failed to deserialize public key for participant {id:?}: {e} (culprits: {:?})",
                    e.culprits()
                ))
            })?;
        verifying_shares.insert(*id, verifying_key);
    }

    // Add the user's public key as a verifying share
    verifying_shares.insert(
        user_identifier,
        VerifyingShare::deserialize(request.public_key.serialize().as_slice()).map_err(|e| {
            SignerError::SerializationError(format!(
                "Failed to deserialize user public key: {e} (culprits: {:?})",
                e.culprits()
            ))
        })?,
    );

    let verifying_key = VerifyingKey::deserialize(request.verifying_key.serialize().as_slice())
        .map_err(|e| {
            SignerError::SerializationError(format!(
                "Failed to deserialize group verifying key: {e} (culprits: {:?})",
                e.culprits()
            ))
        })?;

    // Create a public key package with all verifying shares and the group's verifying key
    let public_key_package = PublicKeyPackage::new(verifying_shares, verifying_key, None);

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
            "Failed to aggregate signatures: {e} (culprits: {:?})",
            e.culprits()
        ))
    })
}

#[cfg(test)]
mod tests {
    use bitcoin::key::TapTweak as _;
    use bitcoin::secp256k1::rand::thread_rng;
    use bitcoin::secp256k1::{Message, SecretKey, schnorr};
    use frost_core::round1::Nonce;
    use frost_secp256k1_tr::keys::{KeyPackage, SigningShare};
    use frost_secp256k1_tr::round1::SigningNonces;
    use k256::Scalar;
    use k256::elliptic_curve::PrimeField as _;

    use super::*;
    use crate::signer::{FrostDerivation, SparkSignerAdapter, create_test_signer};
    use crate::tree::TreeNodeId;
    use crate::tree::tests::create_test_tree_node;

    /// Signs an adaptor signature with the user's share from the default signer
    /// and one operator's share, and aggregates them as the client does. The
    /// user signs with `user_v2`, the operator and the aggregation with
    /// `operator_v2`. Returns how many completions with the adaptor secret, by
    /// adding or subtracting it, verify.
    async fn adaptor_completions_that_verify(user_v2: bool, operator_v2: bool) -> usize {
        let secp = Secp256k1::new();
        let signer = SparkSignerAdapter::new(Arc::new(create_test_signer()));
        let leaf_id = TreeNodeId::generate();
        let user_public_key = signer.get_public_key_for_leaf(&leaf_id).await.unwrap();
        let user = Identifier::derive(b"user").unwrap();

        let operator = Identifier::derive(b"operator").unwrap();
        let operator_secret = SecretKey::new(&mut thread_rng());
        let operator_public_key = operator_secret.public_key(&secp);
        let nonce = || Nonce::deserialize(&SecretKey::new(&mut thread_rng()).secret_bytes());
        let operator_nonces = SigningNonces::from_nonces(nonce().unwrap(), nonce().unwrap());
        let operator_commitments = BTreeMap::from([(operator, *operator_nonces.commitments())]);
        let verifying_key = user_public_key.combine(&operator_public_key).unwrap();

        let adaptor_secret = SecretKey::new(&mut thread_rng());
        let adaptor = |v2| FrostAdaptor {
            public_key: adaptor_secret.public_key(&secp),
            v2,
        };
        let sighash = [7u8; 32];

        let user_share = signer
            .sign_frost(vec![FrostJob {
                derivation: FrostDerivation::SigningLeaf { leaf_id },
                sighash,
                verifying_key,
                operator_commitments: operator_commitments.clone(),
                adaptor: Some(adaptor(user_v2)),
            }])
            .await
            .unwrap()
            .remove(0);

        let operator_package = frost_signing_package(
            user,
            &sighash,
            operator_commitments.clone(),
            &user_share.commitment,
            Some(adaptor(operator_v2)),
        )
        .unwrap();
        let operator_key_package = KeyPackage::new(
            operator,
            SigningShare::deserialize(&operator_secret.secret_bytes()).unwrap(),
            VerifyingShare::deserialize(&operator_public_key.serialize()).unwrap(),
            VerifyingKey::deserialize(&verifying_key.serialize()).unwrap(),
            1,
        );
        let operator_share = frost_secp256k1_tr::round2::sign_with_tweak(
            &operator_package,
            &operator_nonces,
            &operator_key_package,
            Some(&[]),
        )
        .unwrap();

        let pre_signature = aggregate_frost(AggregateFrostRequest {
            message: &sighash,
            statechain_signatures: BTreeMap::from([(operator, operator_share)]),
            statechain_public_keys: BTreeMap::from([(operator, operator_public_key)]),
            verifying_key: &verifying_key,
            statechain_commitments: operator_commitments,
            self_commitment: &user_share.commitment,
            public_key: &user_public_key,
            self_signature: &user_share.signature_share,
            adaptor: Some(adaptor(operator_v2)),
        })
        .unwrap()
        .serialize()
        .unwrap();

        let scalar = |bytes: [u8; 32]| Scalar::from_repr(bytes.into()).unwrap();
        let z = scalar(pre_signature[32..].try_into().unwrap());
        let t = scalar(adaptor_secret.secret_bytes());
        let (output_key, _) = verifying_key.x_only_public_key().0.tap_tweak(&secp, None);
        [z + t, z - t]
            .into_iter()
            .filter(|completed| {
                let mut signature = pre_signature.clone();
                signature[32..].copy_from_slice(&completed.to_bytes());
                secp.verify_schnorr(
                    &schnorr::Signature::from_slice(&signature).unwrap(),
                    &Message::from_digest(sighash),
                    &output_key.to_x_only_public_key(),
                )
                .is_ok()
            })
            .count()
    }

    #[macros::async_test_all]
    async fn adaptor_signatures_complete_under_either_version() {
        // Enough runs that both nonce parities occur.
        for _ in 0..8 {
            for v2 in [false, true] {
                assert_eq!(adaptor_completions_that_verify(v2, v2).await, 1);
            }
        }
    }

    #[macros::async_test_all]
    async fn adaptor_shares_of_different_versions_do_not_complete() {
        assert_eq!(adaptor_completions_that_verify(true, false).await, 0);
        assert_eq!(adaptor_completions_that_verify(false, true).await, 0);
    }

    /// `derive_leaf_signing_public_key` must recover the user's signing key from
    /// the FROST relation `verifying_public_key = user_share + se_share`, i.e.
    /// return `verifying_public_key - signing_keyshare.public_key`.
    #[test]
    fn derives_user_share_from_verifying_and_se_share() {
        let secp = Secp256k1::new();

        // se_share = a*G (operators' aggregate), user_share = b*G (what we derive).
        let a = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let b = SecretKey::from_slice(&[0x22; 32]).unwrap();
        let se_share = a.public_key(&secp);
        let user_share = b.public_key(&secp);
        let verifying = se_share.combine(&user_share).unwrap();

        let mut node = create_test_tree_node("test_leaf", 1000);
        node.signing_keyshare.public_key = se_share;
        node.verifying_public_key = verifying;

        let derived = derive_leaf_signing_public_key(&node, &secp).unwrap();
        assert_eq!(derived, user_share);
    }

    /// `zip_checked` pairs shares with pending metadata one-to-one and rejects a
    /// count mismatch rather than silently truncating: a plain `zip` would drop
    /// the tail and misroute every share past the gap.
    #[test]
    fn zip_checked_pairs_equal_lengths_and_rejects_mismatch() {
        // Equal lengths: (pending, share) pairs, order preserved.
        let paired = zip_checked(vec!['a', 'b'], vec![1, 2]).unwrap();
        assert_eq!(paired, vec![(1, 'a'), (2, 'b')]);

        // Too few shares (a truncated signer response) errors.
        assert!(zip_checked(vec!['a'], vec![1, 2]).is_err());
        // Too many shares errors as well.
        assert!(zip_checked(vec!['a', 'b', 'c'], vec![1, 2]).is_err());
    }
}
