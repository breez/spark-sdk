use anyhow::anyhow;
use bitcoin::bip32::DerivationPath;
use breez_sdk_common::sync::SyncSigner;
use std::sync::Arc;

use crate::{
    Network,
    signer::{BreezSigner, EciesSigner},
};

const SIGNING_DERIVATION_PATH: &str = "m/1220588449'/0'/0'/0/0";
const SIGNING_DERIVATION_PATH_TEST: &str = "m/1220588449'/1'/0'/0/0";
const SIGNING_DERIVATION_PATH_SIGNET: &str = "m/1220588449'/1'/1'/0/0";
const ENCRYPTION_DERIVATION_PATH: &str = "m/1782705014'/0'/0'/0/0";
const ENCRYPTION_DERIVATION_PATH_TEST: &str = "m/1782705014'/1'/0'/0/0";
const ENCRYPTION_DERIVATION_PATH_SIGNET: &str = "m/1782705014'/1'/1'/0/0";

pub struct RTSyncSigner {
    signer: Arc<dyn BreezSigner>,
    ecies: Arc<dyn EciesSigner>,
    signing_path: DerivationPath,
    encryption_path: DerivationPath,
}

impl RTSyncSigner {
    pub fn new(
        signer: Arc<dyn BreezSigner>,
        ecies: Arc<dyn EciesSigner>,
        network: Network,
    ) -> Result<Self, bitcoin::bip32::Error> {
        let signing_path: DerivationPath = match network {
            Network::Mainnet => SIGNING_DERIVATION_PATH,
            Network::Regtest => SIGNING_DERIVATION_PATH_TEST,
            Network::Signet => SIGNING_DERIVATION_PATH_SIGNET,
        }
        .parse()?;
        let encryption_path: DerivationPath = match network {
            Network::Mainnet => ENCRYPTION_DERIVATION_PATH,
            Network::Regtest => ENCRYPTION_DERIVATION_PATH_TEST,
            Network::Signet => ENCRYPTION_DERIVATION_PATH_SIGNET,
        }
        .parse()?;

        Ok(Self {
            signer,
            ecies,
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
        self.ecies
            .encrypt_ecies(&msg, &self.encryption_path)
            .await
            .map_err(|e| anyhow!(e.to_string()))
    }

    async fn decrypt_ecies(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        self.ecies
            .decrypt_ecies(&msg, &self.encryption_path)
            .await
            .map_err(|e| anyhow!(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::breez::BreezSignerImpl;
    use bitcoin::bip32::Xpriv;

    #[test]
    fn sync_derivation_paths() {
        let master = Xpriv::new_master(bitcoin::Network::Signet, &[7; 32]).unwrap();
        let signer = Arc::new(BreezSignerImpl::new(master));
        for (network, signing, encryption) in [
            (
                Network::Mainnet,
                "m/1220588449'/0'/0'/0/0",
                "m/1782705014'/0'/0'/0/0",
            ),
            (
                Network::Regtest,
                "m/1220588449'/1'/0'/0/0",
                "m/1782705014'/1'/0'/0/0",
            ),
            (
                Network::Signet,
                "m/1220588449'/1'/1'/0/0",
                "m/1782705014'/1'/1'/0/0",
            ),
        ] {
            let sync = RTSyncSigner::new(signer.clone(), signer.clone(), network).unwrap();
            assert_eq!(sync.signing_path, signing.parse().unwrap());
            assert_eq!(sync.encryption_path, encryption.parse().unwrap());
        }
    }

    #[macros::async_test_all]
    async fn signet_sync_is_isolated_from_regtest_with_the_same_signer() {
        let master = Xpriv::new_master(bitcoin::Network::Signet, &[7; 32]).unwrap();
        let shared_signer = Arc::new(BreezSignerImpl::new(master));
        let signet = RTSyncSigner::new(
            shared_signer.clone(),
            shared_signer.clone(),
            Network::Signet,
        )
        .unwrap();
        let regtest =
            RTSyncSigner::new(shared_signer.clone(), shared_signer, Network::Regtest).unwrap();
        let message = b"sync data".to_vec();
        assert_ne!(
            signet.sign_ecdsa_recoverable(&message).await.unwrap(),
            regtest.sign_ecdsa_recoverable(&message).await.unwrap()
        );
        for (source, other) in [(&signet, &regtest), (&regtest, &signet)] {
            let encrypted = source.encrypt_ecies(message.clone()).await.unwrap();
            assert_eq!(
                source.decrypt_ecies(encrypted.clone()).await.unwrap(),
                message
            );
            assert!(other.decrypt_ecies(encrypted).await.is_err());
        }
    }
}
