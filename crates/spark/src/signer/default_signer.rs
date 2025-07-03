use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use bitcoin::bip32::{ChildNumber, DerivationPath, Xpriv};
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::secp256k1::rand::thread_rng;
use bitcoin::secp256k1::{self, Message, SecretKey};
use bitcoin::{
    hashes::{Hash, sha256},
    key::Secp256k1,
    secp256k1::All,
    secp256k1::PublicKey,
};
use frost_core::round1::Nonce;
use frost_secp256k1_tr::keys::{
    EvenY, KeyPackage, PublicKeyPackage, SigningShare, Tweak, VerifyingShare,
};
use frost_secp256k1_tr::round1::{SigningCommitments, SigningNonces};
use frost_secp256k1_tr::round2::SignatureShare;
use frost_secp256k1_tr::{Identifier, SigningPackage, VerifyingKey};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::signer::Secret;
use crate::tree::TreeNodeId;
use crate::{
    Network,
    signer::{Signer, SignerError},
};

use super::VerifiableSecretShare;

const PURPOSE: u32 = 8797555;

fn identity_derivation_path(network: Network) -> DerivationPath {
    DerivationPath::from(vec![
        purpose(),
        coin_type(network),
        ChildNumber::from_hardened_idx(0).expect("Hardened zero is invalid"),
    ])
}

fn signing_derivation_path(network: Network) -> DerivationPath {
    DerivationPath::from(vec![
        purpose(),
        coin_type(network),
        ChildNumber::from_hardened_idx(1).expect("Hardened one is invalid"),
    ])
}

fn deposit_derivation_path(network: Network) -> DerivationPath {
    DerivationPath::from(vec![
        purpose(),
        coin_type(network),
        ChildNumber::from_hardened_idx(2).expect("Hardened two is invalid"),
    ])
}

fn static_deposit_derivation_path(network: Network) -> DerivationPath {
    DerivationPath::from(vec![
        purpose(),
        coin_type(network),
        ChildNumber::from_hardened_idx(3).expect("Hardened three is invalid"),
    ])
}

fn coin_type(network: Network) -> ChildNumber {
    let coin_type: u32 = match network {
        Network::Mainnet => 0,
        _ => 1,
    };
    ChildNumber::from_hardened_idx(coin_type)
        .expect(format!("Hardened coin type {} is invalid", coin_type).as_str())
}

fn purpose() -> ChildNumber {
    ChildNumber::from_hardened_idx(PURPOSE)
        .expect(format!("Hardened purpose {} is invalid", PURPOSE).as_str())
}

fn frost_signing_package(
    user_identifier: Identifier,
    message: &[u8],
    statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
    self_commitment: &SigningCommitments,
    adaptor_public_key: Option<&PublicKey>,
) -> Result<SigningPackage, SignerError> {
    // Clone statechain commitments to add our own commitment
    let mut signing_commitments = statechain_commitments.clone();

    // Create participant groups for the signing operation
    // First group is all statechain participants
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

#[derive(Clone)]
pub struct DefaultSigner {
    identity_key: SecretKey,
    master_key: Xpriv,
    network: Network,
    nonce_commitments: Arc<Mutex<HashMap<Vec<u8>, SigningNonces>>>, // TODO: Nonce commitments are never cleared, is this okay?
    private_key_map: Arc<Mutex<HashMap<PublicKey, SecretKey>>>,     // TODO: Is this really the way?
    secp: Secp256k1<All>,
    signing_master_key: Xpriv,
}

#[derive(Debug, Error)]
pub enum DefaultSignerError {
    #[error("invalid seed")]
    InvalidSeed,

    #[error("key derivation error: {0}")]
    KeyDerivationError(String),
}

impl From<secp256k1::Error> for DefaultSignerError {
    fn from(e: secp256k1::Error) -> Self {
        DefaultSignerError::KeyDerivationError(e.to_string())
    }
}

impl From<bitcoin::bip32::Error> for DefaultSignerError {
    fn from(e: bitcoin::bip32::Error) -> Self {
        DefaultSignerError::KeyDerivationError(e.to_string())
    }
}

impl DefaultSigner {
    pub fn new(seed: &[u8], network: Network) -> Result<Self, DefaultSignerError> {
        let master_key =
            Xpriv::new_master(network, seed).map_err(|_| DefaultSignerError::InvalidSeed)?;
        let secp = Secp256k1::new();
        let identity_key = master_key
            .derive_priv(&secp, &identity_derivation_path(network))?
            .private_key;
        let signing_master_key =
            master_key.derive_priv(&secp, &signing_derivation_path(network))?;
        Ok(DefaultSigner {
            identity_key,
            master_key,
            network,
            nonce_commitments: Arc::new(Mutex::new(HashMap::new())),
            private_key_map: Arc::new(Mutex::new(HashMap::new())),
            secp,
            signing_master_key,
        })
    }
}

impl DefaultSigner {
    fn derive_signing_key(&self, hash: sha256::Hash) -> Result<SecretKey, SignerError> {
        let u32_bytes = hash.as_byte_array()[..4]
            .try_into()
            .map_err(|_| SignerError::InvalidHash)?;
        let index = u32::from_be_bytes(u32_bytes) % 0x80000000;
        let child_number =
            ChildNumber::from_hardened_idx(index).map_err(|_| SignerError::InvalidHash)?;
        let derivation_path = DerivationPath::from(vec![child_number]);
        let child = self
            .signing_master_key
            .derive_priv(&self.secp, &derivation_path)
            .map_err(|e| SignerError::KeyDerivationError(format!("failed to derive child: {}", e)))?
            .private_key;
        Ok(child)
    }
}

#[async_trait::async_trait]
impl Signer for DefaultSigner {
    /// Aggregates FROST (Flexible Round-Optimized Schnorr Threshold) signature shares into a complete signature
    ///
    /// This function takes signature shares from multiple parties (statechain and user),
    /// combines them with the corresponding public keys and commitments, and produces
    /// a single aggregated threshold signature that can be verified using the group's verifying key.
    ///
    /// # Parameters
    /// * `message` - The message being signed
    /// * `statechain_signatures` - Map of identifier to signature shares from statechain participants
    /// * `statechain_public_keys` - Map of identifier to public keys from statechain participants
    /// * `verifying_key` - The group's verifying key used to validate the final signature
    /// * `statechain_commitments` - Map of identifier to commitment values from statechain participants
    /// * `self_commitment` - The local user's commitment value
    /// * `public_key` - The local user's public key
    /// * `self_signature` - The local user's signature share
    /// * `adaptor_public_key` - Optional adaptor public key for adaptor signatures
    ///
    /// # Returns
    /// A complete FROST signature that can be verified against the group's public key
    async fn aggregate_frost(
        &self,
        message: &[u8],
        statechain_signatures: BTreeMap<Identifier, SignatureShare>,
        statechain_public_keys: BTreeMap<Identifier, PublicKey>,
        verifying_key: &PublicKey,
        statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
        self_commitment: &SigningCommitments,
        public_key: &PublicKey,
        self_signature: &SignatureShare,
        adaptor_public_key: Option<&PublicKey>,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        tracing::trace!("default_signer::aggregate_frost");

        // Derive an identifier for the local user
        let user_identifier =
            Identifier::derive("user".as_bytes()).map_err(|_| SignerError::IdentifierError)?;

        // Create a signing package containing commitments, participant groups, message and adaptor
        let signing_package = frost_signing_package(
            user_identifier,
            message,
            statechain_commitments,
            self_commitment,
            adaptor_public_key,
        )?;

        // Combine all signature shares (statechain + user)
        let mut signature_shares = statechain_signatures.clone();
        signature_shares.insert(user_identifier, *self_signature);

        // Build a map of verifying shares for all participants
        let mut verifying_shares = BTreeMap::new();
        // Convert statechain public keys to verifying shares
        for (id, pk) in statechain_public_keys.iter() {
            let verifying_key =
                VerifyingShare::deserialize(pk.serialize().as_slice()).map_err(|e| {
                    SignerError::SerializationError(format!(
                        "Failed to deserialize public key for participant {id:?}: {e} (culprit: {:?})", e.culprit()
                    ))
                })?;
            verifying_shares.insert(*id, verifying_key);
        }

        // Add the user's public key as a verifying share
        verifying_shares.insert(
            user_identifier,
            VerifyingShare::deserialize(public_key.serialize().as_slice()).map_err(|e| {
                SignerError::SerializationError(format!(
                    "Failed to deserialize user public key: {e} (culprit: {:?})",
                    e.culprit()
                ))
            })?,
        );

        let verifying_key = VerifyingKey::deserialize(verifying_key.serialize().as_slice())
            .map_err(|e| {
                SignerError::SerializationError(format!(
                    "Failed to deserialize group verifying key: {e} (culprit: {:?})",
                    e.culprit()
                ))
            })?;

        // Create a public key package with all verifying shares and the group's verifying key
        let public_key_package = PublicKeyPackage::new(verifying_shares, verifying_key);

        tracing::trace!("signing_package: {:?}", signing_package);
        tracing::trace!("signature_shares: {:?}", signature_shares);
        tracing::trace!("public_key_package: {:?}", public_key_package);

        // For taproot signatures, we provide an empty merkle root
        let merkle_root = Vec::new();

        // Aggregate all signature shares into a final signature
        let signature = frost_secp256k1_tr::aggregate_with_tweak(
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
        })?;

        tracing::info!("signature: {:?}", signature);
        Ok(signature)
    }

    fn sign_message_ecdsa_with_identity_key<T: AsRef<[u8]>>(
        &self,
        message: T,
    ) -> Result<Signature, SignerError> {
        let digest = sha256::Hash::hash(message.as_ref());
        let sig = self.secp.sign_ecdsa(
            &Message::from_digest(digest.to_byte_array()),
            &self.identity_key,
        );
        Ok(sig)
    }

    async fn generate_frost_signing_commitments(&self) -> Result<SigningCommitments, SignerError> {
        let mut nonce_commitments = self.nonce_commitments.lock().await;
        let mut rng = thread_rng();

        let binding_sk = SecretKey::new(&mut rng);
        let hiding_sk = SecretKey::new(&mut rng);
        let binding = Nonce::deserialize(&binding_sk.secret_bytes())
            .map_err(|e| SignerError::NonceCreationError(e.to_string()))?;
        let hiding = Nonce::deserialize(&hiding_sk.secret_bytes())
            .map_err(|e| SignerError::NonceCreationError(e.to_string()))?;

        let nonces = SigningNonces::from_nonces(hiding, binding);
        let commitments = nonces.commitments();
        let commitment_bytes = commitments.serialize().map_err(|e| {
            SignerError::SerializationError(format!("failed to serialize commitments: {}", e))
        })?;

        nonce_commitments.insert(commitment_bytes, nonces.clone());

        Ok(*commitments)
    }

    fn get_public_key_for_node(&self, id: &TreeNodeId) -> Result<PublicKey, SignerError> {
        let hash = sha256::Hash::hash(id.to_string().as_bytes());
        let signing_key = self.derive_signing_key(hash)?;
        Ok(signing_key.public_key(&self.secp))
    }
    fn generate_random_public_key(&self) -> Result<PublicKey, SignerError> {
        let (_secret_key, public_key) = self.secp.generate_keypair(&mut thread_rng());
        // TODO: save secret key in memory
        Ok(public_key)
    }

    fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        Ok(self.identity_key.public_key(&self.secp))
    }

    fn subtract_private_keys_given_public_keys(
        &self,
        signing_public_key: &PublicKey,
        new_signing_public_key: &PublicKey,
    ) -> Result<PublicKey, SignerError> {
        // TODO: Implement private key subtraction
        todo!()
    }

    fn split_secret_with_proofs(
        &self,
        secret: &Secret,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<super::VerifiableSecretShare>, SignerError> {
        // TODO: Implement threshold secret sharing with proofs
        todo!()
    }

    fn encrypt_leaf_private_key_ecies(
        &self,
        receiver_public_key: &PublicKey,
        public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        // TODO: Implement ECIES encryption of leaf private key
        todo!()
    }

    fn decrypt_leaf_private_key_ecies(
        &self,
        encrypted_data: &[u8],
    ) -> Result<PublicKey, SignerError> {
        todo!()
    }

    /// Creates a FROST signature share for threshold signing
    ///
    /// This function generates a partial signature (signature share) that will be combined
    /// with other shares from statechain participants to create a complete threshold signature.
    /// It uses pre-generated nonce commitments and the corresponding signing key.
    ///
    /// # Parameters
    /// * `message` - The message being signed
    /// * `public_key` - The public key associated with the local signing key
    /// * `private_as_public_key` - Public key representation of the private key used for signing
    /// * `verifying_key` - The group's verifying key (threshold public key)
    /// * `self_commitment` - The local user's previously generated commitment
    /// * `statechain_commitments` - Map of identifier to commitment values from statechain participants
    /// * `adaptor_public_key` - Optional adaptor public key for adaptor signatures
    ///
    /// # Returns
    /// A signature share that can be combined with other shares to form a complete signature
    ///
    /// # Errors
    /// * `UnknownNonceCommitment` - If the provided commitment doesn't match any stored nonce
    /// * `UnknownKey` - If the public key doesn't correspond to any known private key
    /// * `SerializationError` - If there are issues serializing cryptographic components
    async fn sign_frost(
        &self,
        message: &[u8],
        public_key: &PublicKey,
        private_as_public_key: &PublicKey,
        verifying_key: &PublicKey,
        self_commitment: &SigningCommitments,
        statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
        adaptor_public_key: Option<&PublicKey>,
    ) -> Result<SignatureShare, SignerError> {
        tracing::trace!("default_signer::sign_frost");

        // Derive a deterministic identifier for the local user from the string "user"
        let user_identifier =
            Identifier::derive("user".as_bytes()).map_err(|_| SignerError::IdentifierError)?;

        // Create a signing package containing the message, commitments, and participant groups
        // This is used by the FROST protocol to coordinate the multi-party signing process
        let signing_package = frost_signing_package(
            user_identifier,
            message,
            statechain_commitments,
            self_commitment,
            adaptor_public_key,
        )?;

        // Serialize the commitment to look up the corresponding nonces in our storage
        let serialized_commitment = self_commitment.serialize().map_err(|e| {
            SignerError::SerializationError(format!(
                "failed to serialize self commitment: {e} (culprit: {:?})",
                e.culprit()
            ))
        })?;

        // Retrieve the nonces that were previously generated and stored when creating the commitment
        // These nonces are critical for the security of the Schnorr signature scheme
        let nonce_commitments_guard = self.nonce_commitments.lock().await;
        let signing_nonces = nonce_commitments_guard
            .get(&serialized_commitment)
            .ok_or(SignerError::UnknownNonceCommitment)?;

        // Retrieve the private key corresponding to the provided public key
        // This is the user's secret share of the threshold signing key
        let secret_key = self
            .private_key_map
            .lock()
            .await
            .get(private_as_public_key)
            .cloned()
            .ok_or(SignerError::UnknownKey)?;

        // Convert the Bitcoin secret key to FROST SigningShare format
        // This allows it to be used with the FROST API for creating signature shares
        let signing_share = SigningShare::deserialize(&secret_key.secret_bytes()).map_err(|e| {
            SignerError::SerializationError(format!(
                "Failed to deserialize secret key: {e} (culprit: {:?})",
                e.culprit()
            ))
        })?;

        // Convert the Bitcoin public key to FROST VerifyingShare format
        // This represents the user's public verification key in the threshold scheme
        let verifying_share = VerifyingShare::deserialize(public_key.serialize().as_slice())
            .map_err(|e| {
                SignerError::SerializationError(format!(
                    "Failed to deserialize private as public key: {e} (culprit: {:?})",
                    e.culprit()
                ))
            })?;

        // Convert the group's Bitcoin public key to FROST VerifyingKey format
        // This is the aggregate public key that will verify the final threshold signature
        let verifying_key = VerifyingKey::deserialize(verifying_key.serialize().as_slice())
            .map_err(|e| {
                SignerError::SerializationError(format!(
                    "Failed to deserialize verifying key: {e} (culprit: {:?})",
                    e.culprit()
                ))
            })?;

        // Create a key package containing all the necessary cryptographic material
        // for the user's participation in the threshold signing protocol
        let untweaked_key_package = KeyPackage::new(
            user_identifier,
            signing_share,
            verifying_share,
            verifying_key,
            1, // Minimum signers required (set to 1 for this user's perspective)
        );

        // We don't want to tweak the key with merkle root, but we need to make sure the key is even.
        // Then the total verifying key will need to tweak with the merkle root.
        let merkle_root = Vec::new(); // For taproot signatures, we provide an empty merkle root
        let tweaked_key_package = untweaked_key_package.clone().tweak(Some(&merkle_root));
        let even_y_key_package = untweaked_key_package
            .clone()
            .into_even_y(Some(verifying_key.has_even_y()));
        let key_package = KeyPackage::new(
            *even_y_key_package.identifier(),
            *even_y_key_package.signing_share(),
            *even_y_key_package.verifying_share(),
            *tweaked_key_package.verifying_key(),
            *tweaked_key_package.min_signers(),
        );

        tracing::trace!("signing_package: {:?}", signing_package);
        tracing::trace!("signing_nonces: {:?}", signing_nonces);
        tracing::trace!("key_package: {:?}", key_package);

        // Generate the user's signature share using the FROST round2 signing algorithm
        // This combines the message, nonces, and key package to produce a partial signature
        let signature_share =
            frost_secp256k1_tr::round2::sign(&signing_package, signing_nonces, &key_package)
                .map_err(|e| {
                    SignerError::FrostError(format!(
                        "Failed to sign: {e} (culprit: {:?})",
                        e.culprit()
                    ))
                })?;

        // Return the generated signature share to be combined with other shares
        // from the statechain participants to form a complete threshold signature
        return Ok(signature_share);
    }
}

#[cfg(test)]
mod test {
    use crate::{
        Network,
        signer::default_signer::{
            deposit_derivation_path, identity_derivation_path, signing_derivation_path,
            static_deposit_derivation_path,
        },
    };

    /// Ensure constants are defined correctly and don't panic.
    #[test]
    fn test_constant_derivation_paths() {
        identity_derivation_path(Network::Mainnet);
        identity_derivation_path(Network::Testnet);
        identity_derivation_path(Network::Regtest);
        identity_derivation_path(Network::Signet);

        signing_derivation_path(Network::Mainnet);
        signing_derivation_path(Network::Testnet);
        signing_derivation_path(Network::Regtest);
        signing_derivation_path(Network::Signet);

        deposit_derivation_path(Network::Mainnet);
        deposit_derivation_path(Network::Testnet);
        deposit_derivation_path(Network::Regtest);
        deposit_derivation_path(Network::Signet);

        static_deposit_derivation_path(Network::Mainnet);
        static_deposit_derivation_path(Network::Testnet);
        static_deposit_derivation_path(Network::Regtest);
        static_deposit_derivation_path(Network::Signet);
    }
}
