use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use breez_sdk_spark::passkey::{
    DeriveRequest, DeriveResponse, NostrRelayConfig, PasskeyError, PasskeyPrfError, PrfProvider,
    RegisterRequest, RegisterResponse, RestoreRequest, RestoreResponse, SetupWalletRequest,
    WalletSetup,
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
    derive_prf_seed_fn: Arc<dyn Fn(String) -> DartFnFuture<Vec<u8>> + Send + Sync>,
    is_prf_available_fn: Arc<dyn Fn() -> DartFnFuture<bool> + Send + Sync>,
}

#[async_trait::async_trait]
impl PrfProvider for CallbackPrfProvider {
    async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
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

/// Flutter wrapper for passkey-based wallet operations.
#[derive(Clone)]
#[frb(opaque)]
pub struct Passkey {
    pub(crate) inner: breez_sdk_spark::passkey::Passkey,
}

impl Passkey {
    /// Create a new Passkey instance using Dart callbacks.
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
            inner: breez_sdk_spark::passkey::Passkey::new(provider, relay_config),
        }
    }

    /// Single-prompt setup: derive the wallet seed plus any
    /// caller-supplied extra salts, prime the Nostr identity cache,
    /// and (when `request.publish_label` is true) ensure the label is
    /// published. See [`SetupWalletRequest`] / [`WalletSetup`] for the
    /// full shape.
    pub async fn setup_wallet(
        &self,
        request: SetupWalletRequest,
    ) -> Result<WalletSetup, PasskeyError> {
        self.inner.setup_wallet(request).await
    }

    /// List labels published to Nostr for this passkey's identity.
    /// Requires 1 PRF call (Nostr identity derivation).
    pub async fn list_labels(&self) -> Result<Vec<String>, PasskeyError> {
        self.inner.list_labels().await
    }

    /// Publish a label to Nostr (idempotent). Requires 1 PRF call.
    pub async fn store_label(&self, label: String) -> Result<(), PasskeyError> {
        self.inner.store_label(label).await
    }

    /// True if the device supports passkey PRF.
    pub async fn is_available(&self) -> Result<bool, PasskeyError> {
        self.inner.is_available().await
    }
}

/// High-level orchestrator. See the [`breez_sdk_spark::passkey::PasskeyClient`]
/// docs for the register / restore / derive semantics.
///
/// Currently `register` will fail with
/// [`PasskeyPrfError::PrfNotSupported`] on Flutter because the Dart-side
/// `PrfProvider` callbacks don't yet expose `createPasskey` (blocked on
/// flutter_rust_bridge supporting `Option<impl Fn(...) -> DartFnFuture<...>>`
/// trait callbacks). `derive` and `restore` use only `derivePrfSeed` and
/// work today.
#[derive(Clone)]
#[frb(opaque)]
pub struct PasskeyClient {
    pub(crate) inner: breez_sdk_spark::passkey::PasskeyClient,
}

impl PasskeyClient {
    /// Construct using Dart callbacks for the underlying `PrfProvider`.
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

    /// Speculative cold-restore for returning users without local
    /// state.
    pub async fn restore(
        &self,
        request: RestoreRequest,
    ) -> Result<RestoreResponse, PasskeyError> {
        self.inner.restore(request).await
    }

    /// Returning user with the correct label cached locally.
    pub async fn derive(&self, request: DeriveRequest) -> Result<DeriveResponse, PasskeyError> {
        self.inner.derive(request).await
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
            derive_prf_seed_fn: Arc::new(derive),
            is_prf_available_fn: Arc::new(is_available),
        }
    }

    #[tokio::test]
    async fn test_derive_prf_seed_success() {
        let expected = vec![42u8; 32];
        let expected_clone = expected.clone();
        let provider = make_provider(
            move |_salt| ready(expected_clone.clone()),
            || ready(true),
        );
        let result = provider.derive_prf_seed("test".to_string()).await;
        assert_eq!(result.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_derive_prf_seed_panic_caught() {
        let provider = make_provider(
            |_salt| panicking("Dart threw an exception"),
            || ready(true),
        );
        let err = provider.derive_prf_seed("test".to_string()).await.unwrap_err();
        assert!(
            matches!(err, PasskeyPrfError::Generic(ref msg) if msg.contains("Dart threw an exception")),
            "Expected Generic error with panic message, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_is_prf_available_success() {
        let provider = make_provider(|_salt| ready(vec![]), || ready(false));
        assert!(!provider.is_prf_available().await.unwrap());
    }

    #[tokio::test]
    async fn test_is_prf_available_panic_caught() {
        let provider = make_provider(
            |_salt| ready(vec![]),
            || panicking("device check failed"),
        );
        let err = provider.is_prf_available().await.unwrap_err();
        assert!(
            matches!(err, PasskeyPrfError::Generic(ref msg) if msg.contains("device check failed")),
            "Expected Generic error with panic message, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_derive_prf_seeds_falls_back_to_loop() {
        let provider = make_provider(
            move |salt| ready(format!("seed:{salt}").into_bytes()),
            || ready(true),
        );
        let seeds = provider
            .derive_prf_seeds(vec!["a".into(), "b".into()])
            .await
            .unwrap();
        assert_eq!(seeds, vec![b"seed:a".to_vec(), b"seed:b".to_vec()]);
    }
}
