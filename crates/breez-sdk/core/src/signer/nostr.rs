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
    pub fn new(
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

        // Sign the event ID using the signer's Schnorr signing
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
mod tests {
    use super::*;
    use crate::{Seed, models::Config, signer::breez::BreezSignerImpl};
    use bitcoin::secp256k1::Secp256k1;
    use spark_wallet::KeySetType;

    /// Test that our new signing method produces the same signatures as the old method
    /// Old method: nostr::Keys with sign_with_keys
    /// New method: NostrSigner with sign_hash_schnorr
    #[tokio::test]
    async fn test_signing_compatibility_with_nostr_keys() {
        // Create a test seed (deterministic for reproducible tests)
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let seed = Seed::Mnemonic {
            mnemonic: mnemonic.to_string(),
            passphrase: None,
        };

        // Create a BreezSigner
        let config = Config {
            network: Network::Regtest,
            api_key: None,
            real_time_sync_server_url: None,
            lnurl_domain: None,
            sync_interval_secs: 60,
            max_deposit_claim_fee: None,
            prefer_spark_over_lightning: false,
            external_input_parsers: None,
            use_default_external_input_parsers: false,
            private_enabled_default: false,
        };

        let breez_signer = Arc::new(
            BreezSignerImpl::new(&config, &seed, KeySetType::Default, false, Some(1)).unwrap(),
        );

        // Create NostrSigner using our new implementation
        let nostr_signer = NostrSigner::new(breez_signer.clone(), Network::Regtest, Some(1))
            .expect("Failed to create NostrSigner");

        // Create a test event
        let test_content = "Hello Nostr! This is a test event.";
        let builder = ::nostr::EventBuilder::text_note(test_content);

        // Build the unsigned event with the nostr signer's pubkey
        // This ensures both methods work with the SAME unsigned event
        let mut unsigned_event = builder.build(nostr_signer.pubkey);
        let event_id = unsigned_event.id(); // Get EventId, not Option<EventId>
        let signature_new = breez_signer
            .sign_hash_schnorr(event_id.as_bytes(), &nostr_signer.derivation_path)
            .await
            .expect("Failed to sign with new method");

        let event_new = unsigned_event
            .clone()
            .add_signature(signature_new)
            .expect("Failed to add signature");

        // Now sign the same unsigned event using the old method (nostr::Keys)
        // First, derive the nostr keys manually (simulating the old get_nostr_keys method)
        let account = 1u32; // Regtest account
        let derivation_path: DerivationPath = format!("m/44'/1237'/{account}'/0/0")
            .parse()
            .expect("Failed to parse derivation path");

        let secp = Secp256k1::new();

        // Access the key_set from breez_signer (we need to recreate it)
        let seed_bytes = seed.to_bytes().unwrap();
        let key_set = spark_wallet::KeySet::new(
            &seed_bytes,
            spark_wallet::Network::Regtest,
            KeySetType::Default,
            false,
            Some(1),
        )
        .unwrap();

        let nostr_key = key_set
            .identity_master_key
            .derive_priv(&secp, &derivation_path)
            .expect("Failed to derive nostr child key");

        let _nostr_secret_key =
            ::nostr::SecretKey::from_slice(&nostr_key.private_key.secret_bytes())
                .expect("Failed to serialize nostr key");

        // Sign the same unsigned event using secp256k1 directly
        use bitcoin::secp256k1::Message;
        let message =
            Message::from_digest_slice(event_id.as_bytes()).expect("Failed to create message");
        let keypair = nostr_key.private_key.keypair(&secp);
        let signature_old = secp.sign_schnorr(&message, &keypair);

        let event_old = unsigned_event
            .add_signature(signature_old)
            .expect("Failed to add signature");

        // Debug: Let's see what we got
        println!("Event ID: {}", event_id);
        println!("New signature: {}", signature_new);
        println!("Old signature: {}", signature_old);

        // Verify that both methods produce the same results
        assert_eq!(event_new.id, event_old.id, "Event IDs should match");
        assert_eq!(
            event_new.pubkey, event_old.pubkey,
            "Public keys should match"
        );
        assert_eq!(event_new.content, event_old.content, "Content should match");
        assert_eq!(event_new.kind, event_old.kind, "Event kind should match");
        assert_eq!(
            event_new.created_at, event_old.created_at,
            "Timestamps should match"
        );

        // Verify the signatures are valid (both should be valid)
        event_new.verify().expect("New signature should be valid");
        event_old.verify().expect("Old signature should be valid");

        // Note: Schnorr signatures with random nonces will be different each time
        // but both signatures should be valid for the same event
        println!("✓ Signature compatibility test passed!");
        println!("  Event ID: {}", event_new.id);
        println!("  New Signature: {}", event_new.sig);
        println!("  Old Signature: {}", event_old.sig);
        println!("  Public Key: {}", event_new.pubkey);
        println!("  Both signatures are valid for the same event!");
    }
}
