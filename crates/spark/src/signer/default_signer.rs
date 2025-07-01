use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bip32::{ChildNumber, XPrv};
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
use frost_secp256k1_tr::Identifier;
use frost_secp256k1_tr::round1::{SigningCommitments, SigningNonces};
use frost_secp256k1_tr::round2::SignatureShare;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::tree::TreeNodeId;
use crate::{
    Network,
    signer::{Signer, SignerError},
};

use super::VerifiableSecretShare;

const PURPOSE: u32 = 8797555;
fn coin_type(network: Network) -> ChildNumber {
    let coin_type: u32 = match network {
        Network::Mainnet => 0,
        _ => 1,
    };
    ChildNumber::new(coin_type, true)
        .expect(format!("Hardened coin type {} is invalid", coin_type).as_str())
}

fn purpose() -> ChildNumber {
    ChildNumber::new(PURPOSE, true)
        .expect(format!("Hardened purpose {} is invalid", PURPOSE).as_str())
}

fn zero() -> ChildNumber {
    ChildNumber::new(0, true).expect("Hardened zero is invalid")
}

#[derive(Clone)]
pub struct DefaultSigner {
    identity_key: SecretKey,
    master_key: XPrv,
    network: Network,
    nonce_commitments: Arc<Mutex<HashMap<Vec<u8>, SigningNonces>>>, // TODO: Nonce commitments are never cleared, is this okay?
    private_key_map: Arc<Mutex<HashMap<PublicKey, SecretKey>>>,     // TODO: Is this really the way?
    secp: Secp256k1<All>,
}

#[derive(Debug, Error)]
pub enum DefaultSignerError {
    #[error("invalid seed")]
    InvalidSeed,

    #[error("key derivation error: {0}")]
    KeyDerivationError(String),
}

impl From<bip32::Error> for DefaultSignerError {
    fn from(e: bip32::Error) -> Self {
        DefaultSignerError::KeyDerivationError(e.to_string())
    }
}

impl From<secp256k1::Error> for DefaultSignerError {
    fn from(e: secp256k1::Error) -> Self {
        DefaultSignerError::KeyDerivationError(e.to_string())
    }
}

impl DefaultSigner {
    pub fn new(seed: [u8; 32], network: Network) -> Result<Self, DefaultSignerError> {
        let master_key = XPrv::new(seed).map_err(|_| DefaultSignerError::InvalidSeed)?;
        let extended_identity_key = master_key
            .derive_child(purpose())?
            .derive_child(coin_type(network))?
            .derive_child(zero())?;
        let identity_key = SecretKey::from_slice(&extended_identity_key.private_key().to_bytes())?;
        Ok(DefaultSigner {
            identity_key,
            master_key,
            network,
            nonce_commitments: Arc::new(Mutex::new(HashMap::new())),
            private_key_map: Arc::new(Mutex::new(HashMap::new())),
            secp: Secp256k1::new(),
        })
    }
}

impl DefaultSigner {
    fn derive_signing_key(&self, hash: sha256::Hash) -> Result<SecretKey, SignerError> {
        let u32_bytes = hash.as_byte_array()[..4]
            .try_into()
            .map_err(|_| SignerError::InvalidHash)?;
        let index = u32::from_be_bytes(u32_bytes) % 0x80000000;
        let child_number = ChildNumber::new(index, true).map_err(|_| SignerError::InvalidHash)?;
        let child = self.master_key.derive_child(child_number).map_err(|e| {
            SignerError::KeyDerivationError(format!("failed to derive child: {}", e))
        })?;
        SecretKey::from_slice(&child.private_key().to_bytes()).map_err(|e| {
            SignerError::KeyDerivationError(format!("failed to create private key: {}", e))
        })
    }
}

#[async_trait::async_trait]
impl Signer for DefaultSigner {
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
        adaptor_pub_key: Option<PublicKey>,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        todo!()
        // frost_secp256k1_tr::round2::aggregate(signing_package, signature_shares, public_key_package)
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
        let nonce = self
            .nonce_commitments
            .lock()
            .await
            .get(&self_commitment.serialize().map_err(|e| {
                SignerError::SerializationError(format!(
                    "failed to serialize self commitment: {}",
                    e
                ))
            })?)
            .ok_or(SignerError::UnknownNonceCommitment)?;

        let secret_key = self
            .private_key_map
            .lock()
            .await
            .get(public_key)
            .cloned()
            .ok_or(SignerError::UnknownKey)?;
        todo!()
        // let signature_share = frost_secp256k1_tr::round2::sign(
        //     &SigningPackage::new(statechain_commitments, message),
        //     nonce,
        //     &KeyPackage::new(
        //         adaptor_public_key: todo!(),
        //         secret_key: todo!(),
        //         verifying_share: todo!(),
        //         verifying_key: todo!(),
        //         min_signers: todo!(),
        //     ),
        // )?;
        // Ok(signature_share)
    }

    async fn split_secret_with_proofs(
        &self,
        secret: Vec<u8>,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Vec<VerifiableSecretShare>, SignerError> {
        todo!()
    }
}

#[cfg(test)]
mod test {
    use crate::{
        Network,
        signer::default_signer::{coin_type, purpose, zero},
    };

    /// Ensure constants are defined correctly and don't panic.
    #[test]
    fn test_constants() {
        assert_eq!(coin_type(Network::Mainnet).index(), 0);
        assert_eq!(coin_type(Network::Mainnet).is_hardened(), true);
        assert_eq!(coin_type(Network::Testnet).index(), 1);
        assert_eq!(coin_type(Network::Testnet).is_hardened(), true);
        assert_eq!(coin_type(Network::Regtest).index(), 1);
        assert_eq!(coin_type(Network::Regtest).is_hardened(), true);
        assert_eq!(coin_type(Network::Signet).index(), 1);
        assert_eq!(coin_type(Network::Signet).is_hardened(), true);
        assert_eq!(purpose().index(), 8797555u32);
        assert_eq!(purpose().is_hardened(), true);
        assert_eq!(zero().index(), 0u32);
        assert_eq!(zero().is_hardened(), true);
    }
}
