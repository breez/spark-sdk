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
        self.signer
            .sign_ecdsa_recoverable(data, &self.signing_path)
            .await
            .map_err(|e| anyhow!(e.to_string()))
    }

    async fn ecies_encrypt(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        self.signer
            .ecies_encrypt(&msg, &self.encryption_path)
            .await
            .map_err(|e| anyhow!(e.to_string()))
    }

    async fn ecies_decrypt(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        self.signer
            .ecies_decrypt(&msg, &self.encryption_path)
            .await
            .map_err(|e| anyhow!(e.to_string()))
    }
}
