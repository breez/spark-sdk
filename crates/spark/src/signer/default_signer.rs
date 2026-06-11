use bitcoin::bip32::{ChildNumber, DerivationPath, Xpriv};
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::secp256k1::rand::thread_rng;
use bitcoin::secp256k1::{self, All, Message, PublicKey, SecretKey, schnorr};
use bitcoin::{
    hashes::{Hash, sha256},
    key::Secp256k1,
};
use frost_core::round1::Nonce;
use frost_secp256k1_tr::keys::{EvenY, KeyPackage, SigningShare, Tweak, VerifyingShare};
use frost_secp256k1_tr::round1::SigningNonces;
use frost_secp256k1_tr::round2::SignatureShare;
use frost_secp256k1_tr::{Identifier, VerifyingKey};
use thiserror::Error;

use crate::signer::{
    EncryptedSecret, FrostSigningCommitmentsWithNonces, SignFrostRequest, secret_sharing,
};
use crate::signer::{SecretSource, SecretToSplit};
use crate::{
    Network,
    signer::{Signer, SignerError},
};

use super::VerifiableSecretShare;

/// The default Spark account number (`m/8797555'/{account}'`) for `network`,
/// used when the caller does not pin one. Every signer backend should apply the
/// same default so a wallet seed derives the same keys regardless of backend.
pub fn default_account_number(network: Network) -> u32 {
    match network {
        Network::Regtest => 0,
        _ => 1,
    }
}

/// Path of the identity / ECIES key under the account master: the `0'` child.
/// The SDK-layer `BreezSigner` roots here and the Spark identity lives here too.
fn identity_path() -> DerivationPath {
    DerivationPath::from(vec![
        ChildNumber::from_hardened_idx(0).expect("0 is a valid hardened index"),
    ])
}

/// The Spark account master (`base`): `m/8797555'/{account}'`. Every wallet key
/// derives from it (identity at `0'`, leaf signing under `1'`, static deposit
/// under `3'`). `account_no` falls back to a per-network default when unset.
pub fn account_master_key(
    seed: &[u8],
    network: Network,
    account_no: Option<u32>,
) -> Result<Xpriv, DefaultSignerError> {
    let account_number = account_no.unwrap_or_else(|| default_account_number(network));
    let path: DerivationPath = format!("m/8797555'/{account_number}'").parse()?;
    let master = Xpriv::new_master(network, seed)?;
    Ok(master.derive_priv(&Secp256k1::new(), &path)?)
}

/// The wallet identity master (`base/0'`): the `BreezSigner` derivation root and
/// the Spark identity / ECIES key.
pub fn identity_master_key(
    seed: &[u8],
    network: Network,
    account_no: Option<u32>,
) -> Result<Xpriv, DefaultSignerError> {
    let base = account_master_key(seed, network, account_no)?;
    Ok(base.derive_priv(&Secp256k1::new(), &identity_path())?)
}

/// The wallet identity public key (`base/0'`). Lets storage be scoped per wallet
/// before any signer is built.
pub fn identity_public_key(
    seed: &[u8],
    network: Network,
    account_no: Option<u32>,
) -> Result<PublicKey, DefaultSignerError> {
    let master = identity_master_key(seed, network, account_no)?;
    Ok(master.private_key.public_key(&Secp256k1::new()))
}

#[derive(Clone)]
pub struct DefaultSigner {
    /// The master node every key is derived from.
    master: Xpriv,
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
        Ok(Self::from_master(account_master_key(seed, network, None)?))
    }

    /// Builds a signer rooted at `master`. Every key derives from it by BIP32
    /// path; the identity / ECIES key is its `0'` child.
    pub fn from_master(master: Xpriv) -> Self {
        DefaultSigner {
            master,
            secp: Secp256k1::new(),
        }
    }
}

impl DefaultSigner {
    /// Derives the raw secret key at `path` under the master.
    fn derive_at(&self, path: &DerivationPath) -> Result<SecretKey, SignerError> {
        Ok(self
            .master
            .derive_priv(&self.secp, path)
            .map_err(|e| SignerError::KeyDerivationError(format!("failed to derive child: {e}")))?
            .private_key)
    }

    /// Public key used for ECIES (the key counterparties encrypt to): the
    /// identity key, where the Spark layer derives the wallet identity.
    fn encryption_public_key(&self) -> Result<PublicKey, SignerError> {
        Ok(self.derive_at(&identity_path())?.public_key(&self.secp))
    }

    fn encrypt_message_ecies(
        &self,
        message: &[u8],
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        utils::ecies::encrypt(&receiver_public_key.serialize(), message)
            .map_err(|e| SignerError::Generic(format!("failed to encrypt: {e}")))
    }

    fn decrypt_message_ecies(&self, ciphertext: &[u8]) -> Result<Vec<u8>, SignerError> {
        let secret = self.derive_at(&identity_path())?;
        utils::ecies::decrypt(&secret.secret_bytes(), ciphertext)
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
    async fn sign_message_ecdsa(
        &self,
        path: &DerivationPath,
        message: &[u8],
    ) -> Result<Signature, SignerError> {
        let digest = sha256::Hash::hash(message);
        let sig = self.secp.sign_ecdsa(
            &Message::from_digest(digest.to_byte_array()),
            &self.derive_at(path)?,
        );
        Ok(sig)
    }

    async fn sign_hash_schnorr(
        &self,
        path: &DerivationPath,
        hash: &[u8],
    ) -> Result<schnorr::Signature, SignerError> {
        if hash.len() != 32 {
            return Err(SignerError::Generic(
                "Hash must be exactly 32 bytes".to_string(),
            ));
        }
        let mut hash_array = [0u8; 32];
        hash_array.copy_from_slice(hash);
        let keypair = self.derive_at(path)?.keypair(&self.secp);
        // Always use auxiliary randomness for enhanced security
        let mut rng = thread_rng();
        let sig =
            self.secp
                .sign_schnorr_with_rng(&Message::from_digest(hash_array), &keypair, &mut rng);
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
            self.encrypt_nonces_ecies(&nonces, &self.encryption_public_key()?)?;
        let commitments = *nonces.commitments();

        Ok(FrostSigningCommitmentsWithNonces {
            commitments,
            nonces_ciphertext,
        })
    }

    async fn derive_public_key(&self, path: &DerivationPath) -> Result<PublicKey, SignerError> {
        Ok(self.derive_at(path)?.public_key(&self.secp))
    }

    async fn secret_key(&self, path: &DerivationPath) -> Result<SecretKey, SignerError> {
        self.derive_at(path)
    }

    async fn generate_random_secret(&self) -> Result<EncryptedSecret, SignerError> {
        let (secret_key, _) = self.secp.generate_keypair(&mut thread_rng());
        Ok(EncryptedSecret::new(self.encrypt_private_key_ecies(
            &secret_key,
            &self.encryption_public_key()?,
        )?))
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

        let ciphertext = self.encrypt_private_key_ecies(&res, &self.encryption_public_key()?)?;

        Ok(SecretSource::new_encrypted(ciphertext))
    }

    async fn encrypt_secret_for_receiver(
        &self,
        secret: &SecretSource,
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        let private_key = secret.to_secret_key(self)?;
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
        let signing_package = crate::utils::frost::frost_signing_package(
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
}

impl SecretSource {
    fn to_secret_key(&self, signer: &DefaultSigner) -> Result<SecretKey, SignerError> {
        match self {
            SecretSource::Derived(path) => signer.derive_at(path),
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
    use crate::utils::verify_signature::verify_signature_ecdsa;
    use crate::{Network, signer::default_signer::DefaultSigner};
    use bitcoin::bip32::DerivationPath;

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
            .sign_message_ecdsa(
                &"m/0'".parse::<DerivationPath>().unwrap(),
                message.as_bytes(),
            )
            .await
            .expect("Failed to sign message");

        verify_signature_ecdsa(
            &signer.secp,
            message,
            &signature,
            &signer
                .derive_public_key(&"m/0'".parse::<DerivationPath>().unwrap())
                .await
                .unwrap(),
        )
        .expect("Failed to verify signature");
    }

    #[async_test_all]
    async fn test_verify_signature_ecdsa_invalid_signature() {
        let signer = create_test_signer();
        let signature = signer
            .sign_message_ecdsa(
                &"m/0'".parse::<DerivationPath>().unwrap(),
                "signed message".as_bytes(),
            )
            .await
            .expect("Failed to sign message");

        // Wrong message
        let result = verify_signature_ecdsa(
            &signer.secp,
            "another message",
            &signature,
            &signer
                .derive_public_key(&"m/0'".parse::<DerivationPath>().unwrap())
                .await
                .unwrap(),
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
            .derive_public_key(&"m/0'".parse::<DerivationPath>().unwrap())
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
            .derive_public_key(&"m/0'".parse::<DerivationPath>().unwrap())
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
            .derive_public_key(&"m/0'".parse::<DerivationPath>().unwrap())
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
            .derive_public_key(&"m/0'".parse::<DerivationPath>().unwrap())
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
                &SecretSource::Encrypted(EncryptedSecret::new(encrypted_private_key)),
                &receiver_public_key,
            )
            .await
            .expect("Failed to encrypt for receiver");

        // Verify result is not empty
        assert!(!result.is_empty());

        // Verify receiver can decrypt it
        let decrypted = utils::ecies::decrypt(&receiver_private_key.secret_bytes(), &result)
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
            .derive_public_key(&"m/0'".parse::<DerivationPath>().unwrap())
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
        let path = DerivationPath::from_str("m/1'/0'").expect("Failed to parse path");
        let derived_source = SecretSource::Derived(path.clone());

        let result_public_key = signer
            .public_key_from_secret(&derived_source)
            .await
            .expect("Failed to get public key from derived source");

        // Verify it matches what derive_public_key returns for the same path
        let expected_public_key = signer
            .derive_public_key(&path)
            .await
            .expect("Failed to derive public key");

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
