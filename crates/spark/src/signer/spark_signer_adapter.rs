//! Default in-process [`SparkSigner`] implementation.
//!
//! `SparkSignerAdapter` wraps the low-level [`Signer`] trait and performs the
//! flow orchestration (key-tweak / Feldman split / ECIES / payload signing)
//! that used to live in the service layer. It reproduces the exact key
//! derivation we use today, so a wallet keyed by `DefaultSigner` produces
//! byte-identical keys whether it goes through this adapter or the old direct
//! `Signer` calls.

use std::collections::BTreeMap;
use std::sync::Arc;

use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::rand::thread_rng;
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use frost_secp256k1_tr::Identifier;
use prost::Message as _;

use super::spark_signer::*;
use super::{
    SecretSource, SecretToSplit, SignFrostRequest, Signer, SignerError, VerifiableSecretShare,
};
use crate::operator::rpc::spark as proto;
use crate::utils::tagged_hasher::TaggedHasher;

/// Length of a Lightning payment preimage in bytes.
const PREIMAGE_LEN: usize = 32;

pub struct SparkSignerAdapter {
    signer: Arc<dyn Signer>,
    secp: Secp256k1<bitcoin::secp256k1::All>,
}

impl SparkSignerAdapter {
    pub fn new(signer: Arc<dyn Signer>) -> Self {
        Self {
            signer,
            secp: Secp256k1::new(),
        }
    }

    /// Maps a flow-level [`FrostDerivation`] onto the low-level signer's
    /// [`SecretSource`], reproducing the current key derivation exactly.
    async fn secret_source_for(
        &self,
        derivation: &FrostDerivation,
    ) -> Result<SecretSource, SignerError> {
        match derivation {
            FrostDerivation::SigningLeaf { leaf_id } => Ok(SecretSource::Derived(leaf_id.clone())),
            FrostDerivation::StaticDeposit { index } => {
                self.signer.static_deposit_secret_encrypted(*index).await
            }
            FrostDerivation::HtlcPreimage => Err(SignerError::Generic(
                "HtlcPreimage FROST derivation not yet supported by the default adapter"
                    .to_string(),
            )),
            FrostDerivation::Identity => Err(SignerError::Generic(
                "Identity FROST derivation not supported".to_string(),
            )),
        }
    }

    /// Signs one FROST job: generates a fresh nonce commitment, derives the
    /// signing key, and produces the round-2 share.
    async fn sign_one_frost(
        &self,
        derivation: &FrostDerivation,
        sighash: &[u8; 32],
        verifying_key: &PublicKey,
        operator_commitments: BTreeMap<Identifier, frost_secp256k1_tr::round1::SigningCommitments>,
        adaptor_public_key: Option<&PublicKey>,
    ) -> Result<FrostShareResult, SignerError> {
        let private_key = self.secret_source_for(derivation).await?;
        let public_key = self.signer.public_key_from_secret(&private_key).await?;
        let self_nonce_commitment = self.signer.generate_random_signing_commitment().await?;
        let signature_share = self
            .signer
            .sign_frost(SignFrostRequest {
                message: sighash,
                public_key: &public_key,
                private_key: &private_key,
                verifying_key,
                self_nonce_commitment: &self_nonce_commitment,
                statechain_commitments: operator_commitments,
                adaptor_public_key,
            })
            .await?;
        Ok(FrostShareResult {
            commitment: self_nonce_commitment,
            signature_share,
        })
    }

    /// Signs a refund job with a caller-supplied `SecretSource` (used by claim,
    /// where the key is the freshly-derived receiver key rather than a
    /// `FrostDerivation`).
    async fn sign_refund_with_key(
        &self,
        private_key: &SecretSource,
        sighash: &[u8; 32],
        verifying_key: &PublicKey,
        operator_commitments: BTreeMap<Identifier, frost_secp256k1_tr::round1::SigningCommitments>,
    ) -> Result<FrostShareResult, SignerError> {
        let public_key = self.signer.public_key_from_secret(private_key).await?;
        let self_nonce_commitment = self.signer.generate_random_signing_commitment().await?;
        let signature_share = self
            .signer
            .sign_frost(SignFrostRequest {
                message: sighash,
                public_key: &public_key,
                private_key,
                verifying_key,
                self_nonce_commitment: &self_nonce_commitment,
                statechain_commitments: operator_commitments,
                adaptor_public_key: None,
            })
            .await?;
        Ok(FrostShareResult {
            commitment: self_nonce_commitment,
            signature_share,
        })
    }

    /// ECIES-encrypts `data` to an operator's public key (BIP-340 uncompressed
    /// SEC1, matching the rest of the codebase).
    fn encrypt_for_operator(
        &self,
        operator_public_key: &PublicKey,
        data: &[u8],
    ) -> Result<Vec<u8>, SignerError> {
        utils::ecies::encrypt(&operator_public_key.serialize_uncompressed(), data)
            .map_err(|e| SignerError::Generic(format!("ECIES encryption failed: {e}")))
    }

    /// Computes the per-operator public-key tweak shares for a Feldman split.
    fn pubkey_shares_tweak(
        &self,
        recipients: &[OperatorRecipient],
        shares: &[VerifiableSecretShare],
    ) -> Result<BTreeMap<String, Vec<u8>>, SignerError> {
        let mut pubkey_shares_tweak = BTreeMap::new();
        for recipient in recipients {
            let share = find_share(shares, recipient.id).ok_or_else(|| {
                SignerError::Generic(format!("Share not found for operator {}", recipient.id))
            })?;
            let pubkey_tweak = SecretKey::from_slice(&share.secret_share.share.to_bytes())
                .map_err(|_| SignerError::Generic("Invalid secret share".to_string()))?
                .public_key(&self.secp);
            pubkey_shares_tweak.insert(
                hex::encode(recipient.identifier.serialize()),
                pubkey_tweak.serialize().to_vec(),
            );
        }
        Ok(pubkey_shares_tweak)
    }

    fn proto_secret_share(share: &VerifiableSecretShare) -> proto::SecretShare {
        proto::SecretShare {
            secret_share: share.secret_share.share.to_bytes().to_vec(),
            proofs: share
                .proofs
                .iter()
                .map(|p| p.to_sec1_bytes().to_vec())
                .collect(),
        }
    }
}

/// Finds the Feldman share belonging to operator `operator_id` (share index is
/// `operator_id + 1`).
fn find_share(
    shares: &[VerifiableSecretShare],
    operator_id: usize,
) -> Option<&VerifiableSecretShare> {
    let target = k256::Scalar::from((operator_id + 1) as u64);
    shares.iter().find(|s| s.secret_share.index == target)
}

fn transfer_id_bytes(transfer_id: &crate::services::TransferId) -> Result<Vec<u8>, SignerError> {
    hex::decode(transfer_id.to_string().replace('-', ""))
        .map_err(|e| SignerError::Generic(format!("Failed to decode transfer ID: {e}")))
}

#[macros::async_trait]
impl SparkSigner for SparkSignerAdapter {
    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        self.signer.get_identity_public_key().await
    }

    async fn sign_frost(&self, jobs: Vec<FrostJob>) -> Result<Vec<FrostShareResult>, SignerError> {
        let mut results = Vec::with_capacity(jobs.len());
        for job in jobs {
            results.push(
                self.sign_one_frost(
                    &job.derivation,
                    &job.sighash,
                    &job.verifying_key,
                    job.operator_commitments,
                    job.adaptor_public_key.as_ref(),
                )
                .await?,
            );
        }
        Ok(results)
    }

    async fn prepare_transfer(
        &self,
        request: PrepareTransferRequest,
    ) -> Result<PreparedTransfer, SignerError> {
        let PrepareTransferRequest {
            transfer_id,
            receiver_public_key,
            leaves,
            operator_recipients,
            threshold,
        } = request;

        // Per-operator accumulator of this transfer's leaf key tweaks.
        let mut per_operator: BTreeMap<Identifier, Vec<proto::SendLeafKeyTweak>> = BTreeMap::new();
        let mut new_leaf_keys = Vec::with_capacity(leaves.len());

        for leaf in &leaves {
            let signing_key = SecretSource::Derived(leaf.node.id.clone());
            let new_secret = self.signer.generate_random_secret().await?;
            let new_signing_key = SecretSource::Encrypted(new_secret.clone());

            new_leaf_keys.push(NewLeafKey {
                node_id: leaf.node.id.clone(),
                new_signing_public_key: self
                    .signer
                    .public_key_from_secret(&new_signing_key)
                    .await?,
            });

            // tweak = old - new
            let privkey_tweak = self
                .signer
                .subtract_secrets(&signing_key, &new_signing_key)
                .await?;
            let shares = self
                .signer
                .split_secret_with_proofs(
                    &SecretToSplit::SecretSource(privkey_tweak),
                    threshold,
                    operator_recipients.len(),
                )
                .await?;
            let pubkey_shares_tweak = self.pubkey_shares_tweak(&operator_recipients, &shares)?;

            // The new leaf key, encrypted for the receiver to claim with.
            let secret_cipher = self
                .signer
                .encrypt_secret_for_receiver(&new_secret, &receiver_public_key)
                .await?;

            // Per-leaf signature: leaf_id || transfer_id || secret_cipher.
            let mut payload = Vec::new();
            payload.extend_from_slice(leaf.node.id.to_string().as_bytes());
            payload.extend_from_slice(transfer_id.to_string().as_bytes());
            payload.extend_from_slice(&secret_cipher);
            let signature = self
                .signer
                .sign_message_ecdsa_with_identity_key(&payload)
                .await?;

            for recipient in &operator_recipients {
                let share = find_share(&shares, recipient.id).ok_or_else(|| {
                    SignerError::Generic(format!("Share not found for operator {}", recipient.id))
                })?;
                let tweak = proto::SendLeafKeyTweak {
                    leaf_id: leaf.node.id.to_string(),
                    secret_share_tweak: Some(Self::proto_secret_share(share)),
                    pubkey_shares_tweak: pubkey_shares_tweak.clone().into_iter().collect(),
                    secret_cipher: secret_cipher.clone(),
                    signature: signature.serialize_compact().to_vec(),
                    refund_signature: Vec::new(),
                    direct_refund_signature: Vec::new(),
                    direct_from_cpfp_refund_signature: Vec::new(),
                };
                per_operator
                    .entry(recipient.identifier)
                    .or_default()
                    .push(tweak);
            }
        }

        // ECIES-encrypt each operator's bundle of leaf key tweaks.
        let mut key_tweak_package: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut operator_packages = Vec::with_capacity(operator_recipients.len());
        for recipient in &operator_recipients {
            let leaves_to_send = per_operator
                .remove(&recipient.identifier)
                .unwrap_or_default();
            let proto_bytes = proto::SendLeafKeyTweaks { leaves_to_send }.encode_to_vec();
            let encrypted = self.encrypt_for_operator(&recipient.public_key, &proto_bytes)?;
            key_tweak_package.insert(
                hex::encode(recipient.identifier.serialize()),
                encrypted.clone(),
            );
            operator_packages.push(OperatorPackage {
                operator_identifier: recipient.identifier,
                encrypted_package: encrypted,
            });
        }

        // Transfer-package payload signature: tag || transfer_id || tweak map.
        let signing_payload = TaggedHasher::new(&["spark", "transfer", "signing payload"])
            .add_bytes(&transfer_id_bytes(&transfer_id)?)
            .add_map_string_to_bytes(&key_tweak_package)
            .signable_message();
        let transfer_user_signature = self
            .signer
            .sign_message_ecdsa_with_identity_key(&signing_payload)
            .await?;

        Ok(PreparedTransfer {
            operator_packages,
            new_leaf_keys,
            transfer_user_signature,
        })
    }

    async fn prepare_claim(
        &self,
        request: PrepareClaimRequest,
    ) -> Result<PreparedClaim, SignerError> {
        let PrepareClaimRequest {
            transfer_id,
            sender_identity_public_key: _,
            leaves,
            operator_recipients,
            threshold,
        } = request;

        let mut per_operator: BTreeMap<Identifier, Vec<proto::ClaimLeafKeyTweak>> = BTreeMap::new();
        let mut prepared_leaves = Vec::with_capacity(leaves.len());

        for leaf in &leaves {
            // Incoming leaf key (ECIES-encrypted to our identity key by the
            // sender) and the receiver's new derived key.
            let incoming_key = SecretSource::new_encrypted(leaf.leaf_key_ciphertext.clone());
            let new_signing_key = SecretSource::Derived(leaf.node.id.clone());

            // tweak = incoming - new
            let privkey_tweak = self
                .signer
                .subtract_secrets(&incoming_key, &new_signing_key)
                .await?;
            let shares = self
                .signer
                .split_secret_with_proofs(
                    &SecretToSplit::SecretSource(privkey_tweak),
                    threshold,
                    operator_recipients.len(),
                )
                .await?;
            let pubkey_shares_tweak = self.pubkey_shares_tweak(&operator_recipients, &shares)?;

            for recipient in &operator_recipients {
                let share = find_share(&shares, recipient.id).ok_or_else(|| {
                    SignerError::Generic(format!("Share not found for operator {}", recipient.id))
                })?;
                let tweak = proto::ClaimLeafKeyTweak {
                    leaf_id: leaf.node.id.to_string(),
                    secret_share_tweak: Some(Self::proto_secret_share(share)),
                    pubkey_shares_tweak: pubkey_shares_tweak.clone().into_iter().collect(),
                };
                per_operator
                    .entry(recipient.identifier)
                    .or_default()
                    .push(tweak);
            }

            // FROST-sign the claim refunds with the new receiver key.
            let cpfp_refund = self
                .sign_refund_with_key(
                    &new_signing_key,
                    &leaf.cpfp_refund.sighash,
                    &leaf.cpfp_refund.verifying_key,
                    leaf.cpfp_refund.operator_commitments.clone(),
                )
                .await?;
            let direct_refund = match &leaf.direct_refund {
                Some(job) => Some(
                    self.sign_refund_with_key(
                        &new_signing_key,
                        &job.sighash,
                        &job.verifying_key,
                        job.operator_commitments.clone(),
                    )
                    .await?,
                ),
                None => None,
            };
            let direct_from_cpfp_refund = match &leaf.direct_from_cpfp_refund {
                Some(job) => Some(
                    self.sign_refund_with_key(
                        &new_signing_key,
                        &job.sighash,
                        &job.verifying_key,
                        job.operator_commitments.clone(),
                    )
                    .await?,
                ),
                None => None,
            };

            prepared_leaves.push(PreparedClaimLeaf {
                node_id: leaf.node.id.clone(),
                new_signing_public_key: self
                    .signer
                    .public_key_from_secret(&new_signing_key)
                    .await?,
                cpfp_refund,
                direct_refund,
                direct_from_cpfp_refund,
            });
        }

        // ECIES-encrypt each operator's bundle of claim key tweaks.
        let mut key_tweak_package: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut operator_packages = Vec::with_capacity(operator_recipients.len());
        for recipient in &operator_recipients {
            let leaves_to_receive = per_operator
                .remove(&recipient.identifier)
                .unwrap_or_default();
            let proto_bytes = proto::ClaimLeafKeyTweaks { leaves_to_receive }.encode_to_vec();
            let encrypted = self.encrypt_for_operator(&recipient.public_key, &proto_bytes)?;
            key_tweak_package.insert(
                hex::encode(recipient.identifier.serialize()),
                encrypted.clone(),
            );
            operator_packages.push(OperatorPackage {
                operator_identifier: recipient.identifier,
                encrypted_package: encrypted,
            });
        }

        // Claim-package payload signature: tag || transfer_id || tweak map.
        let signing_payload = TaggedHasher::new(&["spark", "claim", "signing payload"])
            .add_bytes(&transfer_id_bytes(&transfer_id)?)
            .add_map_string_to_bytes(&key_tweak_package)
            .signable_message();
        let claim_user_signature = self
            .signer
            .sign_message_ecdsa_with_identity_key(&signing_payload)
            .await?;

        Ok(PreparedClaim {
            leaves: prepared_leaves,
            operator_packages,
            claim_user_signature,
        })
    }

    async fn prepare_lightning_receive(
        &self,
        request: PrepareLightningReceiveRequest,
    ) -> Result<PreparedLightningReceive, SignerError> {
        let PrepareLightningReceiveRequest {
            operator_recipients,
            threshold,
        } = request;

        // Generate a preimage in-process; only its hash leaves this method.
        let preimage: [u8; PREIMAGE_LEN] = {
            let mut rng = thread_rng();
            let sk = SecretKey::new(&mut rng);
            sk.secret_bytes()
        };
        let payment_hash = sha256::Hash::hash(&preimage).to_byte_array();

        let shares = self
            .signer
            .split_secret_with_proofs(
                &SecretToSplit::Preimage(preimage.to_vec()),
                threshold,
                operator_recipients.len(),
            )
            .await?;

        let mut operator_preimage_packages = Vec::with_capacity(operator_recipients.len());
        for recipient in &operator_recipients {
            let share = find_share(&shares, recipient.id).ok_or_else(|| {
                SignerError::Generic(format!("Share not found for operator {}", recipient.id))
            })?;
            let proto_bytes = Self::proto_secret_share(share).encode_to_vec();
            let encrypted = self.encrypt_for_operator(&recipient.public_key, &proto_bytes)?;
            operator_preimage_packages.push(OperatorPackage {
                operator_identifier: recipient.identifier,
                encrypted_package: encrypted,
            });
        }

        Ok(PreparedLightningReceive {
            payment_hash,
            operator_preimage_packages,
        })
    }

    async fn prepare_static_deposit(
        &self,
        request: PrepareStaticDepositRequest,
    ) -> Result<PreparedStaticDeposit, SignerError> {
        let PrepareStaticDepositRequest {
            index,
            ssp_public_key,
            frost_jobs,
        } = request;

        // Export the static-deposit secret (encrypted) to the SSP.
        let static_secret = self.signer.static_deposit_secret_encrypted(index).await?;
        let SecretSource::Encrypted(encrypted_secret) = static_secret else {
            return Err(SignerError::Generic(
                "static_deposit_secret_encrypted did not return an encrypted secret".to_string(),
            ));
        };
        let exported_secret = self
            .signer
            .encrypt_secret_for_receiver(&encrypted_secret, &ssp_public_key)
            .await?;

        let frost_shares = self.sign_frost(frost_jobs).await?;

        Ok(PreparedStaticDeposit {
            exported_secret,
            frost_shares,
        })
    }

    async fn sign_spark_invoice(
        &self,
        request: SignSparkInvoiceRequest,
    ) -> Result<SignedSparkInvoice, SignerError> {
        let signature = self
            .signer
            .sign_hash_schnorr_with_identity_key(&request.invoice_hash)
            .await?;
        Ok(SignedSparkInvoice { signature })
    }

    async fn prepare_token_transaction(
        &self,
        request: PrepareTokenTransactionRequest,
    ) -> Result<PreparedTokenTransaction, SignerError> {
        let signature = self
            .signer
            .sign_hash_schnorr_with_identity_key(&request.digest)
            .await?;
        Ok(PreparedTokenTransaction { signature })
    }
}
