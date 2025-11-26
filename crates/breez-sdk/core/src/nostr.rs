use bitcoin::bip32::{DerivationPath, Xpriv};
use nostr::{
    EventBuilder, JsonUtil, Keys, SecretKey,
    event::{TagKind, TagStandard},
    filter::{Alphabet, SingleLetterTag},
    secp256k1::{All, Secp256k1, XOnlyPublicKey},
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
        let keys = Keys::new(self.nostr_key.clone());

        let Some(p_tag) = zap_request_event.tags.iter().find(|t| {
            t.kind()
                == TagKind::SingleLetter(SingleLetterTag {
                    character: Alphabet::P,
                    uppercase: false,
                })
        }) else {
            return Err(NostrError::ZapReceiptCreationError(
                "Zap request event missing 'p' tag".to_string(),
            ));
        };

        let Some(TagStandard::PublicKey { public_key, .. }) = p_tag.as_standardized() else {
            return Err(NostrError::ZapReceiptCreationError(
                "Zap request event 'p' tag is not a public key".to_string(),
            ));
        };

        if self.nostr_pubkey() != public_key.to_string() {
            return Err(NostrError::ZapReceiptCreationError(
                "Nostr client key does not match zap request 'p' tag".to_string(),
            ));
        }

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
