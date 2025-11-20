use bitcoin::{
    Network, XOnlyPublicKey,
    bip32::{DerivationPath, Xpriv},
    key::Secp256k1,
    secp256k1::SecretKey,
};
use nostr::{EventBuilder, JsonUtil, Keys};

use crate::{Payment, PaymentDetails};

#[derive(Debug)]
pub enum NostrError {
    KeyDerivationError(String),
    ZapReceiptCreationError(String),
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

    pub fn create_zap_receipt(
        &self,
        zap_request: &str,
        payment: &Payment,
    ) -> Result<String, NostrError> {
        // Parse the zap request event
        let zap_request_event = nostr::Event::from_json(zap_request).map_err(|e| {
            NostrError::ZapReceiptCreationError(format!("Failed to parse zap request: {e}"))
        })?;

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
        let nostr_secret_key = nostr::SecretKey::from_slice(&self.nostr_key.secret_bytes())
            .map_err(|e| {
                NostrError::ZapReceiptCreationError(format!("Failed to convert secret key: {e}"))
            })?;
        let keys = Keys::new(nostr_secret_key);

        // Build and sign the zap receipt event
        let zap_receipt = EventBuilder::zap_receipt(invoice, preimage.clone(), &zap_request_event)
            .sign_with_keys(&keys)
            .map_err(|e| {
                NostrError::ZapReceiptCreationError(format!("Failed to build zap receipt: {e}"))
            })?;

        zap_receipt.try_as_json().map_err(|e| {
            NostrError::ZapReceiptCreationError(format!(
                "Failed to convert zap receipt to JSON: {e}"
            ))
        })
    }
}
