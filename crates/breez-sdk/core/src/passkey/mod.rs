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
//! 3. **Wallet Seed Derivation**: `PRF(passkey, user_salt)` produces 32 bytes, the first
//!    16 of which are converted to a 12-word BIP39 mnemonic.
//!
//! # Platform Implementation
//!
//! Platforms must implement the [`PrfProvider`] trait to derive wallet seeds.
//! The built-in `PasskeyProvider` on each platform satisfies this contract
//! by authenticating with a platform passkey; custom CLI providers (`YubiKey`,
//! FIDO2, file-backed) implement the same trait for deterministic derivation
//! from other sources. The SDK orchestrates the flow, while implementations
//! handle the actual derivation.
//!
//! # Example
//!
//! ```ignore
//! use breez_sdk_spark::passkey::{Passkey, NostrRelayConfig};
//!
//! // Platform provides a PrfProvider implementation
//! let prf_provider = Arc::new(MyPrfProvider::new());
//!
//! let passkey = Passkey::new(prf_provider, None);
//!
//! // Get a wallet (creates or restores)
//! let wallet = passkey.get_wallet(Some("my-wallet".to_string())).await?;
//!
//! // List available labels for restore
//! let labels = passkey.list_labels().await?;
//!
//! // Store a label to Nostr
//! passkey.store_label("my-wallet".to_string()).await?;
//! ```

mod derivation;
mod error;
mod label_store;
mod models;
mod nostr_client;
mod passkey_prf_provider;

pub use derivation::ACCOUNT_MASTER_SALT;
use derivation::prf_to_mnemonic;
pub use error::{ErrorKind, PasskeyError, PasskeyPrfError};
pub use label_store::{Identity, LabelStore};
pub use models::{
    CreatePasskeyRequest, NamedSalt, NostrRelayConfig, RegisteredCredential, SetupWalletRequest,
    Wallet, WalletSetup,
};
pub use passkey_prf_provider::{DomainAssociation, PrfProvider};

use std::collections::HashMap;
use std::sync::Arc;

use platform_utils::tokio;
use tokio::sync::OnceCell;
use tracing::warn;

use crate::Seed;
use derivation::derive_nostr_keypair;
use nostr_client::NostrSaltClient;

/// The default label used when none is provided to [`Passkey::get_wallet`].
const DEFAULT_LABEL: &str = "Default";

/// Maximum allowed label length in bytes.
const MAX_LABEL_LENGTH: usize = 1024;

/// Wire prefix for caller-supplied [`NamedSalt`] entries. Keeps the
/// host-controlled namespace separate from existing label salts.
const APP_SALT_PREFIX: &str = "app.";

/// Validate a user-provided label string.
fn validate_label(label: &str) -> Result<(), PasskeyError> {
    if label.is_empty() {
        return Err(PasskeyError::InvalidSalt(
            "label must not be empty".to_string(),
        ));
    }
    if label.len() > MAX_LABEL_LENGTH {
        return Err(PasskeyError::InvalidSalt(format!(
            "label exceeds maximum length of {MAX_LABEL_LENGTH} bytes"
        )));
    }
    Ok(())
}

/// Orchestrates passkey-based wallet derivation and label
/// management. Composes a [`PrfProvider`] (for the deterministic
/// 32-byte derivation) with a [`LabelStore`] (for label sync). The
/// default label store is Nostr-backed; integrators can swap it for
/// a server-mediated store via [`Passkey::with_label_store`].
///
/// The passkey identity (derived from the account-master PRF salt)
/// is cached after the first derivation so subsequent label ops on
/// the same instance need no additional PRF prompts.
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct Passkey {
    prf_provider: Arc<dyn PrfProvider>,
    label_store: Arc<dyn LabelStore>,
    /// Cached identity (Nostr keypair) derived from the passkey's
    /// account-master salt; populated lazily on first label op.
    identity: Arc<OnceCell<Identity>>,
}

impl Passkey {
    /// Derive or retrieve the cached identity. One PRF call per
    /// `Passkey` instance lifetime; the cache is shared via `Arc`.
    async fn derive_identity(&self) -> Result<Identity, PasskeyError> {
        self.identity
            .get_or_try_init(|| async {
                let account_master = self
                    .prf_provider
                    .derive_prf_seed(ACCOUNT_MASTER_SALT.to_string())
                    .await?;
                let keys = derive_nostr_keypair(&account_master)?;
                Ok(Identity { keys })
            })
            .await
            .cloned()
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl Passkey {
    /// Create a `Passkey` with the default Nostr-backed label store.
    /// `relay_config` of `None` falls back to default relays (no API
    /// key, default timeout).
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn new(prf_provider: Arc<dyn PrfProvider>, relay_config: Option<NostrRelayConfig>) -> Self {
        let nostr = NostrSaltClient::new(relay_config.unwrap_or_default());
        Self::with_label_store(prf_provider, Arc::new(nostr))
    }

    /// Derive a wallet for a given label.
    ///
    /// Uses the passkey PRF to derive a 12-word BIP39 mnemonic from the label
    /// and returns it as a [`Wallet`] containing the seed and resolved label.
    /// This works for both creating a new wallet and restoring an existing one.
    ///
    /// # Arguments
    /// * `label` - A user-chosen label (e.g., "personal", "business").
    ///   If `None`, defaults to [`DEFAULT_LABEL`].
    pub async fn get_wallet(&self, label: Option<String>) -> Result<Wallet, PasskeyError> {
        let label = label.unwrap_or_else(|| DEFAULT_LABEL.to_string());
        validate_label(&label)?;
        let root_key = self.prf_provider.derive_prf_seed(label.clone()).await?;
        let mnemonic = prf_to_mnemonic(&root_key)?;
        Ok(Wallet {
            seed: Seed::Mnemonic {
                mnemonic,
                passphrase: None,
            },
            label,
        })
    }

    /// Single-prompt setup: derive the Nostr identity, the wallet seed
    /// for `request.label`, and any caller-supplied [`NamedSalt`]s, all
    /// in one PRF ceremony where the platform supports it. Primes the
    /// Nostr identity cache; conditionally publishes the label.
    ///
    /// On platforms whose [`PrfProvider`] implements the dual-salt fast
    /// path, N inputs cost ⌈N / 2⌉ user prompts. The default trait
    /// impl loops, so on platforms without that override the prompt
    /// count equals N. Built-in `PasskeyProvider`s override.
    ///
    /// The Nostr identity is cached after this call; subsequent
    /// [`Self::list_labels`] / [`Self::store_label`] calls on the same
    /// `Passkey` instance need no additional PRF interactions.
    ///
    /// `publish_label = false` enables **speculative cold-restore**:
    /// derive a candidate wallet for a guessed label without polluting
    /// the user's Nostr label set if the guess is wrong. After this
    /// call, the caller runs [`Self::list_labels`] (free, cached) and
    /// either keeps the candidate wallet on a match or re-derives via
    /// [`Self::get_wallet`] for the right label.
    ///
    /// `extra_salts` accepts caller-supplied salts (e.g. a local-DB
    /// encryption key, a server-auth token) that ride the same
    /// ceremony as the wallet seed. Outputs are returned keyed by name
    /// in [`WalletSetup::extra_seeds`].
    pub async fn setup_wallet(
        &self,
        request: SetupWalletRequest,
    ) -> Result<WalletSetup, PasskeyError> {
        let label = request.label.unwrap_or_else(|| DEFAULT_LABEL.to_string());
        validate_label(&label)?;

        // Compose: [account_master, label, app.<name1>, app.<name2>, ...].
        // The first two are the existing wire-format salts (preserved
        // for backward compat). Caller salts are app-namespaced so
        // they can never collide with future SDK-internal additions.
        let extra_count = request.extra_salts.len();
        let expected = extra_count.saturating_add(2);
        let mut salts = Vec::with_capacity(expected);
        salts.push(ACCOUNT_MASTER_SALT.to_string());
        salts.push(label.clone());
        for s in &request.extra_salts {
            salts.push(format!("{APP_SALT_PREFIX}{}", s.name));
        }

        let seeds = self.prf_provider.derive_prf_seeds(salts).await?;
        if seeds.len() != expected {
            return Err(PasskeyError::PrfError(PasskeyPrfError::Generic(format!(
                "derive_prf_seeds returned {} outputs, expected {expected}",
                seeds.len()
            ))));
        }

        let identity = Identity {
            keys: derive_nostr_keypair(&seeds[0])?,
        };
        let _ = self.identity.set(identity.clone());

        // Build the wallet before reaching out to the label store so a
        // transient publish failure can't burn the PRF ceremony.
        let mnemonic = prf_to_mnemonic(&seeds[1])?;
        let wallet = Wallet {
            seed: Seed::Mnemonic {
                mnemonic,
                passphrase: None,
            },
            label: label.clone(),
        };

        let extra_seeds: HashMap<String, Vec<u8>> = request
            .extra_salts
            .iter()
            .zip(seeds.into_iter().skip(2))
            .map(|(salt, seed)| (salt.name.clone(), seed))
            .collect();

        if request.publish_label
            && let Err(e) = self
                .label_store
                .ensure_label_published(&identity, &label)
                .await
        {
            warn!("setup_wallet: ensure_label_published failed, returning wallet anyway: {e}");
        }

        Ok(WalletSetup {
            wallet,
            extra_seeds,
        })
    }

    /// List labels published for this passkey's identity. Requires
    /// one PRF call to derive the identity (cached after the first
    /// call on this `Passkey` instance).
    pub async fn list_labels(&self) -> Result<Vec<String>, PasskeyError> {
        let identity = self.derive_identity().await?;
        self.label_store.list_labels(&identity).await
    }

    /// Idempotently publish `label` for this passkey's identity.
    /// Requires one PRF call to derive the identity (cached).
    pub async fn store_label(&self, label: String) -> Result<(), PasskeyError> {
        validate_label(&label)?;
        let identity = self.derive_identity().await?;
        self.label_store
            .ensure_label_published(&identity, &label)
            .await
    }

    /// Check if passkey PRF is available on this device.
    ///
    /// Delegates to the platform's `PrfProvider` implementation.
    pub async fn is_available(&self) -> Result<bool, PasskeyError> {
        self.prf_provider
            .is_prf_available()
            .await
            .map_err(PasskeyError::from)
    }
}

/// Convenience constructors that don't cross the `UniFFI` boundary.
/// Bindings that expose `Passkey` via `UniFFI` use [`Passkey::new`]
/// from the exported impl above; native Rust callers can use these
/// to inject a custom [`LabelStore`] or to avoid duplicating
/// `api_key` between `Passkey` and [`crate::connect`].
impl Passkey {
    /// Build a `Passkey` from the SDK [`crate::Config`] the rest of
    /// the app passes to [`crate::connect`]. Reads `config.api_key`
    /// into the default Nostr-backed label store.
    pub fn from_config(prf_provider: Arc<dyn PrfProvider>, config: &crate::Config) -> Self {
        Self::new(
            prf_provider,
            Some(NostrRelayConfig {
                breez_api_key: config.api_key.clone(),
                timeout_secs: None,
            }),
        )
    }

    /// Build a `Passkey` with a caller-supplied [`LabelStore`]. Use
    /// to opt out of the default Nostr-backed sync (server-mediated
    /// store, in-memory tests, etc.). Custom store injection is
    /// Rust-only; `UniFFI` bindings see only [`Passkey::new`].
    pub fn with_label_store(
        prf_provider: Arc<dyn PrfProvider>,
        label_store: Arc<dyn LabelStore>,
    ) -> Self {
        Self {
            prf_provider,
            label_store,
            identity: Arc::new(OnceCell::new()),
        }
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

    // Mock implementation of PrfProvider for testing
    struct MockPrfProvider {
        seed: [u8; 32],
    }

    impl MockPrfProvider {
        fn new(seed: [u8; 32]) -> Self {
            Self { seed }
        }
    }

    #[macros::async_trait]
    impl PrfProvider for MockPrfProvider {
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
    impl PrfProvider for SaltAwareMockProvider {
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
    struct FailingPrfProvider {
        error: PasskeyPrfError,
    }

    impl FailingPrfProvider {
        fn new(error: PasskeyPrfError) -> Self {
            Self { error }
        }
    }

    #[macros::async_trait]
    impl PrfProvider for FailingPrfProvider {
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
    impl PrfProvider for UnavailablePrfProvider {
        async fn derive_prf_seed(&self, _salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
            Err(PasskeyPrfError::PrfNotSupported)
        }

        async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
            Ok(false)
        }
    }

    #[macros::test_all]
    fn test_passkey_new() {
        let prf_provider = Arc::new(MockPrfProvider::new([0u8; 32]));
        let config = NostrRelayConfig::default();

        let _passkey = Passkey::new(prf_provider, Some(config));
    }

    #[macros::async_test_all]
    async fn test_is_available() {
        let prf_provider = Arc::new(MockPrfProvider::new([0u8; 32]));
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
        let prf_provider = Arc::new(FailingPrfProvider::new(
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
        let prf_provider1 = Arc::new(MockPrfProvider::new([42u8; 32]));
        let prf_provider2 = Arc::new(MockPrfProvider::new([42u8; 32]));

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
        let prf_provider1 = Arc::new(MockPrfProvider::new([1u8; 32]));
        let prf_provider2 = Arc::new(MockPrfProvider::new([2u8; 32]));

        let passkey1 = Passkey::new(prf_provider1, None);
        let passkey2 = Passkey::new(prf_provider2, None);

        let mnemonic1 =
            unwrap_mnemonic(passkey1.get_wallet(Some("test".to_string())).await.unwrap());
        let mnemonic2 =
            unwrap_mnemonic(passkey2.get_wallet(Some("test".to_string())).await.unwrap());

        assert_ne!(mnemonic1, mnemonic2);
    }

    #[macros::async_test_all]
    async fn test_get_wallet_produces_12_words() {
        let prf_provider = Arc::new(MockPrfProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        let mnemonic = unwrap_mnemonic(passkey.get_wallet(Some("test".to_string())).await.unwrap());

        let word_count = mnemonic.split_whitespace().count();
        assert_eq!(word_count, 12, "Mnemonic should be 12 words");
    }

    #[macros::async_test_all]
    async fn test_get_wallet_default_label() {
        let prf_provider = Arc::new(MockPrfProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        // None label should default to "Default" and not error
        let wallet = passkey.get_wallet(None).await.unwrap();
        assert_eq!(wallet.label, "Default");
    }

    #[macros::async_test_all]
    async fn test_get_wallet_custom_label() {
        let prf_provider = Arc::new(MockPrfProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        let wallet = passkey
            .get_wallet(Some("personal".to_string()))
            .await
            .unwrap();
        assert_eq!(wallet.label, "personal");
    }

    #[macros::async_test_all]
    async fn test_get_wallet_error_propagation() {
        let prf_provider = Arc::new(FailingPrfProvider::new(PasskeyPrfError::UserCancelled));
        let passkey = Passkey::new(prf_provider, None);

        let result = passkey.get_wallet(Some("test".to_string())).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PasskeyError::PrfError(PasskeyPrfError::UserCancelled)
        ));
    }

    #[macros::async_test_all]
    async fn test_different_labels_produce_different_mnemonics() {
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
            "Different labels should produce different mnemonics"
        );
    }

    #[macros::async_test_all]
    async fn test_same_label_deterministic() {
        let prf_provider = Arc::new(SaltAwareMockProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None);

        let mnemonic1 =
            unwrap_mnemonic(passkey.get_wallet(Some("test".to_string())).await.unwrap());
        let mnemonic2 =
            unwrap_mnemonic(passkey.get_wallet(Some("test".to_string())).await.unwrap());

        assert_eq!(
            mnemonic1, mnemonic2,
            "Same label should produce same mnemonic"
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
    async fn test_derive_identity_deterministic() {
        let prf_provider1 = Arc::new(MockPrfProvider::new([99u8; 32]));
        let prf_provider2 = Arc::new(MockPrfProvider::new([99u8; 32]));

        let passkey1 = Passkey::new(prf_provider1, None);
        let passkey2 = Passkey::new(prf_provider2, None);

        let id1 = passkey1.derive_identity().await.unwrap();
        let id2 = passkey2.derive_identity().await.unwrap();

        assert_eq!(
            id1.public_key_bytes(),
            id2.public_key_bytes(),
            "Same PRF output should produce same identity"
        );
    }

    #[macros::async_test_all]
    async fn test_derive_identity_different_providers() {
        let prf_provider1 = Arc::new(MockPrfProvider::new([1u8; 32]));
        let prf_provider2 = Arc::new(MockPrfProvider::new([2u8; 32]));

        let passkey1 = Passkey::new(prf_provider1, None);
        let passkey2 = Passkey::new(prf_provider2, None);

        let id1 = passkey1.derive_identity().await.unwrap();
        let id2 = passkey2.derive_identity().await.unwrap();

        assert_ne!(
            id1.public_key_bytes(),
            id2.public_key_bytes(),
            "Different PRF outputs should produce different identities"
        );
    }
}
