//! Flutter bindings for seedless wallet restore.
//!
//! This module provides Flutter/Dart bindings for the seedless restore functionality
//! using `flutter_rust_bridge`. Instead of passing a trait object across FFI, this
//! implementation uses Dart callbacks for the PRF operations.

use std::sync::Arc;

use breez_sdk_spark::seedless_restore::{
    NostrRelayConfig, PasskeyPrfError, PasskeyPrfProvider, SeedlessRestoreError,
};
use breez_sdk_spark::Seed;
use flutter_rust_bridge::{DartFnFuture, frb};

/// Callback-based implementation of `PasskeyPrfProvider` for Flutter.
///
/// This struct wraps Dart callbacks to implement the PRF provider trait,
/// allowing Flutter to provide the passkey PRF implementation.
struct CallbackPrfProvider {
    derive_prf_seed_fn: Arc<dyn Fn(String) -> DartFnFuture<Vec<u8>> + Send + Sync>,
    is_prf_available_fn: Arc<dyn Fn() -> DartFnFuture<bool> + Send + Sync>,
}

#[async_trait::async_trait]
impl PasskeyPrfProvider for CallbackPrfProvider {
    async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
        // DartFnFuture returns the value directly (Dart throws on error)
        Ok((self.derive_prf_seed_fn)(salt).await)
    }

    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
        Ok((self.is_prf_available_fn)().await)
    }
}

/// Flutter wrapper for SeedlessRestore.
///
/// Orchestrates seedless wallet creation and restore operations using
/// passkey PRF callbacks and Nostr relays.
#[frb(opaque)]
pub struct SeedlessRestore {
    inner: breez_sdk_spark::seedless_restore::SeedlessRestore,
}

impl SeedlessRestore {
    /// Create a new SeedlessRestore instance using Dart callbacks.
    ///
    /// # Arguments
    /// * `derive_prf_seed` - Dart callback to derive a 32-byte seed from passkey PRF with a salt
    /// * `is_prf_available` - Dart callback to check if PRF-capable passkey is available
    /// * `relay_config` - Configuration for Nostr relay connections
    pub fn new(
        derive_prf_seed: impl Fn(String) -> DartFnFuture<Vec<u8>> + Send + Sync + 'static,
        is_prf_available: impl Fn() -> DartFnFuture<bool> + Send + Sync + 'static,
        relay_config: NostrRelayConfig,
    ) -> Self {
        let provider = Arc::new(CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(derive_prf_seed),
            is_prf_available_fn: Arc::new(is_prf_available),
        });

        Self {
            inner: breez_sdk_spark::seedless_restore::SeedlessRestore::new(provider, relay_config),
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
    /// The derived wallet seed (24-word mnemonic)
    pub async fn create_seed(&self, salt: String) -> Result<Seed, SeedlessRestoreError> {
        self.inner.create_seed(salt).await
    }

    /// List all salts published to Nostr for this passkey's identity.
    ///
    /// This method queries Nostr relays for all kind-1 text note events
    /// authored by the Nostr identity derived from the passkey.
    ///
    /// # Returns
    /// A list of salt strings found
    pub async fn list_salts(&self) -> Result<Vec<String>, SeedlessRestoreError> {
        self.inner.list_salts().await
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
    /// The derived wallet seed (24-word mnemonic)
    pub async fn restore_seed(&self, salt: String) -> Result<Seed, SeedlessRestoreError> {
        self.inner.restore_seed(salt).await
    }

    /// Check if passkey PRF is available on this device.
    ///
    /// # Returns
    /// `true` if PRF-capable passkey is available
    pub async fn is_prf_available(&self) -> Result<bool, SeedlessRestoreError> {
        self.inner.is_prf_available().await
    }
}

/// Create a default NostrRelayConfig with public relays.
#[frb(sync)]
pub fn default_nostr_relay_config() -> NostrRelayConfig {
    NostrRelayConfig::default()
}

/// Create a NostrRelayConfig with Breez-operated relays.
#[frb(sync)]
pub fn breez_nostr_relay_config() -> NostrRelayConfig {
    NostrRelayConfig::breez_relays()
}

/// Create a custom NostrRelayConfig.
#[frb(sync)]
pub fn custom_nostr_relay_config(relay_urls: Vec<String>, timeout_secs: u32) -> NostrRelayConfig {
    NostrRelayConfig::custom(relay_urls, timeout_secs)
}
