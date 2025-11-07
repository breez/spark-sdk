use bitcoin::{
    Network, XOnlyPublicKey,
    bip32::{DerivationPath, Xpriv},
    key::Secp256k1,
    secp256k1::SecretKey,
};

pub enum NostrError {
    KeyDerivationError(String),
}

pub struct NostrClient {
    secp: Secp256k1<bitcoin::secp256k1::All>,
    nostr_key: SecretKey,
}

impl NostrClient {
    pub fn new(seed: &[u8], account: u32, network: Network) -> Result<Self, NostrError> {
        let derivation_path: DerivationPath =
            format!("m/44'/1237'/{account}'/0/0").parse().map_err(|e| {
                NostrError::KeyDerivationError(format!("Failed to parse derivation path: {e:?}"))
            })?;
        let master_key = Xpriv::new_master(network, seed).map_err(|e| {
            NostrError::KeyDerivationError(format!("Failed to derive master key: {e:?}"))
        })?;
        let secp = Secp256k1::new();
        let nostr_key = master_key
            .derive_priv(&secp, &derivation_path)
            .map_err(|e| {
                NostrError::KeyDerivationError(format!("Failed to derive nostr child key: {e:?}"))
            })?;

        Ok(NostrClient {
            secp,
            nostr_key: nostr_key.private_key,
        })
    }

    pub fn nostr_pubkey(&self) -> String {
        let (xonly_pubkey, _) = XOnlyPublicKey::from_keypair(&self.nostr_key.keypair(&self.secp));
        xonly_pubkey.to_string()
    }
}
