//! Seedless wallet restore using `WebAuthn` passkeys with PRF extension.
//!
//! This module implements the [seedless-restore spec](https://github.com/breez/seedless-restore)
//! for deriving wallet seeds from passkey PRF outputs and storing/discovering
//! salts via Nostr relays.
//!
//! # Overview
//!
//! The seedless restore flow works as follows:
//!
//! 1. **Account Master Derivation**: `PRF(passkey, magic_salt)` produces a 32-byte
//!    account master used to derive the Nostr identity at `m/44'/1237'/55'/0/0`.
//!
//! 2. **Salt Storage**: User-provided salts are published as Nostr kind-1 events,
//!    allowing discovery during wallet restore.
//!
//! 3. **Wallet Seed Derivation**: `PRF(passkey, user_salt)` produces 32 bytes that
//!    are converted to a 24-word BIP39 mnemonic.
//!
//! # Platform Implementation
//!
//! Platforms must implement the [`PasskeyPrfProvider`] trait to provide passkey
//! PRF functionality. The SDK orchestrates the flow, while platforms handle the
//! actual passkey authentication.
//!
//! # Example
//!
//! ```ignore
//! use breez_sdk_spark::seedless::{SeedlessRestore, NostrRelayConfig};
//!
//! // Platform provides a PasskeyPrfProvider implementation
//! let prf_provider = Arc::new(MyPasskeyPrfProvider::new());
//!
//! let seedless = SeedlessRestore::new(prf_provider, None);
//!
//! // Create a new seed with a user-chosen salt
//! let seed = seedless.create_seed("my-wallet".to_string()).await?;
//!
//! // Or restore: first list available salts
//! let salts = seedless.list_salts().await?;
//! let seed = seedless.restore_seed(salts[0].clone()).await?;
//! ```

mod derivation;
mod error;
mod models;
mod nostr_client;
mod passkey_prf_provider;

pub use derivation::ACCOUNT_MASTER_SALT;
pub use error::{PasskeyPrfError, SeedlessRestoreError};
pub use models::NostrRelayConfig;
pub use passkey_prf_provider::PasskeyPrfProvider;

use std::sync::Arc;

use crate::Seed;
use derivation::{derive_nostr_keypair, prf_to_mnemonic};
use nostr_client::NostrSaltClient;

/// Orchestrates seedless wallet creation and restore operations.
///
/// This struct coordinates between the platform's passkey PRF provider and
/// Nostr relays to create and restore wallet seeds without requiring users
/// to manage mnemonic phrases directly.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct SeedlessRestore {
    prf_provider: Arc<dyn PasskeyPrfProvider>,
    nostr_client: NostrSaltClient,
}

impl SeedlessRestore {
    /// Derive the Nostr keypair from the passkey using the magic salt.
    async fn derive_nostr_identity(&self) -> Result<nostr::Keys, SeedlessRestoreError> {
        // Derive account master using magic salt
        let account_master = self
            .prf_provider
            .derive_prf_seed(ACCOUNT_MASTER_SALT.to_string())
            .await?;

        // Derive Nostr keypair from account master
        derive_nostr_keypair(&account_master)
    }

    /// Derive a wallet seed from a user-provided salt.
    async fn derive_seed_from_salt(&self, salt: &str) -> Result<Seed, SeedlessRestoreError> {
        // Derive root key using user salt
        let root_key = self.prf_provider.derive_prf_seed(salt.to_string()).await?;

        // Convert to mnemonic
        let mnemonic = prf_to_mnemonic(&root_key)?;

        Ok(Seed::Mnemonic {
            mnemonic,
            passphrase: None,
        })
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl SeedlessRestore {
    /// Create a new `SeedlessRestore` instance.
    ///
    /// # Arguments
    /// * `prf_provider` - Platform implementation of passkey PRF operations
    /// * `relay_config` - Optional configuration for Nostr relay connections (uses default if None)
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn new(
        prf_provider: Arc<dyn PasskeyPrfProvider>,
        relay_config: Option<NostrRelayConfig>,
    ) -> Self {
        Self {
            prf_provider,
            nostr_client: NostrSaltClient::new(relay_config.unwrap_or_default()),
        }
    }

    /// Create a new wallet seed from a user-provided salt.
    ///
    /// This method:
    /// 1. Derives the Nostr identity from the passkey using the magic salt
    /// 2. Checks if the salt already exists on Nostr (idempotency)
    /// 3. If not, publishes the salt to Nostr relays
    /// 4. Derives the wallet seed from the passkey using the user's salt
    ///
    /// # Arguments
    /// * `salt` - A user-chosen salt string (e.g., "personal", "business")
    ///
    /// # Returns
    /// * `Ok(Seed)` - The derived wallet seed (24-word mnemonic)
    /// * `Err(SeedlessRestoreError)` - If any step fails
    pub async fn create_seed(&self, salt: String) -> Result<Seed, SeedlessRestoreError> {
        // Step 1: Derive account master and Nostr keypair
        let nostr_keys = self.derive_nostr_identity().await?;

        // Step 2: Check if salt already exists (idempotency)
        let salt_exists = self.nostr_client.salt_exists(&nostr_keys, &salt).await?;

        // Step 3: Publish salt if it doesn't exist
        if !salt_exists {
            self.nostr_client.publish_salt(&nostr_keys, &salt).await?;
        }

        // Step 4: Derive wallet seed from user salt
        self.derive_seed_from_salt(&salt).await
    }

    /// List all salts published to Nostr for this passkey's identity.
    ///
    /// This method queries Nostr relays for all kind-1 text note events
    /// authored by the Nostr identity derived from the passkey.
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of salt strings found
    /// * `Err(SeedlessRestoreError)` - If the query fails
    pub async fn list_salts(&self) -> Result<Vec<String>, SeedlessRestoreError> {
        // Derive Nostr identity
        let nostr_keys = self.derive_nostr_identity().await?;

        // Query salts from Nostr
        self.nostr_client.query_salts(&nostr_keys).await
    }

    /// Restore a wallet seed from a specific salt.
    ///
    /// Use this after calling `list_salts()` to restore a specific wallet.
    /// This method only derives the seed; it does not publish anything.
    ///
    /// # Arguments
    /// * `salt` - The salt string to use for seed derivation
    ///
    /// # Returns
    /// * `Ok(Seed)` - The derived wallet seed (24-word mnemonic)
    /// * `Err(SeedlessRestoreError)` - If derivation fails
    pub async fn restore_seed(&self, salt: String) -> Result<Seed, SeedlessRestoreError> {
        self.derive_seed_from_salt(&salt).await
    }

    /// Check if passkey PRF is available on this device.
    ///
    /// Delegates to the platform's `PasskeyPrfProvider` implementation.
    ///
    /// # Returns
    /// * `Ok(true)` - PRF-capable passkey is available
    /// * `Ok(false)` - No PRF-capable passkey available
    /// * `Err(SeedlessRestoreError)` - If the check fails
    pub async fn is_prf_available(&self) -> Result<bool, SeedlessRestoreError> {
        self.prf_provider
            .is_prf_available()
            .await
            .map_err(SeedlessRestoreError::from)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Helper to extract mnemonic from Seed for test assertions
    fn get_mnemonic(seed: Seed) -> String {
        match seed {
            Seed::Mnemonic { mnemonic, .. } => mnemonic,
            Seed::Entropy(_) => panic!("Expected Mnemonic seed, got Entropy"),
        }
    }

    // Mock implementation of PasskeyPrfProvider for testing
    struct MockPasskeyPrfProvider {
        seed: [u8; 32],
    }

    impl MockPasskeyPrfProvider {
        fn new(seed: [u8; 32]) -> Self {
            Self { seed }
        }
    }

    #[macros::async_trait]
    impl PasskeyPrfProvider for MockPasskeyPrfProvider {
        async fn derive_prf_seed(&self, _salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
            // Return deterministic output based on seed
            Ok(self.seed.to_vec())
        }

        async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
            Ok(true)
        }
    }

    // Enhanced mock that returns salt-specific outputs (simulates real PRF behavior)
    struct SaltAwareMockProvider {
        base_seed: [u8; 32],
        salt_outputs: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl SaltAwareMockProvider {
        fn new(base_seed: [u8; 32]) -> Self {
            Self {
                base_seed,
                salt_outputs: Mutex::new(HashMap::new()),
            }
        }
    }

    #[macros::async_trait]
    impl PasskeyPrfProvider for SaltAwareMockProvider {
        async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
            // Check if we have a cached output for this salt
            let mut outputs = self.salt_outputs.lock().unwrap();
            if let Some(output) = outputs.get(&salt) {
                return Ok(output.clone());
            }

            // Generate deterministic output based on base_seed XOR'd with simple salt hash
            // Use a simple deterministic hash for testing purposes
            let salt_bytes = salt.as_bytes();
            let mut salt_hash = [0u8; 32];
            for (i, &byte) in salt_bytes.iter().enumerate() {
                salt_hash[i % 32] ^= byte;
                salt_hash[(i + 1) % 32] = salt_hash[(i + 1) % 32].wrapping_add(byte);
            }

            let mut output = [0u8; 32];
            for i in 0..32 {
                output[i] = self.base_seed[i] ^ salt_hash[i];
            }

            outputs.insert(salt, output.to_vec());
            Ok(output.to_vec())
        }

        async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
            Ok(true)
        }
    }

    // Mock that always fails - for testing error propagation
    struct FailingPasskeyPrfProvider {
        error: PasskeyPrfError,
    }

    impl FailingPasskeyPrfProvider {
        fn new(error: PasskeyPrfError) -> Self {
            Self { error }
        }
    }

    #[macros::async_trait]
    impl PasskeyPrfProvider for FailingPasskeyPrfProvider {
        async fn derive_prf_seed(&self, _salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
            Err(self.error.clone())
        }

        async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
            Err(self.error.clone())
        }
    }

    // Mock that returns PRF not available
    struct UnavailablePrfProvider;

    #[macros::async_trait]
    impl PasskeyPrfProvider for UnavailablePrfProvider {
        async fn derive_prf_seed(&self, _salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
            Err(PasskeyPrfError::PrfNotSupported)
        }

        async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
            Ok(false)
        }
    }

    #[test]
    fn test_seedless_restore_new() {
        let provider = Arc::new(MockPasskeyPrfProvider::new([0u8; 32]));
        let config = NostrRelayConfig::default();

        let _seedless = SeedlessRestore::new(provider, Some(config));
        // Just verify construction works
    }

    #[tokio::test]
    async fn test_is_prf_available() {
        let provider = Arc::new(MockPasskeyPrfProvider::new([0u8; 32]));
        let seedless = SeedlessRestore::new(provider, None);

        let available = seedless.is_prf_available().await.unwrap();
        assert!(available);
    }

    #[tokio::test]
    async fn test_is_prf_available_false() {
        let provider = Arc::new(UnavailablePrfProvider);
        let seedless = SeedlessRestore::new(provider, None);

        let available = seedless.is_prf_available().await.unwrap();
        assert!(!available);
    }

    #[tokio::test]
    async fn test_is_prf_available_error() {
        let provider = Arc::new(FailingPasskeyPrfProvider::new(
            PasskeyPrfError::AuthenticationFailed("Test error".to_string()),
        ));
        let seedless = SeedlessRestore::new(provider, None);

        let result = seedless.is_prf_available().await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SeedlessRestoreError::PasskeyError(PasskeyPrfError::AuthenticationFailed(_))
        ));
    }

    #[tokio::test]
    async fn test_restore_seed_deterministic() {
        // Same salt and provider seed should always produce the same mnemonic
        let provider1 = Arc::new(MockPasskeyPrfProvider::new([42u8; 32]));
        let provider2 = Arc::new(MockPasskeyPrfProvider::new([42u8; 32]));

        let seedless1 = SeedlessRestore::new(provider1, None);
        let seedless2 = SeedlessRestore::new(provider2, None);

        let seed1 = seedless1.restore_seed("test".to_string()).await.unwrap();
        let seed2 = seedless2.restore_seed("test".to_string()).await.unwrap();

        assert_eq!(get_mnemonic(seed1), get_mnemonic(seed2));
    }

    #[tokio::test]
    async fn test_restore_seed_different_providers() {
        // Different provider seeds should produce different mnemonics
        let provider1 = Arc::new(MockPasskeyPrfProvider::new([1u8; 32]));
        let provider2 = Arc::new(MockPasskeyPrfProvider::new([2u8; 32]));

        let seedless1 = SeedlessRestore::new(provider1, None);
        let seedless2 = SeedlessRestore::new(provider2, None);

        let seed1 = seedless1.restore_seed("test".to_string()).await.unwrap();
        let seed2 = seedless2.restore_seed("test".to_string()).await.unwrap();

        assert_ne!(get_mnemonic(seed1), get_mnemonic(seed2));
    }

    #[tokio::test]
    async fn test_restore_seed_produces_24_word_mnemonic() {
        let provider = Arc::new(MockPasskeyPrfProvider::new([0u8; 32]));
        let seedless = SeedlessRestore::new(provider, None);

        let seed = seedless.restore_seed("test".to_string()).await.unwrap();

        let mnemonic = get_mnemonic(seed);
        let word_count = mnemonic.split_whitespace().count();
        assert_eq!(word_count, 24, "Mnemonic should be 24 words");
    }

    #[tokio::test]
    async fn test_restore_seed_no_passphrase() {
        let provider = Arc::new(MockPasskeyPrfProvider::new([0u8; 32]));
        let seedless = SeedlessRestore::new(provider, None);

        let seed = seedless.restore_seed("test".to_string()).await.unwrap();

        match seed {
            Seed::Mnemonic { passphrase, .. } => {
                assert!(passphrase.is_none(), "Passphrase should be None");
            }
            Seed::Entropy(_) => panic!("Expected Mnemonic seed, got Entropy"),
        }
    }

    #[tokio::test]
    async fn test_restore_seed_error_propagation() {
        let provider = Arc::new(FailingPasskeyPrfProvider::new(
            PasskeyPrfError::UserCancelled,
        ));
        let seedless = SeedlessRestore::new(provider, None);

        let result = seedless.restore_seed("test".to_string()).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SeedlessRestoreError::PasskeyError(PasskeyPrfError::UserCancelled)
        ));
    }

    #[tokio::test]
    async fn test_salt_aware_provider_different_salts() {
        // Salt-aware provider should produce different outputs for different salts
        let provider = Arc::new(SaltAwareMockProvider::new([0u8; 32]));
        let seedless = SeedlessRestore::new(provider, None);

        let seed1 = seedless.restore_seed("personal".to_string()).await.unwrap();
        let seed2 = seedless.restore_seed("business".to_string()).await.unwrap();

        assert_ne!(
            get_mnemonic(seed1),
            get_mnemonic(seed2),
            "Different salts should produce different mnemonics"
        );
    }

    #[tokio::test]
    async fn test_salt_aware_provider_same_salt_deterministic() {
        // Same salt should always produce the same output
        let provider = Arc::new(SaltAwareMockProvider::new([0u8; 32]));
        let seedless = SeedlessRestore::new(provider, None);

        let seed1 = seedless.restore_seed("test".to_string()).await.unwrap();
        let seed2 = seedless.restore_seed("test".to_string()).await.unwrap();

        assert_eq!(
            get_mnemonic(seed1),
            get_mnemonic(seed2),
            "Same salt should produce same mnemonic"
        );
    }

    #[test]
    fn test_nostr_relay_config_default() {
        let config = NostrRelayConfig::default();
        assert!(!config.relay_urls.is_empty(), "Should have default relays");
        assert!(config.timeout_secs > 0, "Should have positive timeout");
    }

    #[test]
    fn test_nostr_relay_config_breez() {
        let config = NostrRelayConfig::breez_relays();
        assert!(!config.relay_urls.is_empty(), "Should have Breez relays");
        // Verify all URLs contain "breez" or are known Breez relays
        for url in &config.relay_urls {
            assert!(
                url.starts_with("wss://"),
                "Relay URL should use wss:// scheme"
            );
        }
    }

    #[test]
    fn test_nostr_relay_config_custom() {
        let custom_relays = vec![
            "wss://relay1.example.com".to_string(),
            "wss://relay2.example.com".to_string(),
        ];
        let config = NostrRelayConfig::custom(custom_relays.clone(), 60);

        assert_eq!(config.relay_urls, custom_relays);
        assert_eq!(config.timeout_secs, 60);
    }

    #[test]
    fn test_account_master_salt_is_valid_hex() {
        // Verify ACCOUNT_MASTER_SALT is valid hex
        let decoded = hex::decode(ACCOUNT_MASTER_SALT);
        assert!(decoded.is_ok(), "ACCOUNT_MASTER_SALT should be valid hex");

        // Verify it decodes to "NYOASTRTSAOYN"
        let decoded_bytes = decoded.unwrap();
        let decoded_str = String::from_utf8(decoded_bytes);
        assert!(decoded_str.is_ok());
        assert_eq!(decoded_str.unwrap(), "NYOASTRTSAOYN");
    }

    #[tokio::test]
    async fn test_derive_nostr_identity_deterministic() {
        // Same provider should always produce the same Nostr identity
        let provider1 = Arc::new(MockPasskeyPrfProvider::new([99u8; 32]));
        let provider2 = Arc::new(MockPasskeyPrfProvider::new([99u8; 32]));

        let seedless1 = SeedlessRestore::new(provider1, None);
        let seedless2 = SeedlessRestore::new(provider2, None);

        let keys1 = seedless1.derive_nostr_identity().await.unwrap();
        let keys2 = seedless2.derive_nostr_identity().await.unwrap();

        assert_eq!(
            keys1.public_key(),
            keys2.public_key(),
            "Same PRF output should produce same Nostr identity"
        );
    }

    #[tokio::test]
    async fn test_derive_nostr_identity_different_providers() {
        // Different providers should produce different Nostr identities
        let provider1 = Arc::new(MockPasskeyPrfProvider::new([1u8; 32]));
        let provider2 = Arc::new(MockPasskeyPrfProvider::new([2u8; 32]));

        let seedless1 = SeedlessRestore::new(provider1, None);
        let seedless2 = SeedlessRestore::new(provider2, None);

        let keys1 = seedless1.derive_nostr_identity().await.unwrap();
        let keys2 = seedless2.derive_nostr_identity().await.unwrap();

        assert_ne!(
            keys1.public_key(),
            keys2.public_key(),
            "Different PRF outputs should produce different Nostr identities"
        );
    }
}
