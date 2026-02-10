use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use breez_sdk_spark::seedless_restore::{
    NostrRelayConfig, PasskeyPrfError, PasskeyPrfProvider, SeedlessRestoreError,
};
use breez_sdk_spark::Seed;
use flutter_rust_bridge::{DartFnFuture, frb};
use futures::FutureExt;

/// Extract a human-readable message from a panic payload.
fn panic_message(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "Dart callback panicked".to_string())
}

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
        // DartFnFuture panics if the Dart callback throws. Catch the panic here
        // so it doesn't unwind through the core SDK.
        AssertUnwindSafe((self.derive_prf_seed_fn)(salt))
            .catch_unwind()
            .await
            .map_err(|e| PasskeyPrfError::Generic(panic_message(e)))
    }

    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
        AssertUnwindSafe((self.is_prf_available_fn)())
            .catch_unwind()
            .await
            .map_err(|e| PasskeyPrfError::Generic(panic_message(e)))
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
    /// * `relay_config` - Optional configuration for Nostr relay connections (uses default if None)
    #[frb(sync)]
    pub fn new(
        derive_prf_seed: impl Fn(String) -> DartFnFuture<Vec<u8>> + Send + Sync + 'static,
        is_prf_available: impl Fn() -> DartFnFuture<bool> + Send + Sync + 'static,
        relay_config: Option<NostrRelayConfig>,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a `DartFnFuture<T>` from a value.
    fn ready<T: Send + 'static>(val: T) -> DartFnFuture<T> {
        Box::pin(std::future::ready(val))
    }

    /// Helper to create a `DartFnFuture<T>` that panics with the given message.
    fn panicking<T: Send + 'static>(msg: &'static str) -> DartFnFuture<T> {
        Box::pin(async move { panic!("{msg}") })
    }

    #[tokio::test]
    async fn test_derive_prf_seed_success() {
        let expected = vec![42u8; 32];
        let expected_clone = expected.clone();
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(move |_salt| ready(expected_clone.clone())),
            is_prf_available_fn: Arc::new(|| ready(true)),
        };

        let result = provider.derive_prf_seed("test".to_string()).await;
        assert_eq!(result.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_derive_prf_seed_panic_caught() {
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(|_salt| panicking("Dart threw an exception")),
            is_prf_available_fn: Arc::new(|| ready(true)),
        };

        let result = provider.derive_prf_seed("test".to_string()).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, PasskeyPrfError::Generic(ref msg) if msg.contains("Dart threw an exception")),
            "Expected Generic error with panic message, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_is_prf_available_success() {
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(|_salt| ready(vec![])),
            is_prf_available_fn: Arc::new(|| ready(false)),
        };

        let result = provider.is_prf_available().await;
        assert_eq!(result.unwrap(), false);
    }

    #[tokio::test]
    async fn test_is_prf_available_panic_caught() {
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(|_salt| ready(vec![])),
            is_prf_available_fn: Arc::new(|| panicking("device check failed")),
        };

        let result = provider.is_prf_available().await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, PasskeyPrfError::Generic(ref msg) if msg.contains("device check failed")),
            "Expected Generic error with panic message, got: {err:?}"
        );
    }
}
