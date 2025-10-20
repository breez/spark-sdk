use std::sync::Arc;

use bitcoin::bip32::DerivationPath;
use breez_sdk_common::sync::signer::SyncSigner;
use spark_wallet::Signer;

pub struct DefaultSyncSigner {
    derivation_path: DerivationPath,
    inner: Arc<dyn Signer>,
}

impl DefaultSyncSigner {
    pub fn new(inner: Arc<dyn Signer>, derivation_path: DerivationPath) -> Self {
        DefaultSyncSigner {
            derivation_path,
            inner,
        }
    }
}
#[macros::async_trait]
impl SyncSigner for DefaultSyncSigner {
    async fn sign_ecdsa_recoverable(&self, data: &[u8]) -> anyhow::Result<Vec<u8>> {
        let (recovery_id, sig) = self
            .inner
            .sign_message_ecdsa_recoverable_from_path(data, &self.derivation_path)?
            .serialize_compact();
        let mut complete_signature = vec![31u8.saturating_add(u8::try_from(recovery_id.to_i32())?)];
        complete_signature.extend_from_slice(&sig);
        Ok(complete_signature)
    }

    async fn ecies_encrypt(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        Ok(self
            .inner
            .ecies_encrypt(msg, self.derivation_path.clone())
            .await?)
    }

    async fn ecies_decrypt(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        Ok(self
            .inner
            .ecies_decrypt(msg, self.derivation_path.clone())
            .await?)
    }
}
