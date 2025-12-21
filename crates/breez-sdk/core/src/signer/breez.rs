use crate::{Seed, error::SdkError, models::Config};
use bitcoin::bip32::DerivationPath;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{self, Message, Secp256k1};
use spark_wallet::{DefaultSigner, KeySet, KeySetType, Signer};

use super::BreezSigner;

pub struct BreezSignerImpl {
    config: Config,

    key_set: KeySet,
    account_number: Option<u32>,
    secp: Secp256k1<secp256k1::All>,
    spark_signer: DefaultSigner,
}

impl BreezSignerImpl {
    pub fn new(
        config: Config,
        seed: &Seed,
        key_set_type: KeySetType,
        use_address_index: bool,
        account_number: Option<u32>,
    ) -> Result<Self, SdkError> {
        let seed_bytes = seed.to_bytes()?;
        let key_set = KeySet::new(
            &seed_bytes,
            config.network.into(),
            key_set_type,
            use_address_index,
            account_number,
        )
        .map_err(|e| SdkError::Generic(e.to_string()))?;

        Ok(Self {
            config,
            key_set: key_set.clone(),
            account_number,
            secp: Secp256k1::new(),
            spark_signer: DefaultSigner::from_key_set(key_set),
        })
    }

    pub fn key_set(&self) -> KeySet {
        self.key_set.clone()
    }

    fn get_nostr_keys(&self) -> Result<nostr::Keys, SdkError> {
        let account = self.account_number.unwrap_or(match self.config.network {
            crate::Network::Mainnet => 0,
            crate::Network::Regtest => 1,
        });

        let derivation_path: DerivationPath = format!("m/44'/1237'/{account}'/0/0")
            .parse()
            .map_err(|e| SdkError::Generic(format!("Failed to parse derivation path: {e:?}")))?;

        let nostr_key = self
            .key_set
            .identity_master_key
            .derive_priv(&self.secp, &derivation_path)
            .map_err(|e| SdkError::Generic(format!("Failed to derive nostr child key: {e:?}")))?;

        let nostr_key = nostr::SecretKey::from_slice(&nostr_key.private_key.secret_bytes())
            .map_err(|e| SdkError::Generic(format!("failed to serialize nostr key: {e:?}")))?;

        Ok(nostr::Keys::new(nostr_key))
    }
}

#[async_trait::async_trait]
impl BreezSigner for BreezSignerImpl {
    fn identity_public_key(&self) -> secp256k1::PublicKey {
        self.key_set.identity_key_pair.public_key()
    }

    fn derive_public_key(&self, path: &DerivationPath) -> Result<secp256k1::PublicKey, SdkError> {
        let derived = self
            .key_set
            .identity_master_key
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        Ok(derived.private_key.public_key(&self.secp))
    }

    async fn sign_ecdsa(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<secp256k1::ecdsa::Signature, SdkError> {
        let derived = self
            .key_set
            .identity_master_key
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let digest = bitcoin::hashes::sha256::Hash::hash(message);
        let message = Message::from_digest(digest.to_byte_array());
        Ok(self.secp.sign_ecdsa(&message, &derived.private_key))
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let derived = self
            .key_set
            .identity_master_key
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let digest = bitcoin::hashes::sha256::Hash::hash(
            bitcoin::hashes::sha256::Hash::hash(message).as_ref(),
        );
        let (recovery_id, sig) = self
            .secp
            .sign_ecdsa_recoverable(
                &Message::from_digest(digest.to_byte_array()),
                &derived.private_key,
            )
            .serialize_compact();

        let mut complete_signature = vec![31u8.saturating_add(
            u8::try_from(recovery_id.to_i32()).map_err(|e| SdkError::Generic(e.to_string()))?,
        )];
        complete_signature.extend_from_slice(&sig);
        Ok(complete_signature)
    }

    async fn ecies_encrypt(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let derived = self
            .key_set
            .identity_master_key
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let rc_pub = derived.private_key.public_key(&self.secp).serialize();
        ecies::encrypt(&rc_pub, message)
            .map_err(|err| SdkError::Generic(format!("Could not encrypt data: {err}")))
    }

    async fn ecies_decrypt(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let derived = self
            .key_set
            .identity_master_key
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let rc_prv = derived.private_key.secret_bytes();
        ecies::decrypt(&rc_prv, message)
            .map_err(|err| SdkError::Generic(format!("Could not decrypt data: {err}")))
    }

    async fn sign_hash_schnorr(
        &self,
        hash: &[u8],
        path: &DerivationPath,
    ) -> Result<secp256k1::schnorr::Signature, SdkError> {
        let derived = self
            .key_set
            .identity_master_key
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let message =
            Message::from_digest_slice(hash).map_err(|e| SdkError::Generic(e.to_string()))?;
        let keypair = derived.private_key.keypair(&self.secp);
        Ok(self.secp.sign_schnorr(&message, &keypair))
    }

    async fn nostr_pubkey(&self) -> Result<String, SdkError> {
        let keys = self.get_nostr_keys()?;
        Ok(keys.public_key().to_string())
    }

    async fn sign_nostr_event(
        &self,
        builder: ::nostr::event::EventBuilder,
    ) -> Result<::nostr::event::Event, SdkError> {
        let keys = self.get_nostr_keys()?;
        builder
            .sign_with_keys(&keys)
            .map_err(|e| SdkError::Generic(format!("Failed to sign nostr event: {e:?}")))
    }

    async fn generate_frost_signing_commitments(
        &self,
    ) -> Result<spark_wallet::FrostSigningCommitmentsWithNonces, SdkError> {
        self.spark_signer
            .generate_frost_signing_commitments()
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn get_public_key_for_node(
        &self,
        id: &spark_wallet::TreeNodeId,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        self.spark_signer
            .get_public_key_for_node(id)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn generate_random_key(&self) -> Result<spark_wallet::PrivateKeySource, SdkError> {
        self.spark_signer
            .generate_random_key()
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<spark_wallet::PrivateKeySource, SdkError> {
        self.spark_signer
            .get_static_deposit_private_key_source(index)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn get_static_deposit_private_key(
        &self,
        index: u32,
    ) -> Result<secp256k1::SecretKey, SdkError> {
        self.spark_signer
            .get_static_deposit_private_key(index)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        self.spark_signer
            .get_static_deposit_public_key(index)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn subtract_private_keys(
        &self,
        signing_key: &spark_wallet::PrivateKeySource,
        new_signing_key: &spark_wallet::PrivateKeySource,
    ) -> Result<spark_wallet::PrivateKeySource, SdkError> {
        self.spark_signer
            .subtract_private_keys(signing_key, new_signing_key)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn split_secret_with_proofs(
        &self,
        secret: &spark_wallet::SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<spark_wallet::VerifiableSecretShare>, SdkError> {
        self.spark_signer
            .split_secret_with_proofs(secret, threshold, num_shares)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn encrypt_private_key_for_receiver(
        &self,
        private_key: &spark_wallet::EncryptedPrivateKey,
        receiver_public_key: &secp256k1::PublicKey,
    ) -> Result<Vec<u8>, SdkError> {
        self.spark_signer
            .encrypt_private_key_for_receiver(private_key, receiver_public_key)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn get_public_key_from_private_key_source(
        &self,
        private_key: &spark_wallet::PrivateKeySource,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        self.spark_signer
            .get_public_key_from_private_key_source(private_key)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn sign_frost<'a>(
        &self,
        request: spark_wallet::SignFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::round2::SignatureShare, SdkError> {
        self.spark_signer
            .sign_frost(request)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn aggregate_frost<'a>(
        &self,
        request: spark_wallet::AggregateFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::Signature, SdkError> {
        self.spark_signer
            .aggregate_frost(request)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }
}
