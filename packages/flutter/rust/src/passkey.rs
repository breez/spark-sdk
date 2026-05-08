use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use breez_sdk_spark::passkey::{
    NostrRelayConfig, PasskeyError, PrfProvider, PrfProviderError, RegisterRequest,
    RegisterResponse, SignInRequest, SignInResponse,
};
use flutter_rust_bridge::{DartFnFuture, frb};
use futures::FutureExt;

/// Extract a human-readable message from a panic payload.
fn panic_message(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "Dart callback panicked".to_string())
}

/// Wraps Dart callbacks as a [`PrfProvider`] implementation. Only the
/// two required trait methods are exposed to Dart; bulk PRF and domain
/// association use the trait defaults (looped derive / `Skipped`).
/// Bulk PRF override on Flutter is blocked on flutter_rust_bridge
/// supporting `Option<impl Fn(...) -> DartFnFuture<...>>` parameters.
struct CallbackPrfProvider {
    derive_seed_fn: Arc<dyn Fn(String) -> DartFnFuture<Vec<u8>> + Send + Sync>,
    is_supported_fn: Arc<dyn Fn() -> DartFnFuture<bool> + Send + Sync>,
}

#[async_trait::async_trait]
impl PrfProvider for CallbackPrfProvider {
    async fn derive_seed(&self, salt: String) -> Result<Vec<u8>, PrfProviderError> {
        AssertUnwindSafe((self.derive_seed_fn)(salt))
            .catch_unwind()
            .await
            .map_err(|e| PrfProviderError::Generic(panic_message(e)))
    }

    async fn is_supported(&self) -> Result<bool, PrfProviderError> {
        AssertUnwindSafe((self.is_supported_fn)())
            .catch_unwind()
            .await
            .map_err(|e| PrfProviderError::Generic(panic_message(e)))
    }
}

/// High-level orchestrator. See the [`breez_sdk_spark::passkey::PasskeyClient`]
/// docs for the register / sign_in semantics.
///
/// Currently `register` will fail with
/// [`PrfProviderError::PrfNotSupported`] on Flutter because the Dart-side
/// `PrfProvider` callbacks don't yet expose `createPasskey` (blocked on
/// flutter_rust_bridge supporting `Option<impl Fn(...) -> DartFnFuture<...>>`
/// trait callbacks). `sign_in` uses only `derive_seed` and works today.
#[derive(Clone)]
#[frb(opaque)]
pub struct PasskeyClient {
    pub(crate) inner: breez_sdk_spark::passkey::PasskeyClient,
}

impl PasskeyClient {
    /// Construct using Dart callbacks for the underlying `PrfProvider`.
    #[frb(sync)]
    pub fn new(
        derive_seed: impl Fn(String) -> DartFnFuture<Vec<u8>> + Send + Sync + 'static,
        is_supported: impl Fn() -> DartFnFuture<bool> + Send + Sync + 'static,
        relay_config: Option<NostrRelayConfig>,
    ) -> Self {
        let provider = Arc::new(CallbackPrfProvider {
            derive_seed_fn: Arc::new(derive_seed),
            is_supported_fn: Arc::new(is_supported),
        });
        Self {
            inner: breez_sdk_spark::passkey::PasskeyClient::new(provider, relay_config),
        }
    }

    /// First-time setup. Currently returns `PrfNotSupported` on Flutter.
    pub async fn register(
        &self,
        request: RegisterRequest,
    ) -> Result<RegisterResponse, PasskeyError> {
        self.inner.register(request).await
    }

    /// Returning-user sign-in. Fast path with `label` set; cold-restore
    /// with discovery when `label` is `None`.
    pub async fn sign_in(
        &self,
        request: SignInRequest,
    ) -> Result<SignInResponse, PasskeyError> {
        self.inner.sign_in(request).await
    }

    /// List labels published for this passkey's identity.
    pub async fn list_labels(&self) -> Result<Vec<String>, PasskeyError> {
        self.inner.list_labels().await
    }

    /// Idempotently publish `label`.
    pub async fn store_label(&self, label: String) -> Result<(), PasskeyError> {
        self.inner.store_label(label).await
    }

    /// True if the device supports passkey PRF.
    pub async fn is_available(&self) -> Result<bool, PasskeyError> {
        self.inner.is_available().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready<T: Send + 'static>(val: T) -> DartFnFuture<T> {
        Box::pin(std::future::ready(val))
    }

    fn panicking<T: Send + 'static>(msg: &'static str) -> DartFnFuture<T> {
        Box::pin(async move { panic!("{msg}") })
    }

    fn make_provider(
        derive: impl Fn(String) -> DartFnFuture<Vec<u8>> + Send + Sync + 'static,
        is_available: impl Fn() -> DartFnFuture<bool> + Send + Sync + 'static,
    ) -> CallbackPrfProvider {
        CallbackPrfProvider {
            derive_seed_fn: Arc::new(derive),
            is_supported_fn: Arc::new(is_available),
        }
    }

    #[tokio::test]
    async fn test_derive_seed_success() {
        let expected = vec![42u8; 32];
        let expected_clone = expected.clone();
        let provider = make_provider(
            move |_salt| ready(expected_clone.clone()),
            || ready(true),
        );
        let result = provider.derive_seed("test".to_string()).await;
        assert_eq!(result.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_derive_seed_panic_caught() {
        let provider = make_provider(
            |_salt| panicking("Dart threw an exception"),
            || ready(true),
        );
        let err = provider.derive_seed("test".to_string()).await.unwrap_err();
        assert!(
            matches!(err, PrfProviderError::Generic(ref msg) if msg.contains("Dart threw an exception")),
            "Expected Generic error with panic message, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_is_supported_success() {
        let provider = make_provider(|_salt| ready(vec![]), || ready(false));
        assert!(!provider.is_supported().await.unwrap());
    }

    #[tokio::test]
    async fn test_is_supported_panic_caught() {
        let provider = make_provider(
            |_salt| ready(vec![]),
            || panicking("device check failed"),
        );
        let err = provider.is_supported().await.unwrap_err();
        assert!(
            matches!(err, PrfProviderError::Generic(ref msg) if msg.contains("device check failed")),
            "Expected Generic error with panic message, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_derive_seeds_falls_back_to_loop() {
        let provider = make_provider(
            move |salt| ready(format!("seed:{salt}").into_bytes()),
            || ready(true),
        );
        let seeds = provider
            .derive_seeds(vec!["a".into(), "b".into()])
            .await
            .unwrap();
        assert_eq!(seeds, vec![b"seed:a".to_vec(), b"seed:b".to_vec()]);
    }
}
