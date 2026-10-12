//! Signing a transaction against commitments the operators made first, so the SSP
//! makes its nonce and signs with it in one step.

use std::collections::HashMap;
use std::sync::Arc;

use bitcoin::hashes::Hash as _;
use bitcoin::secp256k1::PublicKey;
use bitcoin::{TapSighash, Transaction};
use spark::operator::rpc::SparkRpcClient;
use spark::operator::rpc::common::SigningCommitment;
use spark::operator::rpc::spark as pb;
use spark::services::{ServiceError, SigningResult};
use spark::signer::{CommitAndSignFrostRequest, SecretSource, Signer};

/// The operators' commitments for one signature, by operator identifier.
pub type OperatorCommitments = HashMap<String, SigningCommitment>;

/// Returns `node_id_count * count` sets of commitments, all from the same operators.
pub async fn operator_commitments(
    client: &SparkRpcClient,
    node_id_count: u32,
    count: u32,
) -> Result<Vec<OperatorCommitments>, ServiceError> {
    let response = client
        .get_signing_commitments(pb::GetSigningCommitmentsRequest {
            node_ids: Vec::new(),
            count,
            node_id_count,
        })
        .await?;
    Ok(response
        .signing_commitments
        .into_iter()
        .map(|commitments| commitments.signing_nonce_commitments)
        .collect())
}

/// The SSP's share of a signature, with the commitments it was made against.
pub struct SignedShare {
    signing_nonce_commitment: SigningCommitment,
    user_signature: Vec<u8>,
    operator_commitments: OperatorCommitments,
}

impl SignedShare {
    #[must_use]
    pub fn into_signing_job(
        self,
        signing_public_key: &PublicKey,
        tx: &Transaction,
    ) -> pb::UserSignedTxSigningJob {
        pb::UserSignedTxSigningJob {
            signing_public_key: signing_public_key.serialize().to_vec(),
            raw_tx: bitcoin::consensus::serialize(tx),
            signing_nonce_commitment: Some(self.signing_nonce_commitment),
            user_signature: self.user_signature,
            signing_commitments: Some(pb::SigningCommitments {
                signing_commitments: self.operator_commitments,
            }),
            ..Default::default()
        }
    }

    #[must_use]
    pub fn into_input_signing_data(self) -> pb::InputSigningData {
        pb::InputSigningData {
            signing_nonce_commitment: Some(self.signing_nonce_commitment),
            user_signature: self.user_signature,
            signing_commitments: Some(pb::SigningCommitments {
                signing_commitments: self.operator_commitments,
            }),
        }
    }
}

/// Signs `sighash` with a nonce made for this signature alone.
pub async fn sign_share(
    signer: &Arc<dyn Signer>,
    sighash: &TapSighash,
    signing_key: &SecretSource,
    signing_public_key: &PublicKey,
    verifying_key: &PublicKey,
    operator_commitments: OperatorCommitments,
) -> Result<SignedShare, ServiceError> {
    let statechain_commitments = SigningResult::try_from(&pb::SigningResult {
        signing_nonce_commitments: operator_commitments.clone(),
        ..Default::default()
    })?
    .signing_commitments;
    let share = signer
        .commit_and_sign_frost(CommitAndSignFrostRequest {
            message: sighash.as_byte_array(),
            public_key: signing_public_key,
            private_key: signing_key,
            verifying_key,
            statechain_commitments,
            adaptor_public_key: None,
        })
        .await?;
    Ok(SignedShare {
        signing_nonce_commitment: share.commitments.try_into()?,
        user_signature: share.signature_share.serialize(),
        operator_commitments,
    })
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::{BTreeMap, BTreeSet};

    use bitcoin::TapSighash;
    use bitcoin::hashes::Hash as _;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey, rand::thread_rng};
    use frost_secp256k1_tr::keys::{KeyPackage, SigningShare, VerifyingShare};
    use frost_secp256k1_tr::round1::{SigningCommitments, SigningNonces};
    use frost_secp256k1_tr::round2::SignatureShare;
    use frost_secp256k1_tr::{Identifier, SigningPackage, VerifyingKey};
    use spark::operator::rpc::spark as pb;
    use spark::signer::AggregateFrostRequest;
    use spark::utils::frost::aggregate_frost;

    use super::OperatorCommitments;

    /// One operator holding the whole operator key. It signs as the operators'
    /// signer does and aggregates as their coordinator does.
    pub(crate) struct TestOperator {
        identifier: Identifier,
        secret: SecretKey,
        nonces: SigningNonces,
    }

    impl TestOperator {
        pub(crate) fn new(secret_byte: u8) -> Self {
            let secret = SecretKey::from_slice(&[secret_byte; 32]).expect("secret");
            let share = SigningShare::deserialize(&secret.secret_bytes()).expect("share");
            let (nonces, _) = frost_secp256k1_tr::round1::commit(&share, &mut thread_rng());
            Self {
                identifier: Identifier::try_from(1u16).expect("identifier"),
                secret,
                nonces,
            }
        }

        pub(crate) fn public_key(&self) -> PublicKey {
            self.secret.public_key(&Secp256k1::new())
        }

        pub(crate) fn commitments(&self) -> OperatorCommitments {
            OperatorCommitments::from([(
                hex::encode(self.identifier.serialize()),
                (*self.nonces.commitments()).try_into().expect("commitment"),
            )])
        }

        /// The signature `share` was made for, finished with this operator's share.
        pub(crate) fn finish(
            &self,
            sighash: &TapSighash,
            verifying_key: &PublicKey,
            user_public_key: &PublicKey,
            share: pb::InputSigningData,
        ) -> Vec<u8> {
            let user_commitment: SigningCommitments = share
                .signing_nonce_commitment
                .expect("user commitment")
                .try_into()
                .expect("user commitment");
            let user_share = SignatureShare::deserialize(&share.user_signature).expect("share");
            let user = Identifier::derive(b"user").expect("identifier");

            let signing_package = SigningPackage::new_with_adaptor(
                BTreeMap::from([
                    (self.identifier, *self.nonces.commitments()),
                    (user, user_commitment),
                ]),
                Some(vec![
                    BTreeSet::from([self.identifier]),
                    BTreeSet::from([user]),
                ]),
                sighash.as_byte_array(),
                None,
            );
            let key_package = KeyPackage::new(
                self.identifier,
                SigningShare::deserialize(&self.secret.secret_bytes()).expect("share"),
                VerifyingShare::deserialize(&self.public_key().serialize()).expect("share"),
                VerifyingKey::deserialize(&verifying_key.serialize()).expect("verifying key"),
                1,
            );
            let operator_share = frost_secp256k1_tr::round2::sign_with_tweak(
                &signing_package,
                &self.nonces,
                &key_package,
                Some(&[]),
            )
            .expect("operator share");

            aggregate_frost(AggregateFrostRequest {
                message: sighash.as_byte_array(),
                statechain_signatures: BTreeMap::from([(self.identifier, operator_share)]),
                statechain_public_keys: BTreeMap::from([(self.identifier, self.public_key())]),
                verifying_key,
                statechain_commitments: BTreeMap::from([(
                    self.identifier,
                    *self.nonces.commitments(),
                )]),
                self_commitment: &user_commitment,
                public_key: user_public_key,
                self_signature: &user_share,
                adaptor_public_key: None,
            })
            .expect("aggregate")
            .serialize()
            .expect("signature")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bitcoin::TapSighash;
    use bitcoin::hashes::Hash as _;
    use bitcoin::key::TapTweak as _;
    use bitcoin::secp256k1::{Message, Secp256k1, schnorr};
    use spark::signer::{DefaultSigner, SecretSource, Signer};

    use super::sign_share;
    use super::testing::TestOperator;

    #[tokio::test]
    async fn a_share_made_after_the_operators_commit_finishes_into_a_key_spend_signature() {
        let secp = Secp256k1::new();
        let signer: Arc<dyn Signer> =
            Arc::new(DefaultSigner::new(&[7u8; 32], spark::Network::Regtest).expect("signer"));
        let signing_key =
            SecretSource::Derived(vec![bitcoin::bip32::ChildNumber::Hardened { index: 9 }].into());
        let signing_public_key = signer
            .public_key_from_secret(&signing_key)
            .await
            .expect("public key");
        let operator = TestOperator::new(3);
        let verifying_key = signing_public_key
            .combine(&operator.public_key())
            .expect("verifying key");
        let sighash = TapSighash::from_byte_array([5u8; 32]);

        let share = sign_share(
            &signer,
            &sighash,
            &signing_key,
            &signing_public_key,
            &verifying_key,
            operator.commitments(),
        )
        .await
        .expect("share");
        let signature = operator.finish(
            &sighash,
            &verifying_key,
            &signing_public_key,
            share.into_input_signing_data(),
        );

        let (output_key, _) = verifying_key.x_only_public_key().0.tap_tweak(&secp, None);
        secp.verify_schnorr(
            &schnorr::Signature::from_slice(&signature).expect("signature"),
            &Message::from_digest(sighash.to_byte_array()),
            &output_key.to_x_only_public_key(),
        )
        .expect("the signature spends an output of the verifying key");
    }
}
