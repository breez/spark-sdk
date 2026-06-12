//! `ExternalBreezSigner` implementation backed by Turnkey.
//!
//! Per Turnkey's design, signing is the norm and encryption is not offered:
//! - derive / ECDSA / Schnorr go to Turnkey (the wallet's enclave keys).
//! - ECIES (multi-device sync + session storage) and HMAC (LNURL-auth path
//!   computation) run locally against a dedicated, non-Spark key exported from
//!   Turnkey (a reserved derivation the Spark signer never uses), via an inner
//!   [`BreezSignerImpl`]. Those uses only need a stable key, so a non-Spark key
//!   keeps every Spark key (the identity key included) in the enclave.

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

use super::accounts::{
    ecdsa_from_rs, ecdsa_recoverable_low_s, schnorr_from_rs, spark_address_format,
};
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
    account: u32,
    encryption: BreezSignerImpl,
}

impl TurnkeyBreezSigner {
    pub(crate) fn new(
        client: Arc<TurnkeyClient>,
        network: Network,
        account: u32,
        encryption: BreezSignerImpl,
    ) -> Self {
        Self {
            client,
            network,
            account,
            encryption,
        }
    }

    /// Roots a caller-supplied path at the wallet identity master
    /// (`m/8797555'/{account}'/0'`). The `BreezSigner` contract applies caller
    /// paths relative to the identity master (as the seed backend does), so the
    /// same path yields the same key on either backend.
    fn identity_rooted_path(&self, caller_path: &str) -> String {
        let base = format!("m/8797555'/{}'/0'", self.account);
        let relative = caller_path.trim_start_matches('m').trim_start_matches('/');
        if relative.is_empty() {
            base
        } else {
            format!("{base}/{relative}")
        }
    }
}

#[macros::async_trait]
impl ExternalBreezSigner for TurnkeyBreezSigner {
    async fn derive_public_key(&self, path: String) -> Result<PublicKeyBytes, SignerError> {
        let hex = self
            .client
            .compressed_pubkey_at(self.identity_rooted_path(&path))
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
            .compressed_pubkey_at(self.identity_rooted_path(&path))
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
            .compressed_pubkey_at(self.identity_rooted_path(&path))
            .await
            .map_err(to_signer_err)?;
        let result = self
            .client
            .sign_raw(sign_with, hex::encode(&message.bytes), HASH_FUNCTION_NO_OP)
            .await
            .map_err(to_signer_err)?;
        // Output layout (per the trait): [31 + recovery_id] || r(32) || s(32).
        // Turnkey reports the recovery id in `v` (hex), possibly Ethereum-style
        // (27/28), normalized here. The signature recovers to the wrong key
        // under a wrong recovery id, so a missing or out-of-range `v` is an
        // error rather than a guessed value.
        let v = u32::from_str_radix(result.v.trim(), 16).map_err(|e| {
            SignerError::Generic(format!("invalid Turnkey recovery id '{}': {e}", result.v))
        })?;
        let normalized = if v >= 27 { v.saturating_sub(27) } else { v };
        let recovery_id = u8::try_from(normalized)
            .ok()
            .filter(|id| *id <= 3)
            .ok_or_else(|| {
                SignerError::Generic(format!("Turnkey recovery id '{}' out of range", result.v))
            })?;
        // Low-s normalize to match the non-recoverable path; flips the recovery
        // id's low bit when `s` is negated so the signature still recovers.
        let (compact, recovery_id) =
            ecdsa_recoverable_low_s(&result.r, &result.s, recovery_id).map_err(to_signer_err)?;
        let mut bytes = Vec::with_capacity(65);
        bytes.push(31u8.saturating_add(recovery_id));
        bytes.extend_from_slice(&compact);
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
            .create_account(
                self.identity_rooted_path(&path),
                spark_address_format(self.network),
            )
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
