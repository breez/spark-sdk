//! Passkey-based wallet operations using `WebAuthn` PRF extension.
//!
//! This module implements the [seedless-restore spec](https://github.com/breez/seedless-restore)
//! for deriving wallet seeds from passkey PRF outputs and storing/discovering
//! salts via Nostr relays.
//!
//! # Overview
//!
//! The passkey flow works as follows:
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
//! use breez_sdk_spark::passkey::{Passkey, NostrRelayConfig};
//!
//! // Platform provides a PasskeyPrfProvider implementation
//! let prf_provider = Arc::new(MyPasskeyPrfProvider::new());
//!
//! let passkey = Passkey::new(prf_provider, None);
//!
//! // Get a wallet (creates or restores)
//! let wallet = passkey.get_wallet(Some("my-wallet".to_string())).await?;
//!
//! // List available wallet names for restore
//! let wallet_names = passkey.list_wallet_names().await?;
//!
//! // Store a wallet name to Nostr
//! passkey.store_wallet_name("my-wallet".to_string()).await?;
//! ```

mod derivation;
mod error;
mod models;
mod nostr_client;
mod passkey_prf_provider;

pub use derivation::ACCOUNT_MASTER_SALT;
use derivation::prf_to_mnemonic;
pub use error::{PasskeyError, PasskeyPrfError};
pub use models::{NostrRelayConfig, Wallet};
pub use passkey_prf_provider::PasskeyPrfProvider;

use std::sync::Arc;

use tokio::sync::OnceCell;
use tokio_with_wasm::alias as tokio;

use crate::Seed;
use derivation::derive_nostr_keypair;
use nostr_client::NostrSaltClient;

/// The default wallet name used when none is provided to [`Passkey::get_wallet`].
const DEFAULT_WALLET_NAME: &str = "Default";

/// Maximum allowed wallet name length in bytes.
const MAX_WALLET_NAME_LENGTH: usize = 1024;

/// Validate a user-provided wallet name string.
fn validate_wallet_name(wallet_name: &str) -> Result<(), PasskeyError> {
    if wallet_name.is_empty() {
        return Err(PasskeyError::InvalidSalt(
            "wallet name must not be empty".to_string(),
        ));
    }
    if wallet_name.len() > MAX_WALLET_NAME_LENGTH {
        return Err(PasskeyError::InvalidSalt(format!(
            "wallet name exceeds maximum length of {MAX_WALLET_NAME_LENGTH} bytes"
        )));
    }
    Ok(())
}

/// Orchestrates passkey-based wallet creation and restore operations.
///
/// This struct coordinates between the platform's passkey PRF provider and
/// Nostr relays to derive wallet mnemonics and manage wallet names.
///
/// The Nostr identity (derived from the passkey's magic salt) is cached after
/// the first derivation so that subsequent calls to [`Passkey::list_wallet_names`]
/// and [`Passkey::store_wallet_name`] do not require additional PRF interactions.
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct Passkey {
    prf_provider: Arc<dyn PasskeyPrfProvider>,
    nostr_client: NostrSaltClient,
    /// Cached Nostr identity derived from the passkey's magic salt.
    /// Populated on first use, avoiding repeated PRF calls for Nostr operations.
    nostr_keys: Arc<OnceCell<nostr::Keys>>,
}

impl Passkey {
    /// Derive or retrieve the cached Nostr keypair from the passkey using the magic salt.
    ///
    /// The identity is derived on first call and cached for subsequent use,
    /// so only one PRF interaction (user authentication) is needed.
    async fn derive_nostr_identity(&self) -> Result<nostr::Keys, PasskeyError> {
        self.nostr_keys
            .get_or_try_init(|| async {
                let account_master = self
                    .prf_provider
                    .derive_prf_seed(ACCOUNT_MASTER_SALT.to_string())
                    .await?;
                derive_nostr_keypair(&account_master)
            })
            .await
            .cloned()
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl Passkey {
    /// Create a new `Passkey` instance.
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
            nostr_keys: Arc::new(OnceCell::new()),
        }
    }

    /// Derive a wallet for a given wallet name.
    ///
    /// Uses the passkey PRF to derive a 24-word BIP39 mnemonic from the wallet name
    /// and returns it as a [`Wallet`] containing the seed and resolved name.
    /// This works for both creating a new wallet and restoring an existing one.
    ///
    /// # Arguments
    /// * `wallet_name` - A user-chosen wallet name (e.g., "personal", "business").
    ///   If `None`, defaults to [`DEFAULT_WALLET_NAME`].
    pub async fn get_wallet(&self, wallet_name: Option<String>) -> Result<Wallet, PasskeyError> {
        let name = wallet_name.unwrap_or_else(|| DEFAULT_WALLET_NAME.to_string());
        validate_wallet_name(&name)?;
        let root_key = self.prf_provider.derive_prf_seed(name.clone()).await?;
        let mnemonic = prf_to_mnemonic(&root_key)?;
        Ok(Wallet {
            seed: Seed::Mnemonic {
                mnemonic,
                passphrase: None,
            },
            name,
        })
    }

    /// List all wallet names published to Nostr for this passkey's identity.
    ///
    /// Queries Nostr relays for all wallet names associated with the Nostr identity
    /// derived from this passkey. Requires 1 PRF call.
    pub async fn list_wallet_names(&self) -> Result<Vec<String>, PasskeyError> {
        let nostr_keys = self.derive_nostr_identity().await?;
        self.nostr_client.query_wallet_names(&nostr_keys).await
    }

    /// Publish a wallet name to Nostr relays for this passkey's identity.
    ///
    /// Idempotent: if the wallet name already exists, it is not published again.
    /// Requires 1 PRF call.
    ///
    /// # Arguments
    /// * `wallet_name` - A user-chosen wallet name (e.g., "personal", "business")
    pub async fn store_wallet_name(&self, wallet_name: String) -> Result<(), PasskeyError> {
        validate_wallet_name(&wallet_name)?;
        let nostr_keys = self.derive_nostr_identity().await?;

        let exists = self
            .nostr_client
            .wallet_name_exists(&nostr_keys, &wallet_name)
            .await?;
        if !exists {
            self.nostr_client
                .publish_wallet_name(&nostr_keys, &wallet_name)
                .await?;
        }
        Ok(())
    }

    /// Check if passkey PRF is available on this device.
    ///
    /// Delegates to the platform's `PasskeyPrfProvider` implementation.
    pub async fn is_available(&self) -> Result<bool, PasskeyError> {
        self.prf_provider
            .is_prf_available()
            .await
            .map_err(PasskeyError::from)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

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
            let mut outputs = self.salt_outputs.lock().unwrap();
            if let Some(output) = outputs.get(&salt) {
                return Ok(output.clone());
            }

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

    #[macros::test_all]
    fn test_passkey_new() {
        let prf_provider = Arc::new(MockPasskeyPrfProvider::new([0u8; 32]));
        let config = NostrRelayConfig::default();

        let _passkey = Passkey::new(prf_provider, Some(config));
    }

    #[macros::async_test_all]
    async fn test_is_available() {
        let prf_provider = Arc::new(MockPasskeyPrfProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        let available = passkey.is_available().await.unwrap();
        assert!(available);
    }

    #[macros::async_test_all]
    async fn test_is_available_false() {
        let prf_provider = Arc::new(UnavailablePrfProvider);
        let passkey = Passkey::new(prf_provider, None);

        let available = passkey.is_available().await.unwrap();
        assert!(!available);
    }

    #[macros::async_test_all]
    async fn test_is_available_error() {
        let prf_provider = Arc::new(FailingPasskeyPrfProvider::new(
            PasskeyPrfError::AuthenticationFailed("Test error".to_string()),
        ));
        let passkey = Passkey::new(prf_provider, None);

        let result = passkey.is_available().await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PasskeyError::PrfError(PasskeyPrfError::AuthenticationFailed(_))
        ));
    }

    /// Extract the mnemonic string from a `Wallet`.
    fn unwrap_mnemonic(wallet: Wallet) -> String {
        match wallet.seed {
            Seed::Mnemonic { mnemonic, .. } => mnemonic,
            Seed::Entropy(_) => panic!("Expected Seed::Mnemonic"),
        }
    }

    #[macros::async_test_all]
    async fn test_get_wallet_deterministic() {
        let prf_provider1 = Arc::new(MockPasskeyPrfProvider::new([42u8; 32]));
        let prf_provider2 = Arc::new(MockPasskeyPrfProvider::new([42u8; 32]));

        let passkey1 = Passkey::new(prf_provider1, None);
        let passkey2 = Passkey::new(prf_provider2, None);

        let mnemonic1 =
            unwrap_mnemonic(passkey1.get_wallet(Some("test".to_string())).await.unwrap());
        let mnemonic2 =
            unwrap_mnemonic(passkey2.get_wallet(Some("test".to_string())).await.unwrap());

        assert_eq!(mnemonic1, mnemonic2);
    }

    #[macros::async_test_all]
    async fn test_get_wallet_different_providers() {
        let prf_provider1 = Arc::new(MockPasskeyPrfProvider::new([1u8; 32]));
        let prf_provider2 = Arc::new(MockPasskeyPrfProvider::new([2u8; 32]));

        let passkey1 = Passkey::new(prf_provider1, None);
        let passkey2 = Passkey::new(prf_provider2, None);

        let mnemonic1 =
            unwrap_mnemonic(passkey1.get_wallet(Some("test".to_string())).await.unwrap());
        let mnemonic2 =
            unwrap_mnemonic(passkey2.get_wallet(Some("test".to_string())).await.unwrap());

        assert_ne!(mnemonic1, mnemonic2);
    }

    #[macros::async_test_all]
    async fn test_get_wallet_produces_24_words() {
        let prf_provider = Arc::new(MockPasskeyPrfProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        let mnemonic = unwrap_mnemonic(passkey.get_wallet(Some("test".to_string())).await.unwrap());

        let word_count = mnemonic.split_whitespace().count();
        assert_eq!(word_count, 24, "Mnemonic should be 24 words");
    }

    #[macros::async_test_all]
    async fn test_get_wallet_default_name() {
        let prf_provider = Arc::new(MockPasskeyPrfProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        // None wallet_name should default to "Default" and not error
        let wallet = passkey.get_wallet(None).await.unwrap();
        assert_eq!(wallet.name, "Default");
    }

    #[macros::async_test_all]
    async fn test_get_wallet_custom_name() {
        let prf_provider = Arc::new(MockPasskeyPrfProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        let wallet = passkey
            .get_wallet(Some("personal".to_string()))
            .await
            .unwrap();
        assert_eq!(wallet.name, "personal");
    }

    #[macros::async_test_all]
    async fn test_get_wallet_error_propagation() {
        let prf_provider = Arc::new(FailingPasskeyPrfProvider::new(
            PasskeyPrfError::UserCancelled,
        ));
        let passkey = Passkey::new(prf_provider, None);

        let result = passkey.get_wallet(Some("test".to_string())).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PasskeyError::PrfError(PasskeyPrfError::UserCancelled)
        ));
    }

    #[macros::async_test_all]
    async fn test_different_wallet_names_produce_different_mnemonics() {
        let prf_provider = Arc::new(SaltAwareMockProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        let mnemonic1 = unwrap_mnemonic(
            passkey
                .get_wallet(Some("personal".to_string()))
                .await
                .unwrap(),
        );
        let mnemonic2 = unwrap_mnemonic(
            passkey
                .get_wallet(Some("business".to_string()))
                .await
                .unwrap(),
        );

        assert_ne!(
            mnemonic1, mnemonic2,
            "Different wallet names should produce different mnemonics"
        );
    }

    #[macros::async_test_all]
    async fn test_same_wallet_name_deterministic() {
        let prf_provider = Arc::new(SaltAwareMockProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        let mnemonic1 =
            unwrap_mnemonic(passkey.get_wallet(Some("test".to_string())).await.unwrap());
        let mnemonic2 =
            unwrap_mnemonic(passkey.get_wallet(Some("test".to_string())).await.unwrap());

        assert_eq!(
            mnemonic1, mnemonic2,
            "Same wallet name should produce same mnemonic"
        );
    }

    #[macros::test_all]
    fn test_nostr_relay_config_default() {
        let config = NostrRelayConfig::default();
        assert!(
            config.breez_api_key.is_none(),
            "Default should have no API key"
        );
        assert_eq!(config.timeout_secs(), 30, "Should have 30 sec timeout");
    }

    #[macros::test_all]
    fn test_account_master_salt_is_valid_hex() {
        let decoded = hex::decode(ACCOUNT_MASTER_SALT);
        assert!(decoded.is_ok(), "ACCOUNT_MASTER_SALT should be valid hex");

        let decoded_bytes = decoded.unwrap();
        let decoded_str = String::from_utf8(decoded_bytes);
        assert!(decoded_str.is_ok());
        assert_eq!(decoded_str.unwrap(), "NYOASTRTSAOYN");
    }

    #[macros::async_test_all]
    async fn test_derive_nostr_identity_deterministic() {
        let prf_provider1 = Arc::new(MockPasskeyPrfProvider::new([99u8; 32]));
        let prf_provider2 = Arc::new(MockPasskeyPrfProvider::new([99u8; 32]));

        let passkey1 = Passkey::new(prf_provider1, None);
        let passkey2 = Passkey::new(prf_provider2, None);

        let keys1 = passkey1.derive_nostr_identity().await.unwrap();
        let keys2 = passkey2.derive_nostr_identity().await.unwrap();

        assert_eq!(
            keys1.public_key(),
            keys2.public_key(),
            "Same PRF output should produce same Nostr identity"
        );
    }

    #[macros::async_test_all]
    async fn test_derive_nostr_identity_different_providers() {
        let prf_provider1 = Arc::new(MockPasskeyPrfProvider::new([1u8; 32]));
        let prf_provider2 = Arc::new(MockPasskeyPrfProvider::new([2u8; 32]));

        let passkey1 = Passkey::new(prf_provider1, None);
        let passkey2 = Passkey::new(prf_provider2, None);

        let keys1 = passkey1.derive_nostr_identity().await.unwrap();
        let keys2 = passkey2.derive_nostr_identity().await.unwrap();

        assert_ne!(
            keys1.public_key(),
            keys2.public_key(),
            "Different PRF outputs should produce different Nostr identities"
        );
    }
}
