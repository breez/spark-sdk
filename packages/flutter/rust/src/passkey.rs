use std::sync::Arc;

use anyhow;
use breez_sdk_spark::passkey::{
    NostrRelayConfig, PasskeyPrfError, PasskeyPrfProvider, PasskeyError,
};
use flutter_rust_bridge::{DartFnFuture, frb};

/// Parse a Dart exception message into a typed `PasskeyPrfError`.
///
/// Dart's `PasskeyPrfException.toString()` produces strings like:
///   `"PasskeyPrfException(userCancelled): User dismissed prompt"`
///
/// This function extracts the error code to create the appropriate variant.
fn parse_dart_prf_error(e: anyhow::Error) -> PasskeyPrfError {
    let msg = e.to_string();
    if msg.contains("userCancelled") {
        PasskeyPrfError::UserCancelled
    } else if msg.contains("prfNotSupported") {
        PasskeyPrfError::PrfNotSupported
    } else if msg.contains("noCredential") {
        PasskeyPrfError::CredentialNotFound
    } else if msg.contains("authenticationFailed") {
        PasskeyPrfError::AuthenticationFailed(msg)
    } else {
        PasskeyPrfError::Generic(msg)
    }
}

/// Callback-based implementation of `PasskeyPrfProvider` for Flutter.
///
/// This struct wraps Dart callbacks to implement the PRF provider trait,
/// allowing Flutter to provide the passkey PRF implementation.
///
/// The callbacks return `Result` types so that Dart exceptions propagate
/// cleanly as errors instead of causing panics at the FFI boundary.
struct CallbackPrfProvider {
    derive_prf_seed_fn:
        Arc<dyn Fn(String) -> DartFnFuture<Result<Vec<u8>, anyhow::Error>> + Send + Sync>,
    is_prf_available_fn:
        Arc<dyn Fn() -> DartFnFuture<Result<bool, anyhow::Error>> + Send + Sync>,
}

#[async_trait::async_trait]
impl PasskeyPrfProvider for CallbackPrfProvider {
    async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
        (self.derive_prf_seed_fn)(salt)
            .await
            .map_err(parse_dart_prf_error)
    }

    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
        (self.is_prf_available_fn)()
            .await
            .map_err(parse_dart_prf_error)
    }
}

/// Flutter wrapper for passkey-based wallet operations.
///
/// Orchestrates wallet derivation and label management using
/// passkey PRF callbacks and Nostr relays.
#[derive(Clone)]
#[frb(opaque)]
pub struct Passkey {
    pub(crate) inner: breez_sdk_spark::passkey::Passkey,
}

impl Passkey {
    /// Create a new Passkey instance using Dart callbacks.
    ///
    /// # Arguments
    /// * `derive_prf_seed` - Dart callback to derive a 32-byte seed from passkey PRF with a salt
    /// * `is_prf_available` - Dart callback to check if PRF-capable passkey is available
    /// * `relay_config` - Optional configuration for Nostr relay connections (uses default if None)
    #[frb(sync)]
    pub fn new(
        derive_prf_seed: impl Fn(String) -> DartFnFuture<Result<Vec<u8>, anyhow::Error>>
            + Send
            + Sync
            + 'static,
        is_prf_available: impl Fn() -> DartFnFuture<Result<bool, anyhow::Error>>
            + Send
            + Sync
            + 'static,
        relay_config: Option<NostrRelayConfig>,
    ) -> Self {
        let provider = Arc::new(CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(derive_prf_seed),
            is_prf_available_fn: Arc::new(is_prf_available),
        });

        Self {
            inner: breez_sdk_spark::passkey::Passkey::new(provider, relay_config),
        }
    }

    /// Derive a wallet for a given label.
    ///
    /// Uses the passkey PRF to derive a wallet from the label.
    /// This works for both creating a new wallet and restoring an existing one.
    ///
    /// # Arguments
    /// * `label` - Optional label string (defaults to "Default")
    pub async fn get_wallet(
        &self,
        label: Option<String>,
    ) -> Result<breez_sdk_spark::passkey::Wallet, PasskeyError> {
        self.inner.get_wallet(label).await
    }

    /// List all labels published to Nostr for this passkey's identity.
    ///
    /// Requires 1 PRF call (for Nostr identity derivation).
    pub async fn list_labels(&self) -> Result<Vec<String>, PasskeyError> {
        self.inner.list_labels().await
    }

    /// Publish a label to Nostr relays for this passkey's identity.
    ///
    /// Idempotent: if the label already exists, it is not published again.
    /// Requires 1 PRF call.
    pub async fn store_label(&self, label: String) -> Result<(), PasskeyError> {
        self.inner.store_label(label).await
    }

    /// Check if passkey PRF is available on this device.
    pub async fn is_available(&self) -> Result<bool, PasskeyError> {
        self.inner.is_available().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a successful `DartFnFuture<Result<T, anyhow::Error>>`.
    fn ready_ok<T: Send + 'static>(val: T) -> DartFnFuture<Result<T, anyhow::Error>> {
        Box::pin(std::future::ready(Ok(val)))
    }

    /// Helper to create a failed `DartFnFuture<Result<T, anyhow::Error>>`.
    fn ready_err<T: Send + 'static>(msg: &str) -> DartFnFuture<Result<T, anyhow::Error>> {
        Box::pin(std::future::ready(Err(anyhow::anyhow!("{msg}"))))
    }

    #[tokio::test]
    async fn test_derive_prf_seed_success() {
        let expected = vec![42u8; 32];
        let expected_clone = expected.clone();
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(move |_salt| ready_ok(expected_clone.clone())),
            is_prf_available_fn: Arc::new(|| ready_ok(true)),
        };

        let result = provider.derive_prf_seed("test".to_string()).await;
        assert_eq!(result.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_derive_prf_seed_dart_error_propagated() {
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(|_salt| {
                ready_err("PasskeyPrfException(userCancelled): User dismissed prompt")
            }),
            is_prf_available_fn: Arc::new(|| ready_ok(true)),
        };

        let result = provider.derive_prf_seed("test".to_string()).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, PasskeyPrfError::UserCancelled),
            "Expected UserCancelled, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_derive_prf_seed_prf_not_supported() {
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(|_salt| {
                ready_err("PasskeyPrfException(prfNotSupported): PRF not available")
            }),
            is_prf_available_fn: Arc::new(|| ready_ok(true)),
        };

        let result = provider.derive_prf_seed("test".to_string()).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, PasskeyPrfError::PrfNotSupported),
            "Expected PrfNotSupported, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_derive_prf_seed_no_credential() {
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(|_salt| {
                ready_err("PasskeyPrfException(noCredential): No passkey found")
            }),
            is_prf_available_fn: Arc::new(|| ready_ok(true)),
        };

        let result = provider.derive_prf_seed("test".to_string()).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, PasskeyPrfError::CredentialNotFound),
            "Expected CredentialNotFound, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_derive_prf_seed_generic_error() {
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(|_salt| ready_err("Something unexpected happened")),
            is_prf_available_fn: Arc::new(|| ready_ok(true)),
        };

        let result = provider.derive_prf_seed("test".to_string()).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, PasskeyPrfError::Generic(ref msg) if msg.contains("unexpected")),
            "Expected Generic error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_is_prf_available_success() {
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(|_salt| ready_ok(vec![])),
            is_prf_available_fn: Arc::new(|| ready_ok(false)),
        };

        let result = provider.is_prf_available().await;
        assert_eq!(result.unwrap(), false);
    }

    #[tokio::test]
    async fn test_is_prf_available_error() {
        let provider = CallbackPrfProvider {
            derive_prf_seed_fn: Arc::new(|_salt| ready_ok(vec![])),
            is_prf_available_fn: Arc::new(|| ready_err("Platform check failed")),
        };

        let result = provider.is_prf_available().await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, PasskeyPrfError::Generic(ref msg) if msg.contains("Platform check failed")),
            "Expected Generic error, got: {err:?}"
        );
    }
}
