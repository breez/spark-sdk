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
    async fn derive_bip32_pub_key(&self, derivation_path: &[ChildNumber]) -> LnurlResult<Vec<u8>> {
        // Convert ChildNumber slice to DerivationPath
        let path = DerivationPath::from(derivation_path.to_vec());

        // Delegate to BreezSigner to get xpub
        let xpub = self
            .signer
            .derive_xpub(&path)
            .await
            .map_err(|e| LnurlError::General(e.to_string()))?;

        // Return the encoded xpub bytes
        Ok(xpub.encode().to_vec())
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
        let path = DerivationPath::from(key_derivation_path.to_vec());

        // Delegate to BreezSigner for HMAC-SHA256
        self.signer
            .hmac_sha256(&path, input)
            .await
            .map_err(|e| LnurlError::General(e.to_string()))
    }
}
