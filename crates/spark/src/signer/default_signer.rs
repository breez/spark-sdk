use std::collections::{BTreeMap, BTreeSet};

use bitcoin::bip32::{ChildNumber, DerivationPath, Xpriv};
use bitcoin::key::{Parity, TapTweak};
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::secp256k1::rand::thread_rng;
use bitcoin::secp256k1::{self, All, Keypair, Message, PublicKey, SecretKey, schnorr};
use bitcoin::{
    hashes::{Hash, sha256},
    key::Secp256k1,
};
use frost_core::round1::Nonce;
use frost_secp256k1_tr::keys::{
    EvenY, KeyPackage, PublicKeyPackage, SigningShare, Tweak, VerifyingShare,
};
use frost_secp256k1_tr::round1::{SigningCommitments, SigningNonces};
use frost_secp256k1_tr::round2::SignatureShare;
use frost_secp256k1_tr::{Identifier, SigningPackage, VerifyingKey};
use thiserror::Error;

use crate::signer::{
    AggregateFrostRequest, EncryptedSecret, FrostSigningCommitmentsWithNonces, SignFrostRequest,
    secret_sharing,
};
use crate::signer::{SecretSource, SecretToSplit};
use crate::tree::TreeNodeId;
use crate::{
    Network,
    signer::{Signer, SignerError},
};

use super::VerifiableSecretShare;

fn account_number(network: Network) -> u32 {
    match network {
        Network::Regtest => 0,
        _ => 1,
    }
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum KeySetType {
    #[default]
    Default,
    Taproot,
    NativeSegwit,
    WrappedSegwit,
    Legacy,
}

struct DerivedKeySet {
    derivation_path: DerivationPath,
    master_key: Xpriv,
}

impl DerivedKeySet {
    fn new(
        seed: &[u8],
        network: Network,
        derivation_path: DerivationPath,
    ) -> Result<Self, bitcoin::bip32::Error> {
        let master_key = Xpriv::new_master(network, seed)?;

        Ok(DerivedKeySet {
            derivation_path,
            master_key,
        })
    }

    fn to_key_set(
        &self,
        identity_child_number: Option<ChildNumber>,
    ) -> Result<KeySet, DefaultSignerError> {
        let secp = Secp256k1::new();
        let mut identity_master_key = self.master_key.derive_priv(&secp, &self.derivation_path)?;

        let signing_master_key =
            identity_master_key.derive_priv(&secp, &[ChildNumber::from_hardened_idx(1)?])?;
        let static_deposit_master_key =
            identity_master_key.derive_priv(&secp, &[ChildNumber::from_hardened_idx(3)?])?;
        let encryption_master_key = identity_master_key
            .derive_priv(&secp, &[ChildNumber::from_hardened_idx(712532575)?])?;
        if let Some(child_number) = identity_child_number {
            identity_master_key =
                identity_master_key.derive_priv(&secp, &DerivationPath::from(vec![child_number]))?
        }
        Ok(KeySet {
            identity_key_pair: identity_master_key.private_key.keypair(&secp),
            identity_master_key,
            encryption_master_key,
            signing_master_key,
            static_deposit_master_key,
        })
    }
}

#[derive(Clone)]
pub struct KeySet {
    pub identity_key_pair: Keypair,
    pub identity_master_key: Xpriv,
    pub encryption_master_key: Xpriv,
    pub signing_master_key: Xpriv,
    pub static_deposit_master_key: Xpriv,
}

impl KeySet {
    pub fn new(
        seed: &[u8],
        network: Network,
        key_type: KeySetType,
        use_address_index: bool,
        account_no: Option<u32>,
    ) -> Result<Self, DefaultSignerError> {
        let account_number = account_no.unwrap_or_else(|| account_number(network));
        match key_type {
            KeySetType::Default => Self::default_keys(seed, network, account_number),
            KeySetType::Taproot => {
                Self::taproot_keys(seed, network, use_address_index, account_number)
            }
            KeySetType::NativeSegwit => {
                Self::native_segwit_keys(seed, network, use_address_index, account_number)
            }
            KeySetType::WrappedSegwit => {
                Self::wrapped_segwit_keys(seed, network, use_address_index, account_number)
            }
            KeySetType::Legacy => {
                Self::legacy_bitcoin_keys(seed, network, use_address_index, account_number)
            }
        }
    }

    fn default_keys(
        seed: &[u8],
        network: Network,
        account_number: u32,
    ) -> Result<Self, DefaultSignerError> {
        let derivation_path = format!("m/8797555'/{account_number}'").parse()?;
        let derived_key_set = DerivedKeySet::new(seed, network, derivation_path)?;
        derived_key_set.to_key_set(ChildNumber::from_hardened_idx(0).ok())
    }

    fn taproot_keys(
        seed: &[u8],
        network: Network,
        use_address_index: bool,
        account_number: u32,
    ) -> Result<Self, DefaultSignerError> {
        let derivation_path = if use_address_index {
            format!("m/86'/0'/0'/0/{account_number}")
        } else {
            format!("m/86'/0'/{account_number}'/0/0")
        }
        .parse()?;
        let derived_key_set = DerivedKeySet::new(seed, network, derivation_path)?;
        let mut key_set = derived_key_set.to_key_set(None)?;
        let secp = Secp256k1::new();
        key_set.identity_key_pair = key_set
            .identity_key_pair
            .tap_tweak(&secp, None)
            .to_keypair();
        if let (_, Parity::Odd) = key_set
            .identity_key_pair
            .secret_key()
            .x_only_public_key(&secp)
        {
            key_set.identity_key_pair = key_set
                .identity_key_pair
                .secret_key()
                .negate()
                .keypair(&secp)
        }

        Ok(key_set)
    }

    fn native_segwit_keys(
        seed: &[u8],
        network: Network,
        use_address_index: bool,
        account_number: u32,
    ) -> Result<Self, DefaultSignerError> {
        let derivation_path = if use_address_index {
            format!("m/84'/0'/0'/0/{account_number}")
        } else {
            format!("m/84'/0'/{account_number}'/0/0")
        }
        .parse()?;
        let derived_key_set = DerivedKeySet::new(seed, network, derivation_path)?;
        derived_key_set.to_key_set(None)
    }

    fn wrapped_segwit_keys(
        seed: &[u8],
        network: Network,
        use_address_index: bool,
        account_number: u32,
    ) -> Result<Self, DefaultSignerError> {
        let derivation_path = if use_address_index {
            format!("m/49'/0'/0'/0/{account_number}")
        } else {
            format!("m/49'/0'/{account_number}'/0/0")
        }
        .parse()?;
        let derived_key_set = DerivedKeySet::new(seed, network, derivation_path)?;
        derived_key_set.to_key_set(None)
    }

    fn legacy_bitcoin_keys(
        seed: &[u8],
        network: Network,
        use_address_index: bool,
        account_number: u32,
    ) -> Result<Self, DefaultSignerError> {
        let derivation_path = if use_address_index {
            format!("m/44'/0'/0'/0/{account_number}")
        } else {
            format!("m/44'/0'/{account_number}'/0/0")
        }
        .parse()?;
        let derived_key_set = DerivedKeySet::new(seed, network, derivation_path)?;
        derived_key_set.to_key_set(None)
    }
}

#[derive(Clone)]
pub struct DefaultSigner {
    key_set: KeySet,
    secp: Secp256k1<All>,
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
        Ok(Self::from_key_set(KeySet::new(
            seed,
            network,
            KeySetType::Default,
            false,
            None,
        )?))
    }

    pub fn from_key_set(key_set: KeySet) -> Self {
        let secp = Secp256k1::new();
        DefaultSigner { key_set, secp }
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
            .key_set
            .signing_master_key
            .derive_priv(&self.secp, &derivation_path)
            .map_err(|e| SignerError::KeyDerivationError(format!("failed to derive child: {e}")))?
            .private_key;
        Ok(child)
    }

    fn encrypt_message_ecies(
        &self,
        message: &[u8],
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        ecies::encrypt(&receiver_public_key.serialize(), message)
            .map_err(|e| SignerError::Generic(format!("failed to encrypt: {e}")))
    }

    fn decrypt_message_ecies(&self, ciphertext: &[u8]) -> Result<Vec<u8>, SignerError> {
        ecies::decrypt(&self.key_set.identity_key_pair.secret_bytes(), ciphertext)
            .map_err(|e| SignerError::Generic(format!("failed to decrypt: {e}")))
    }

    fn encrypt_private_key_ecies(
        &self,
        private_key: &SecretKey,
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        let ciphertext =
            self.encrypt_message_ecies(&private_key.secret_bytes(), receiver_public_key)?;
        Ok(ciphertext)
    }

    fn decrypt_private_key_ecies(&self, ciphertext: &[u8]) -> Result<SecretKey, SignerError> {
        let plaintext = self.decrypt_message_ecies(ciphertext)?;
        let secret_key = SecretKey::from_slice(&plaintext)
            .map_err(|e| SignerError::Generic(format!("failed to deserialize secret key: {e}")))?;
        Ok(secret_key)
    }

    fn encrypt_nonces_ecies(
        &self,
        nonces: &SigningNonces,
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        let nonces_bytes = nonces.serialize().map_err(|e| {
            SignerError::SerializationError(format!("failed to serialize nonces: {e}"))
        })?;
        self.encrypt_message_ecies(&nonces_bytes, receiver_public_key)
    }

    fn decrypt_nonces_ecies(&self, ciphertext: &[u8]) -> Result<SigningNonces, SignerError> {
        let plaintext = self.decrypt_message_ecies(ciphertext)?;
        let nonces = SigningNonces::deserialize(&plaintext).map_err(|e| {
            SignerError::SerializationError(format!("failed to deserialize nonces: {e}"))
        })?;
        Ok(nonces)
    }
}

#[macros::async_trait]
impl Signer for DefaultSigner {
    async fn sign_message_ecdsa_with_identity_key(
        &self,
        message: &[u8],
    ) -> Result<Signature, SignerError> {
        let digest = sha256::Hash::hash(message);
        let sig = self.secp.sign_ecdsa(
            &Message::from_digest(digest.to_byte_array()),
            &self.key_set.identity_key_pair.secret_key(),
        );
        Ok(sig)
    }

    async fn sign_hash_schnorr_with_identity_key(
        &self,
        hash: &[u8],
    ) -> Result<schnorr::Signature, SignerError> {
        if hash.len() != 32 {
            return Err(SignerError::Generic(
                "Hash must be exactly 32 bytes".to_string(),
            ));
        }
        let mut hash_array = [0u8; 32];
        hash_array.copy_from_slice(hash);
        // Always use auxiliary randomness for enhanced security
        let mut rng = thread_rng();
        let sig = self.secp.sign_schnorr_with_rng(
            &Message::from_digest(hash_array),
            &self.key_set.identity_key_pair,
            &mut rng,
        );
        Ok(sig)
    }

    async fn generate_random_signing_commitment(
        &self,
    ) -> Result<FrostSigningCommitmentsWithNonces, SignerError> {
        let (binding_sk, hiding_sk) = {
            let mut rng = thread_rng();
            (SecretKey::new(&mut rng), SecretKey::new(&mut rng))
        };
        let binding = Nonce::deserialize(&binding_sk.secret_bytes())
            .map_err(|e| SignerError::NonceCreationError(e.to_string()))?;
        let hiding = Nonce::deserialize(&hiding_sk.secret_bytes())
            .map_err(|e| SignerError::NonceCreationError(e.to_string()))?;

        let nonces = SigningNonces::from_nonces(hiding, binding);
        let nonces_ciphertext =
            self.encrypt_nonces_ecies(&nonces, &self.get_identity_public_key().await?)?;
        let commitments = *nonces.commitments();

        Ok(FrostSigningCommitmentsWithNonces {
            commitments,
            nonces_ciphertext,
        })
    }

    async fn get_public_key_for_node(&self, id: &TreeNodeId) -> Result<PublicKey, SignerError> {
        let signing_key = self.derive_signing_key(id)?;
        let public_key = signing_key.public_key(&self.secp);
        Ok(public_key)
    }

    async fn generate_random_key(&self) -> Result<SecretSource, SignerError> {
        let (secret_key, _) = self.secp.generate_keypair(&mut thread_rng());
        Ok(SecretSource::new_encrypted(
            self.encrypt_private_key_ecies(&secret_key, &self.get_identity_public_key().await?)?,
        ))
    }

    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        Ok(self.key_set.identity_key_pair.public_key())
    }

    async fn static_deposit_secret_encrypted(
        &self,
        index: u32,
    ) -> Result<SecretSource, SignerError> {
        let secret_key = self.static_deposit_secret(index).await?;
        Ok(SecretSource::new_encrypted(
            self.encrypt_private_key_ecies(&secret_key, &self.get_identity_public_key().await?)?,
        ))
    }

    async fn static_deposit_secret(&self, index: u32) -> Result<SecretKey, SignerError> {
        let child_number = ChildNumber::from_hardened_idx(index).map_err(|e| {
            SignerError::Generic(format!("failed to create child from {index}: {e}"))
        })?;
        let derivation_path = DerivationPath::from(vec![child_number]);
        let private_key = self
            .key_set
            .static_deposit_master_key
            .derive_priv(&self.secp, &derivation_path)
            .map_err(|e| SignerError::KeyDerivationError(format!("failed to derive child: {e}")))?
            .private_key;
        Ok(private_key)
    }

    async fn static_deposit_signing_key(&self, index: u32) -> Result<PublicKey, SignerError> {
        let public_key = self
            .static_deposit_secret(index)
            .await?
            .public_key(&self.secp);
        Ok(public_key)
    }

    async fn subtract_secrets(
        &self,
        signing_key: &SecretSource,
        new_signing_key: &SecretSource,
    ) -> Result<SecretSource, SignerError> {
        let signing_key = signing_key.to_secret_key(self)?;
        let new_signing_key = new_signing_key.to_secret_key(self)?;

        if signing_key == new_signing_key {
            return Err(SignerError::Generic(
                "Signing key and new signing key are the same".to_string(),
            ));
        }

        let res = signing_key
            .add_tweak(&new_signing_key.negate().into())
            .map_err(|e| SignerError::Generic(format!("failed to add tweak: {e}")))?;

        let ciphertext =
            self.encrypt_private_key_ecies(&res, &self.get_identity_public_key().await?)?;

        Ok(SecretSource::new_encrypted(ciphertext))
    }

    async fn encrypt_secret_for_receiver(
        &self,
        private_key: &EncryptedSecret,
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        let private_key = SecretSource::Encrypted(private_key.clone()).to_secret_key(self)?;

        self.encrypt_private_key_ecies(&private_key, receiver_public_key)
    }

    async fn public_key_from_secret(
        &self,
        private_key: &SecretSource,
    ) -> Result<PublicKey, SignerError> {
        let private_key = private_key.to_secret_key(self)?;
        Ok(private_key.public_key(&self.secp))
    }

    async fn split_secret_with_proofs(
        &self,
        secret: &SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<VerifiableSecretShare>, SignerError> {
        let secret_bytes = match secret {
            SecretToSplit::SecretSource(privkey_source) => {
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

    async fn sign_frost<'a>(
        &self,
        request: SignFrostRequest<'a>,
    ) -> Result<SignatureShare, SignerError> {
        tracing::trace!("default_signer::sign_frost");

        // Derive a deterministic identifier for the local user from the string "user"
        let user_identifier =
            Identifier::derive("user".as_bytes()).map_err(|_| SignerError::IdentifierError)?;

        // Create a signing package containing the message, commitments, and participant groups
        // This is used by the FROST protocol to coordinate the multi-party signing process
        let signing_package = frost_signing_package(
            user_identifier,
            request.message,
            request.statechain_commitments,
            &request.self_nonce_commitment.commitments,
            request.adaptor_public_key,
        )?;

        // Decrypt the nonces that were previously generated when creating the commitment
        // These nonces are critical for the security of the Schnorr signature scheme
        let signing_nonces =
            self.decrypt_nonces_ecies(&request.self_nonce_commitment.nonces_ciphertext)?;

        let secret_key = request.private_key.to_secret_key(self)?;

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
        let verifying_share = VerifyingShare::deserialize(
            request.public_key.serialize().as_slice(),
        )
        .map_err(|e| {
            SignerError::SerializationError(format!(
                "Failed to deserialize private as public key: {e} (culprit: {:?})",
                e.culprit()
            ))
        })?;

        // Convert the group's Bitcoin public key to FROST VerifyingKey format
        // This is the aggregate public key that will verify the final threshold signature
        let verifying_key = VerifyingKey::deserialize(request.verifying_key.serialize().as_slice())
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
            frost_secp256k1_tr::round2::sign(&signing_package, &signing_nonces, &key_package)
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

    async fn aggregate_frost<'a>(
        &self,
        request: AggregateFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        tracing::trace!("default_signer::aggregate_frost");

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
                        "Failed to deserialize public key for participant {id:?}: {e} (culprit: {:?})", e.culprit()
                    ))
                })?;
            verifying_shares.insert(*id, verifying_key);
        }

        // Add the user's public key as a verifying share
        verifying_shares.insert(
            user_identifier,
            VerifyingShare::deserialize(request.public_key.serialize().as_slice()).map_err(
                |e| {
                    SignerError::SerializationError(format!(
                        "Failed to deserialize user public key: {e} (culprit: {:?})",
                        e.culprit()
                    ))
                },
            )?,
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

        tracing::debug!("signature: {:?}", signature);
        Ok(signature)
    }
}

impl SecretSource {
    fn to_secret_key(&self, signer: &DefaultSigner) -> Result<SecretKey, SignerError> {
        match self {
            SecretSource::Derived(node_id) => signer.derive_signing_key(node_id),
            SecretSource::Encrypted(ciphertext) => {
                signer.decrypt_private_key_ecies(ciphertext.as_slice())
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use bitcoin::secp256k1::rand::thread_rng;
    use bitcoin::secp256k1::{self, PublicKey, Secp256k1, SecretKey};
    use macros::async_test_all;
    use std::str::FromStr;

    use crate::signer::{EncryptedSecret, SecretSource, Signer, SignerError};
    use crate::tree::TreeNodeId;
    use crate::utils::verify_signature::verify_signature_ecdsa;
    use crate::{Network, signer::default_signer::DefaultSigner};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    pub(crate) fn create_test_signer() -> DefaultSigner {
        let test_seed = [42u8; 32]; // Deterministic seed for testing
        DefaultSigner::new(&test_seed, Network::Regtest).expect("Failed to create test signer")
    }

    #[async_test_all]
    async fn test_sign_verify_signature_ecdsa_round_trip() {
        let signer = create_test_signer();
        let message = "test message";
        let signature = signer
            .sign_message_ecdsa_with_identity_key(message.as_bytes())
            .await
            .expect("Failed to sign message");

        verify_signature_ecdsa(
            &signer.secp,
            message,
            &signature,
            &signer.get_identity_public_key().await.unwrap(),
        )
        .expect("Failed to verify signature");
    }

    #[async_test_all]
    async fn test_verify_signature_ecdsa_invalid_signature() {
        let signer = create_test_signer();
        let signature = signer
            .sign_message_ecdsa_with_identity_key("signed message".as_bytes())
            .await
            .expect("Failed to sign message");

        // Wrong message
        let result = verify_signature_ecdsa(
            &signer.secp,
            "another message",
            &signature,
            &signer.get_identity_public_key().await.unwrap(),
        );
        assert!(result.is_err());
        assert!(matches!(result, Err(secp256k1::Error::IncorrectSignature)));

        // Wrong public key
        let result = verify_signature_ecdsa(
            &signer.secp,
            "signed message",
            &signature,
            &PublicKey::from_secret_key(&Secp256k1::new(), &SecretKey::new(&mut thread_rng())),
        );
        assert!(result.is_err());
        assert!(matches!(result, Err(secp256k1::Error::IncorrectSignature)));
    }

    #[async_test_all]
    async fn test_encrypt_decrypt_private_key_ecies_round_trip() {
        let signer = create_test_signer();
        let secp = Secp256k1::new();
        let mut rng = thread_rng();

        // Generate a test private key to encrypt
        let test_private_key = SecretKey::new(&mut rng);

        // Get the signer's identity public key (receiver)
        let receiver_public_key = signer
            .get_identity_public_key()
            .await
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

    #[async_test_all]
    async fn test_subtract_secrets_success() {
        let signer = create_test_signer();
        let mut rng = thread_rng();

        // Generate two different private keys
        let key_a = SecretKey::new(&mut rng);
        let key_b = SecretKey::new(&mut rng);

        // Encrypt both keys using the signer's identity public key
        let identity_public_key = signer
            .get_identity_public_key()
            .await
            .expect("Failed to get identity public key");

        let encrypted_a = signer
            .encrypt_private_key_ecies(&key_a, &identity_public_key)
            .expect("Failed to encrypt key A");
        let encrypted_b = signer
            .encrypt_private_key_ecies(&key_b, &identity_public_key)
            .expect("Failed to encrypt key B");

        let source_a = SecretSource::new_encrypted(encrypted_a);
        let source_b = SecretSource::new_encrypted(encrypted_b);

        // Perform subtraction: A - B = C
        let result = signer
            .subtract_secrets(&source_a, &source_b)
            .await
            .expect("Failed to subtract private keys");

        // Verify result is encrypted
        assert!(matches!(result, SecretSource::Encrypted(_)));

        // Verify mathematical correctness: C + B should equal A
        let result_key = result
            .to_secret_key(&signer)
            .expect("Failed to decrypt result");
        let reconstructed_a = result_key
            .add_tweak(&key_b.into())
            .expect("Failed to add tweak");

        assert_eq!(key_a.secret_bytes(), reconstructed_a.secret_bytes());
    }

    #[async_test_all]
    async fn test_subtract_secrets_same_key_error() {
        let signer = create_test_signer();
        let mut rng = thread_rng();

        // Generate a private key
        let key = SecretKey::new(&mut rng);

        // Encrypt the key using the signer's identity public key
        let identity_public_key = signer
            .get_identity_public_key()
            .await
            .expect("Failed to get identity public key");

        let encrypted_key = signer
            .encrypt_private_key_ecies(&key, &identity_public_key)
            .expect("Failed to encrypt key");

        let source = SecretSource::new_encrypted(encrypted_key);

        // Try to subtract the same key from itself
        let result = signer.subtract_secrets(&source, &source).await;

        // Should return an error
        assert!(result.is_err());
        if let Err(SignerError::Generic(msg)) = result {
            assert_eq!(msg, "Signing key and new signing key are the same");
        } else {
            panic!("Expected Generic error about same keys");
        }
    }

    #[async_test_all]
    async fn test_encrypt_secret_for_receiver_success() {
        let signer = create_test_signer();
        let mut rng = thread_rng();

        // Generate a private key and encrypt it with identity key
        let private_key = SecretKey::new(&mut rng);
        let identity_public_key = signer
            .get_identity_public_key()
            .await
            .expect("Failed to get identity public key");

        let encrypted_private_key = signer
            .encrypt_private_key_ecies(&private_key, &identity_public_key)
            .expect("Failed to encrypt private key");

        // Generate receiver's key pair
        let receiver_private_key = SecretKey::new(&mut rng);
        let receiver_public_key = receiver_private_key.public_key(&signer.secp);

        // Encrypt for receiver
        let result = signer
            .encrypt_secret_for_receiver(
                &EncryptedSecret::new(encrypted_private_key),
                &receiver_public_key,
            )
            .await
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

    #[async_test_all]
    async fn test_public_key_from_secret() {
        let signer = create_test_signer();
        let secp = Secp256k1::new();
        let mut rng = thread_rng();

        // Test with encrypted private key source
        let private_key = SecretKey::new(&mut rng);
        let expected_public_key = private_key.public_key(&secp);

        let identity_public_key = signer
            .get_identity_public_key()
            .await
            .expect("Failed to get identity public key");

        let encrypted_private_key = signer
            .encrypt_private_key_ecies(&private_key, &identity_public_key)
            .expect("Failed to encrypt private key");

        let encrypted_source = SecretSource::new_encrypted(encrypted_private_key);

        let result_public_key = signer
            .public_key_from_secret(&encrypted_source)
            .await
            .expect("Failed to get public key from encrypted source");

        assert_eq!(expected_public_key, result_public_key);

        // Test with derived private key source
        let node_id = TreeNodeId::from_str("test_node").expect("Failed to create node ID");
        let derived_source = SecretSource::Derived(node_id.clone());

        let result_public_key = signer
            .public_key_from_secret(&derived_source)
            .await
            .expect("Failed to get public key from derived source");

        // Verify it matches what get_public_key_for_node returns
        let expected_public_key = signer
            .get_public_key_for_node(&node_id)
            .await
            .expect("Failed to get public key for node");

        assert_eq!(expected_public_key, result_public_key);
    }

    #[async_test_all]
    async fn test_generate_random_signing_commitment_nonces_round_trip() {
        let signer = create_test_signer();
        let commitments = signer
            .generate_random_signing_commitment()
            .await
            .expect("Failed to generate frost signing commitments");

        let signing_nonces = signer
            .decrypt_nonces_ecies(&commitments.nonces_ciphertext)
            .expect("Failed to decrypt nonces");

        assert_eq!(&commitments.commitments, signing_nonces.commitments());
    }
}
