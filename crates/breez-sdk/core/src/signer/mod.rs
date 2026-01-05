use crate::SdkError;
use bitcoin::bip32::DerivationPath;
use bitcoin::secp256k1;

#[async_trait::async_trait]
pub trait BreezSigner: Send + Sync {
    /// Returns the identity public key.
    fn identity_public_key(&self) -> secp256k1::PublicKey;

    async fn sign_ecdsa(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<secp256k1::ecdsa::Signature, SdkError>;

    async fn sign_ecdsa_recoverable(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError>;

    async fn ecies_encrypt(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError>;

    async fn ecies_decrypt(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError>;

    async fn sign_hash_schnorr(
        &self,
        hash: &[u8],
        path: &DerivationPath,
    ) -> Result<secp256k1::schnorr::Signature, SdkError>;

    async fn derive_public_key(
        &self,
        path: &DerivationPath,
    ) -> Result<secp256k1::PublicKey, SdkError>;

    async fn generate_frost_signing_commitments(
        &self,
    ) -> Result<spark_wallet::FrostSigningCommitmentsWithNonces, SdkError>;

    async fn get_public_key_for_node(
        &self,
        id: &spark_wallet::TreeNodeId,
    ) -> Result<secp256k1::PublicKey, SdkError>;

    async fn generate_random_key(&self) -> Result<spark_wallet::PrivateKeySource, SdkError>;

    async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<spark_wallet::PrivateKeySource, SdkError>;

    async fn get_static_deposit_private_key(
        &self,
        index: u32,
    ) -> Result<secp256k1::SecretKey, SdkError>;

    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<secp256k1::PublicKey, SdkError>;

    async fn subtract_private_keys(
        &self,
        signing_key: &spark_wallet::PrivateKeySource,
        new_signing_key: &spark_wallet::PrivateKeySource,
    ) -> Result<spark_wallet::PrivateKeySource, SdkError>;

    async fn split_secret_with_proofs(
        &self,
        secret: &spark_wallet::SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<spark_wallet::VerifiableSecretShare>, SdkError>;

    async fn encrypt_private_key_for_receiver(
        &self,
        private_key: &spark_wallet::EncryptedPrivateKey,
        receiver_public_key: &secp256k1::PublicKey,
    ) -> Result<Vec<u8>, SdkError>;

    async fn get_public_key_from_private_key_source(
        &self,
        private_key: &spark_wallet::PrivateKeySource,
    ) -> Result<secp256k1::PublicKey, SdkError>;

    async fn sign_frost<'a>(
        &self,
        request: spark_wallet::SignFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::round2::SignatureShare, SdkError>;

    async fn aggregate_frost<'a>(
        &self,
        request: spark_wallet::AggregateFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::Signature, SdkError>;

    /// Derives an extended public key (xpub) at the given derivation path.
    /// Returns the xpub encoded bytes including chain code.
    async fn derive_xpub(&self, path: &DerivationPath) -> Result<bitcoin::bip32::Xpub, SdkError>;

    /// Computes HMAC-SHA256 using a key derived at the given path.
    async fn hmac_sha256(
        &self,
        key_path: &DerivationPath,
        input: &[u8],
    ) -> Result<Vec<u8>, SdkError>;
}

pub mod breez;
pub mod lnurl_auth;
pub mod nostr;
pub mod rtsync;
pub mod spark;
