use std::sync::Arc;

use bitcoin::bip32::{ChildNumber, DerivationPath};
use breez_sdk_common::lnurl::auth::LnurlAuthSigner;
use breez_sdk_common::lnurl::error::{LnurlError, LnurlResult};

use crate::signer::BreezSigner;

/// Adapter that implements `LnurlAuthSigner` by delegating to `BreezSigner`
pub struct LnurlAuthSignerAdapter {
    signer: Arc<dyn BreezSigner>,
}

impl LnurlAuthSignerAdapter {
    pub fn new(signer: Arc<dyn BreezSigner>) -> Self {
        Self { signer }
    }
}

#[macros::async_trait]
impl LnurlAuthSigner for LnurlAuthSignerAdapter {
    async fn derive_public_key(
        &self,
        derivation_path: &[ChildNumber],
    ) -> LnurlResult<bitcoin::secp256k1::PublicKey> {
        // Convert ChildNumber slice to DerivationPath
        let path = DerivationPath::from(derivation_path.to_vec());

        // Delegate to BreezSigner to get public key directly
        self.signer
            .derive_public_key(&path)
            .await
            .map_err(|e| LnurlError::General(e.to_string()))
    }

    async fn sign_ecdsa(
        &self,
        msg: &[u8],
        derivation_path: &[ChildNumber],
    ) -> LnurlResult<Vec<u8>> {
        let path = DerivationPath::from(derivation_path.to_vec());

        // Delegate to BreezSigner for ECDSA signing
        let sig = self
            .signer
            .sign_ecdsa(msg, &path)
            .await
            .map_err(|e| LnurlError::General(e.to_string()))?;

        // Return DER-encoded signature
        Ok(sig.serialize_der().to_vec())
    }

    async fn hmac_sha256(
        &self,
        key_derivation_path: &[ChildNumber],
        input: &[u8],
    ) -> LnurlResult<Vec<u8>> {
        use bitcoin::hashes::Hash;

        let path = DerivationPath::from(key_derivation_path.to_vec());

        // Delegate to BreezSigner for HMAC-SHA256
        let hmac = self
            .signer
            .hmac_sha256(&path, input)
            .await
            .map_err(|e| LnurlError::General(e.to_string()))?;

        // Convert Hmac<sha256::Hash> to Vec<u8>
        Ok(hmac.as_byte_array().to_vec())
    }
}
