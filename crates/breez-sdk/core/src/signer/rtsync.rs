use anyhow::anyhow;
use bitcoin::bip32::DerivationPath;
use breez_sdk_common::sync::SyncSigner;
use std::sync::Arc;

use crate::{Network, signer::BreezSigner};

const SIGNING_DERIVATION_PATH: &str = "m/1220588449'/0'/0'/0/0";
const SIGNING_DERIVATION_PATH_TEST: &str = "m/1220588449'/1'/0'/0/0";
const ENCRYPTION_DERIVATION_PATH: &str = "m/1782705014'/0'/0'/0/0";
const ENCRYPTION_DERIVATION_PATH_TEST: &str = "m/1782705014'/1'/0'/0/0";

pub struct RTSyncSigner {
    signer: Arc<dyn BreezSigner>,
    signing_path: DerivationPath,
    encryption_path: DerivationPath,
}

impl RTSyncSigner {
    pub fn new(
        signer: Arc<dyn BreezSigner>,
        network: Network,
    ) -> Result<Self, bitcoin::bip32::Error> {
        let signing_path: DerivationPath = match network {
            Network::Mainnet => SIGNING_DERIVATION_PATH,
            Network::Regtest => SIGNING_DERIVATION_PATH_TEST,
        }
        .parse()?;
        let encryption_path: DerivationPath = match network {
            Network::Mainnet => ENCRYPTION_DERIVATION_PATH,
            Network::Regtest => ENCRYPTION_DERIVATION_PATH_TEST,
        }
        .parse()?;

        Ok(Self {
            signer,
            signing_path,
            encryption_path,
        })
    }
}

#[macros::async_trait]
impl SyncSigner for RTSyncSigner {
    async fn sign_ecdsa_recoverable(&self, data: &[u8]) -> anyhow::Result<Vec<u8>> {
        use bitcoin::hashes::{Hash, sha256};
        use bitcoin::secp256k1::Message;

        // Real-time sync requires double SHA256 hash
        let hash = sha256::Hash::hash(sha256::Hash::hash(data).as_ref());
        let message = Message::from_digest(hash.to_byte_array());
        let sig = self
            .signer
            .sign_ecdsa_recoverable(message, &self.signing_path)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        // Serialize the recoverable signature: recovery_id + 64 bytes
        let (recovery_id, sig_bytes) = sig.serialize_compact();
        let mut complete_signature = vec![31u8.saturating_add(
            u8::try_from(recovery_id.to_i32()).map_err(|e| anyhow!(e.to_string()))?,
        )];
        complete_signature.extend_from_slice(&sig_bytes);
        Ok(complete_signature)
    }

    async fn encrypt_ecies(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        self.signer
            .encrypt_ecies(&msg, &self.encryption_path)
            .await
            .map_err(|e| anyhow!(e.to_string()))
    }

    async fn decrypt_ecies(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        self.signer
            .decrypt_ecies(&msg, &self.encryption_path)
            .await
            .map_err(|e| anyhow!(e.to_string()))
    }
}
