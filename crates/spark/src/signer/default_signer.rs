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

use crate::signer::{EncryptedPrivateKey, secret_sharing};
use crate::signer::{PrivateKeySource, SecretToSplit};
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
        Network::Regtest => 0,
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
            secp,
            signing_master_key,
        })
    }
}

impl DefaultSigner {
    fn derive_signing_key(&self, node_id: &TreeNodeId) -> Result<SecretKey, SignerError> {
        let hash = sha256::Hash::hash(node_id.to_string().as_bytes());
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

    fn encrypt_private_key_ecies(
        &self,
        private_key: &SecretKey,
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        let ciphertext = ecies::encrypt(
            &receiver_public_key.serialize(),
            &private_key.secret_bytes(),
        )
        .map_err(|e| SignerError::Generic(format!("failed to encrypt: {}", e)))?;
        Ok(ciphertext)
    }

    fn decrypt_private_key_ecies(&self, ciphertext: &[u8]) -> Result<SecretKey, SignerError> {
        let plaintext = ecies::decrypt(&self.identity_key.secret_bytes(), ciphertext)
            .map_err(|e| SignerError::Generic(format!("failed to decrypt: {}", e)))?;
        let secret_key = SecretKey::from_slice(&plaintext).map_err(|e| {
            SignerError::Generic(format!("failed to deserialize secret key: {}", e))
        })?;
        Ok(secret_key)
    }
}

#[async_trait::async_trait]
impl Signer for DefaultSigner {
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
        let signing_key = self.derive_signing_key(id)?;
        let public_key = signing_key.public_key(&self.secp);
        Ok(public_key)
    }

    fn generate_random_key(&self) -> Result<PrivateKeySource, SignerError> {
        let (secret_key, _) = self.secp.generate_keypair(&mut thread_rng());
        Ok(PrivateKeySource::new_encrypted(
            self.encrypt_private_key_ecies(&secret_key, &self.get_identity_public_key()?)?,
        ))
    }

    fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        Ok(self.identity_key.public_key(&self.secp))
    }

    fn subtract_private_keys(
        &self,
        signing_key: &PrivateKeySource,
        new_signing_key: &PrivateKeySource,
    ) -> Result<PrivateKeySource, SignerError> {
        let signing_key = signing_key.to_secret_key(self)?;
        let new_signing_key = new_signing_key.to_secret_key(self)?;

        if signing_key == new_signing_key {
            return Err(SignerError::Generic(
                "Signing key and new signing key are the same".to_string(),
            ));
        }

        let res = signing_key
            .add_tweak(&new_signing_key.negate().into())
            .map_err(|e| SignerError::Generic(format!("failed to add tweak: {}", e)))?;

        let ciphertext = self.encrypt_private_key_ecies(&res, &self.get_identity_public_key()?)?;

        Ok(PrivateKeySource::new_encrypted(ciphertext))
    }

    fn encrypt_private_key_for_receiver(
        &self,
        private_key: &EncryptedPrivateKey,
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        let private_key = PrivateKeySource::Encrypted(private_key.clone()).to_secret_key(self)?;

        self.encrypt_private_key_ecies(&private_key, receiver_public_key)
    }

    fn get_public_key_from_private_key_source(
        &self,
        private_key: &PrivateKeySource,
    ) -> Result<PublicKey, SignerError> {
        let private_key = private_key.to_secret_key(self)?;
        Ok(private_key.public_key(&self.secp))
    }

    fn split_secret_with_proofs(
        &self,
        secret: &SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<VerifiableSecretShare>, SignerError> {
        let secret_bytes = match secret {
            SecretToSplit::PrivateKey(privkey_source) => {
                privkey_source.to_secret_key(self)?.secret_bytes().to_vec()
            }
            SecretToSplit::Preimage(bytes) => bytes.clone(),
        };
        let secret_as_scalar = secret_sharing::from_bytes_to_scalar(&secret_bytes)?;
        let shares = secret_sharing::split_secret_with_proofs(
            &secret_as_scalar,
            threshold as usize,
            num_shares,
        )?;

        Ok(shares)
    }

    async fn sign_frost(
        &self,
        message: &[u8],
        public_key: &PublicKey,
        private_key: &PrivateKeySource,
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

        let secret_key = private_key.to_secret_key(self)?;

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
}

impl PrivateKeySource {
    fn to_secret_key(&self, signer: &DefaultSigner) -> Result<SecretKey, SignerError> {
        match self {
            PrivateKeySource::Derived(node_id) => signer.derive_signing_key(node_id),
            PrivateKeySource::Encrypted(ciphertext) => {
                signer.decrypt_private_key_ecies(ciphertext.as_slice())
            }
        }
    }
}

#[cfg(test)]
mod test {
    use bitcoin::secp256k1::rand::thread_rng;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use std::str::FromStr;

    use crate::signer::{EncryptedPrivateKey, PrivateKeySource, Signer, SignerError};
    use crate::tree::TreeNodeId;
    use crate::{
        Network,
        signer::default_signer::DefaultSigner,
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

    fn create_test_signer() -> DefaultSigner {
        let test_seed = [42u8; 32]; // Deterministic seed for testing
        DefaultSigner::new(&test_seed, Network::Regtest).expect("Failed to create test signer")
    }

    #[test]
    fn test_encrypt_decrypt_private_key_ecies_round_trip() {
        let signer = create_test_signer();
        let secp = Secp256k1::new();
        let mut rng = thread_rng();

        // Generate a test private key to encrypt
        let test_private_key = SecretKey::new(&mut rng);

        // Get the signer's identity public key (receiver)
        let receiver_public_key = signer
            .get_identity_public_key()
            .expect("Failed to get identity public key");

        // Encrypt the private key
        let ciphertext = signer
            .encrypt_private_key_ecies(&test_private_key, &receiver_public_key)
            .expect("Failed to encrypt private key");

        // Verify ciphertext is not empty
        assert!(!ciphertext.is_empty());

        // Decrypt the private key
        let decrypted_private_key = signer
            .decrypt_private_key_ecies(&ciphertext)
            .expect("Failed to decrypt private key");

        // Verify the decrypted key matches the original
        assert_eq!(
            test_private_key.secret_bytes(),
            decrypted_private_key.secret_bytes()
        );

        // Verify the public keys match
        let original_public_key = test_private_key.public_key(&secp);
        let decrypted_public_key = decrypted_private_key.public_key(&secp);
        assert_eq!(original_public_key, decrypted_public_key);
    }

    #[test]
    fn test_subtract_private_keys_success() {
        let signer = create_test_signer();
        let secp = Secp256k1::new();
        let mut rng = thread_rng();

        // Generate two different private keys
        let key_a = SecretKey::new(&mut rng);
        let key_b = SecretKey::new(&mut rng);

        // Encrypt both keys using the signer's identity public key
        let identity_public_key = signer
            .get_identity_public_key()
            .expect("Failed to get identity public key");

        let encrypted_a = signer
            .encrypt_private_key_ecies(&key_a, &identity_public_key)
            .expect("Failed to encrypt key A");
        let encrypted_b = signer
            .encrypt_private_key_ecies(&key_b, &identity_public_key)
            .expect("Failed to encrypt key B");

        let source_a = PrivateKeySource::new_encrypted(encrypted_a);
        let source_b = PrivateKeySource::new_encrypted(encrypted_b);

        // Perform subtraction: A - B = C
        let result = signer
            .subtract_private_keys(&source_a, &source_b)
            .expect("Failed to subtract private keys");

        // Verify result is encrypted
        assert!(matches!(result, PrivateKeySource::Encrypted(_)));

        // Verify mathematical correctness: C + B should equal A
        let result_key = result
            .to_secret_key(&signer)
            .expect("Failed to decrypt result");
        let reconstructed_a = result_key
            .add_tweak(&key_b.into())
            .expect("Failed to add tweak");

        assert_eq!(key_a.secret_bytes(), reconstructed_a.secret_bytes());
    }

    #[test]
    fn test_subtract_private_keys_same_key_error() {
        let signer = create_test_signer();
        let mut rng = thread_rng();

        // Generate a private key
        let key = SecretKey::new(&mut rng);

        // Encrypt the key using the signer's identity public key
        let identity_public_key = signer
            .get_identity_public_key()
            .expect("Failed to get identity public key");

        let encrypted_key = signer
            .encrypt_private_key_ecies(&key, &identity_public_key)
            .expect("Failed to encrypt key");

        let source = PrivateKeySource::new_encrypted(encrypted_key);

        // Try to subtract the same key from itself
        let result = signer.subtract_private_keys(&source, &source);

        // Should return an error
        assert!(result.is_err());
        if let Err(SignerError::Generic(msg)) = result {
            assert_eq!(msg, "Signing key and new signing key are the same");
        } else {
            panic!("Expected Generic error about same keys");
        }
    }

    #[test]
    fn test_encrypt_private_key_for_receiver_success() {
        let signer = create_test_signer();
        let mut rng = thread_rng();

        // Generate a private key and encrypt it with identity key
        let private_key = SecretKey::new(&mut rng);
        let identity_public_key = signer
            .get_identity_public_key()
            .expect("Failed to get identity public key");

        let encrypted_private_key = signer
            .encrypt_private_key_ecies(&private_key, &identity_public_key)
            .expect("Failed to encrypt private key");

        // Generate receiver's key pair
        let receiver_private_key = SecretKey::new(&mut rng);
        let receiver_public_key = receiver_private_key.public_key(&signer.secp);

        // Encrypt for receiver
        let result = signer
            .encrypt_private_key_for_receiver(
                &EncryptedPrivateKey::new(encrypted_private_key),
                &receiver_public_key,
            )
            .expect("Failed to encrypt for receiver");

        // Verify result is not empty
        assert!(!result.is_empty());

        // Verify receiver can decrypt it
        let decrypted = ecies::decrypt(&receiver_private_key.secret_bytes(), &result)
            .expect("Failed to decrypt with receiver key");
        let decrypted_key =
            SecretKey::from_slice(&decrypted).expect("Failed to parse decrypted key");

        assert_eq!(private_key.secret_bytes(), decrypted_key.secret_bytes());
    }

    #[test]
    fn test_get_public_key_from_private_key_source() {
        let signer = create_test_signer();
        let secp = Secp256k1::new();
        let mut rng = thread_rng();

        // Test with encrypted private key source
        let private_key = SecretKey::new(&mut rng);
        let expected_public_key = private_key.public_key(&secp);

        let identity_public_key = signer
            .get_identity_public_key()
            .expect("Failed to get identity public key");

        let encrypted_private_key = signer
            .encrypt_private_key_ecies(&private_key, &identity_public_key)
            .expect("Failed to encrypt private key");

        let encrypted_source = PrivateKeySource::new_encrypted(encrypted_private_key);

        let result_public_key = signer
            .get_public_key_from_private_key_source(&encrypted_source)
            .expect("Failed to get public key from encrypted source");

        assert_eq!(expected_public_key, result_public_key);

        // Test with derived private key source
        let node_id = TreeNodeId::from_str("test_node").expect("Failed to create node ID");
        let derived_source = PrivateKeySource::Derived(node_id.clone());

        let result_public_key = signer
            .get_public_key_from_private_key_source(&derived_source)
            .expect("Failed to get public key from derived source");

        // Verify it matches what get_public_key_for_node returns
        let expected_public_key = signer
            .get_public_key_for_node(&node_id)
            .expect("Failed to get public key for node");

        assert_eq!(expected_public_key, result_public_key);
    }
}
