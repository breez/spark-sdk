use std::sync::Arc;

use crate::{Network, signer::BreezSigner};
use bitcoin::bip32::DerivationPath;

#[derive(thiserror::Error, Debug)]
pub enum NostrSignerError {
    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),
    #[error("Zap receipt creation error: {0}")]
    ZapReceiptCreationError(String),
}

pub struct NostrSigner {
    signer: Arc<dyn BreezSigner>,
    pubkey: ::nostr::PublicKey,
    derivation_path: DerivationPath,
}

impl NostrSigner {
    pub async fn new(
        signer: Arc<dyn BreezSigner>,
        network: Network,
        account_number: Option<u32>,
    ) -> Result<Self, NostrSignerError> {
        let account = account_number.unwrap_or(match network {
            Network::Mainnet => 0,
            Network::Regtest => 1,
        });

        let derivation_path: DerivationPath =
            format!("m/44'/1237'/{account}'/0/0").parse().map_err(|e| {
                NostrSignerError::KeyDerivationError(format!(
                    "Failed to parse derivation path: {e:?}"
                ))
            })?;

        let pubkey_secp = signer
            .derive_public_key(&derivation_path)
            .await
            .map_err(|e| NostrSignerError::KeyDerivationError(e.to_string()))?;

        // Convert secp256k1::PublicKey to nostr::PublicKey (x-only)
        let pubkey = ::nostr::PublicKey::from_slice(&pubkey_secp.x_only_public_key().0.serialize())
            .map_err(|e| NostrSignerError::KeyDerivationError(e.to_string()))?;

        Ok(Self {
            signer,
            pubkey,
            derivation_path,
        })
    }

    pub fn nostr_pubkey(&self) -> String {
        self.pubkey.to_string()
    }

    pub async fn sign_event(
        &self,
        builder: ::nostr::event::EventBuilder,
    ) -> Result<::nostr::event::Event, NostrSignerError> {
        // Build the unsigned event
        let mut unsigned_event = builder.build(self.pubkey);

        // Get the event ID (ensures it's computed if not already set)
        let event_id = unsigned_event.id();

        // Sign the event ID using the signer's Schnorr signing (always uses auxiliary randomness)
        let signature = self
            .signer
            .sign_hash_schnorr(event_id.as_bytes(), &self.derivation_path)
            .await
            .map_err(|e| NostrSignerError::ZapReceiptCreationError(e.to_string()))?;

        // Add signature to create the signed event
        // Note: bitcoin::secp256k1::schnorr::Signature is compatible with the nostr library's expected type
        unsigned_event
            .add_signature(signature)
            .map_err(|e| NostrSignerError::ZapReceiptCreationError(e.to_string()))
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use std::sync::Arc;

    use super::NostrSigner;
    use crate::default_config;
    use crate::signer::breez::BreezSignerImpl;
    use crate::{Network, Seed, signer::BreezSigner};
    use spark_wallet::KeySetType;

    #[tokio::test]
    async fn test_signing() {
        // Create a test seed (deterministic for reproducible tests)
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let seed = Seed::Mnemonic {
            mnemonic: mnemonic.to_string(),
            passphrase: None,
        };

        // Create a BreezSigner
        let config = default_config(Network::Regtest);

        let breez_signer: Arc<dyn BreezSigner> = Arc::new(
            BreezSignerImpl::new(&config, &seed, KeySetType::Default, false, Some(1)).unwrap(),
        );

        // Create NostrSigner using our implementation
        let nostr_signer = NostrSigner::new(breez_signer.clone(), Network::Regtest, Some(1))
            .await
            .expect("Failed to create NostrSigner");

        // Create a test event
        let test_content = "Hello Nostr! This is a test event.";
        let builder = ::nostr::EventBuilder::text_note(test_content);

        // Sign the event using our NostrSigner
        let signed_event = nostr_signer
            .sign_event(builder)
            .await
            .expect("Failed to sign event");

        // Verify the signature is valid
        signed_event.verify().expect("Signature should be valid");

        // Verify event properties
        assert_eq!(signed_event.content, test_content, "Content should match");
        assert_eq!(
            signed_event.pubkey.to_string(),
            nostr_signer.nostr_pubkey(),
            "Public key should match"
        );

        println!("✓ Nostr signing test passed!");
        println!("  Event ID: {}", signed_event.id);
        println!("  Signature: {}", signed_event.sig);
        println!("  Public Key: {}", signed_event.pubkey);
    }
}
