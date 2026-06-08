//! `ExternalBreezSigner` implementation backed by Turnkey.
//!
//! Per Turnkey's design, signing is the norm and encryption is not offered:
//! - derive / ECDSA / Schnorr go to Turnkey (the wallet's enclave keys).
//! - ECIES (multi-device sync + session storage) and HMAC (LNURL-auth path
//!   computation) run locally against a single key exported from Turnkey, via an
//!   inner [`BreezSignerImpl`]. Those uses only need to be consistent, not match
//!   the enclave keys, so a stable exported key suffices.

use std::str::FromStr;
use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;

use crate::Network;
use crate::error::SignerError;
use crate::signer::breez::BreezSignerImpl;
use crate::signer::external_types::{
    EcdsaSignatureBytes, HashedMessageBytes, MessageBytes, PublicKeyBytes,
    RecoverableEcdsaSignatureBytes, SchnorrSignatureBytes, string_to_derivation_path,
};
use crate::signer::{BreezSigner, ExternalBreezSigner};

use super::accounts::{decode_scalar_32, ecdsa_from_rs, schnorr_from_rs, spark_address_format};
use super::transport::TurnkeyClient;
use super::types::HASH_FUNCTION_NO_OP;

fn to_signer_err<E: std::fmt::Display>(e: E) -> SignerError {
    SignerError::Generic(e.to_string())
}

/// SDK-layer signer backed by Turnkey. Sign/derive go to Turnkey; ECIES/HMAC
/// delegate to `encryption`, an inner signer rooted at a single exported key.
pub(crate) struct TurnkeyBreezSigner {
    client: Arc<TurnkeyClient>,
    network: Network,
    encryption: BreezSignerImpl,
}

impl TurnkeyBreezSigner {
    pub(crate) fn new(
        client: Arc<TurnkeyClient>,
        network: Network,
        encryption: BreezSignerImpl,
    ) -> Self {
        Self {
            client,
            network,
            encryption,
        }
    }
}

#[macros::async_trait]
impl ExternalBreezSigner for TurnkeyBreezSigner {
    async fn derive_public_key(&self, path: String) -> Result<PublicKeyBytes, SignerError> {
        let hex = self
            .client
            .compressed_pubkey_at(path)
            .await
            .map_err(to_signer_err)?;
        let pk = PublicKey::from_str(&hex).map_err(to_signer_err)?;
        Ok(PublicKeyBytes::from_public_key(&pk))
    }

    async fn sign_ecdsa(
        &self,
        message: MessageBytes,
        path: String,
    ) -> Result<EcdsaSignatureBytes, SignerError> {
        // The compressed account address selects secp256k1 ECDSA; `message` is a
        // 32-byte digest, so Turnkey signs it as-is (NO_OP).
        let sign_with = self
            .client
            .compressed_pubkey_at(path)
            .await
            .map_err(to_signer_err)?;
        let result = self
            .client
            .sign_raw(sign_with, hex::encode(&message.bytes), HASH_FUNCTION_NO_OP)
            .await
            .map_err(to_signer_err)?;
        let sig = ecdsa_from_rs(&result.r, &result.s).map_err(to_signer_err)?;
        Ok(EcdsaSignatureBytes::from_signature(&sig))
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: MessageBytes,
        path: String,
    ) -> Result<RecoverableEcdsaSignatureBytes, SignerError> {
        let sign_with = self
            .client
            .compressed_pubkey_at(path)
            .await
            .map_err(to_signer_err)?;
        let result = self
            .client
            .sign_raw(sign_with, hex::encode(&message.bytes), HASH_FUNCTION_NO_OP)
            .await
            .map_err(to_signer_err)?;
        // Output layout (per the trait): [31 + recovery_id] || r(32) || s(32).
        // Turnkey reports the recovery id in `v`; the exact encoding is verified
        // at integration time (Ethereum-style 27/28 is normalized here).
        let recovery_id = hex::decode(&result.v)
            .ok()
            .and_then(|b| b.last().copied())
            .map_or(0, |b| if b >= 27 { b.saturating_sub(27) } else { b });
        let mut bytes = Vec::with_capacity(65);
        bytes.push(31u8.saturating_add(recovery_id));
        bytes.extend_from_slice(&decode_scalar_32(&result.r).map_err(to_signer_err)?);
        bytes.extend_from_slice(&decode_scalar_32(&result.s).map_err(to_signer_err)?);
        Ok(RecoverableEcdsaSignatureBytes::new(bytes))
    }

    async fn encrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        let path = string_to_derivation_path(&path).map_err(to_signer_err)?;
        self.encryption
            .encrypt_ecies(&message, &path)
            .await
            .map_err(to_signer_err)
    }

    async fn decrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        let path = string_to_derivation_path(&path).map_err(to_signer_err)?;
        self.encryption
            .decrypt_ecies(&message, &path)
            .await
            .map_err(to_signer_err)
    }

    async fn sign_hash_schnorr(
        &self,
        hash: Vec<u8>,
        path: String,
    ) -> Result<SchnorrSignatureBytes, SignerError> {
        // A Spark-format account at the path selects BIP-340 Schnorr; the 32-byte
        // hash is signed as-is (NO_OP).
        let sign_with = self
            .client
            .create_account(path, spark_address_format(self.network))
            .await
            .map_err(to_signer_err)?;
        let result = self
            .client
            .sign_raw(sign_with, hex::encode(&hash), HASH_FUNCTION_NO_OP)
            .await
            .map_err(to_signer_err)?;
        let sig = schnorr_from_rs(&result.r, &result.s).map_err(to_signer_err)?;
        Ok(SchnorrSignatureBytes::from_signature(&sig))
    }

    async fn hmac_sha256(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<HashedMessageBytes, SignerError> {
        let path = string_to_derivation_path(&path).map_err(to_signer_err)?;
        let hmac = self
            .encryption
            .hmac_sha256(&path, &message)
            .await
            .map_err(to_signer_err)?;
        Ok(HashedMessageBytes::from_hmac(&hmac))
    }
}
