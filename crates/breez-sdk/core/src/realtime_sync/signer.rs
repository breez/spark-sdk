use anyhow::anyhow;
use bitcoin::{
    bip32::{DerivationPath, Xpriv},
    hashes::{Hash, sha256},
    key::Secp256k1,
    secp256k1::{All, Message, SecretKey},
};
use breez_sdk_common::sync::SyncSigner;

use crate::Network;

const SIGNING_DERIVATION_PATH: &str = "m/1220588449'/0'/0'/0/0";
const SIGNING_DERIVATION_PATH_TEST: &str = "m/1220588449'/1'/0'/0/0";
const ENCRYPTION_DERIVATION_PATH: &str = "m/1782705014'/0'/0'/0/0";
const ENCRYPTION_DERIVATION_PATH_TEST: &str = "m/1782705014'/1'/0'/0/0";

pub struct DefaultSyncSigner {
    signing_key: SecretKey,
    encryption_key: SecretKey,
    secp: Secp256k1<All>,
}

impl DefaultSyncSigner {
    pub fn new(seed: &[u8], network: Network) -> Result<Self, bitcoin::bip32::Error> {
        let bitcoin_network: bitcoin::Network = network.into();
        let xpriv = Xpriv::new_master(bitcoin_network, seed)?;
        let secp = Secp256k1::new();
        let signing_derivation_path: DerivationPath = match network {
            Network::Mainnet => SIGNING_DERIVATION_PATH,
            Network::Regtest => SIGNING_DERIVATION_PATH_TEST,
        }
        .parse()?;
        let encryption_derivation_path: DerivationPath = match network {
            Network::Mainnet => ENCRYPTION_DERIVATION_PATH,
            Network::Regtest => ENCRYPTION_DERIVATION_PATH_TEST,
        }
        .parse()?;
        let signing_key = xpriv
            .derive_priv(&secp, &signing_derivation_path)?
            .private_key;
        let encryption_key = xpriv
            .derive_priv(&secp, &encryption_derivation_path)?
            .private_key;
        Ok(Self {
            signing_key,
            encryption_key,
            secp,
        })
    }
}

#[macros::async_trait]
impl SyncSigner for DefaultSyncSigner {
    async fn sign_ecdsa_recoverable(&self, data: &[u8]) -> anyhow::Result<Vec<u8>> {
        let digest = sha256::Hash::hash(sha256::Hash::hash(data).as_ref());
        let (recovery_id, sig) = self
            .secp
            .sign_ecdsa_recoverable(
                &Message::from_digest(digest.to_byte_array()),
                &self.signing_key,
            )
            .serialize_compact();

        let mut complete_signature = vec![31u8.saturating_add(u8::try_from(recovery_id.to_i32())?)];
        complete_signature.extend_from_slice(&sig);
        Ok(complete_signature)
    }

    async fn ecies_encrypt(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        let rc_pub = self.encryption_key.public_key(&self.secp).serialize();
        ecies::encrypt(&rc_pub, &msg).map_err(|err| anyhow!("Could not encrypt data: {err}"))
    }

    async fn ecies_decrypt(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        let rc_prv = self.encryption_key.secret_bytes();
        ecies::decrypt(&rc_prv, &msg).map_err(|err| anyhow!("Could not decrypt data: {err}"))
    }
}
