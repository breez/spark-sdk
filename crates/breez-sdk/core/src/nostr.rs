use bitcoin::bip32::{DerivationPath, Xpriv};
use nostr::{
    Keys, SecretKey,
    secp256k1::{All, Secp256k1, XOnlyPublicKey},
    util::JsonUtil,
};

use crate::{Payment, PaymentDetails};

#[derive(Debug)]
pub enum NostrError {
    KeyDerivationError(String),
    ZapReceiptCreationError(String),
}

pub struct NostrClient {
    secp: Secp256k1<All>,
    nostr_key: SecretKey,
}

impl NostrClient {
    pub fn new(master_key: &Xpriv, account: u32) -> Result<Self, NostrError> {
        // This derivation is kind-of NIP-06, but not really, since we derive from the identity key, not the seed.
        let derivation_path: DerivationPath =
            format!("m/44'/1237'/{account}'/0/0").parse().map_err(|e| {
                NostrError::KeyDerivationError(format!("Failed to parse derivation path: {e:?}"))
            })?;
        let secp = Secp256k1::new();
        let nostr_key = master_key
            .derive_priv(&secp, &derivation_path)
            .map_err(|e| {
                NostrError::KeyDerivationError(format!("Failed to derive nostr child key: {e:?}"))
            })?;

        let nostr_key =
            SecretKey::from_slice(&nostr_key.private_key.secret_bytes()).map_err(|e| {
                NostrError::KeyDerivationError(format!("failed to serialize nostr key: {e:?}"))
            })?;
        Ok(NostrClient { secp, nostr_key })
    }

    pub fn nostr_pubkey(&self) -> String {
        let (xonly_pubkey, _) = XOnlyPublicKey::from_keypair(&self.nostr_key.keypair(&self.secp));
        xonly_pubkey.to_string()
    }

    pub fn create_zap_receipt(
        &self,
        zap_request: &str,
        payment: &Payment,
    ) -> Result<String, NostrError> {
        // Extract invoice and preimage from payment details
        let Some(PaymentDetails::Lightning {
            invoice, preimage, ..
        }) = &payment.details
        else {
            return Err(NostrError::ZapReceiptCreationError(
                "Payment is not a lightning payment".to_string(),
            ));
        };

        // Convert bitcoin SecretKey to nostr SecretKey
        let keys = Keys::new(self.nostr_key.clone());

        lnurl_models::nostr::create_zap_receipt(zap_request, invoice, preimage.clone(), &keys)
            .map_err(NostrError::ZapReceiptCreationError)?
            .try_as_json()
            .map_err(|e| {
                NostrError::ZapReceiptCreationError(format!("Failed to serialize zap receipt: {e}"))
            })
    }
}
