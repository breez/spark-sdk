use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use breez_sdk_spark::passkey::{
    ConnectWithPasskeyRequest, ConnectWithPasskeyResponse, DeriveSeedsRequest, PasskeyAvailability,
    PasskeyConfig, PasskeyError, PrfProvider, PrfProviderError, RegisterRequest, RegisterResponse,
    RegisteredCredential, SignInRequest, SignInResponse,
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

/// Wraps Dart callbacks as a [`PrfProvider`] implementation. Each
/// callback returns a Result so Dart-side throws (e.g.
/// `PasskeyPrfException`) propagate cleanly. The known-credential
/// callbacks back `PasskeyClient.credentials()`; hosts without a
/// registry can have them resolve to empty / no-op on the Dart side.
struct CallbackPrfProvider {
    /// Bulk PRF callback. Single OS ceremony for N salts on platforms
    /// that support the WebAuthn dual-salt fast path (saltInput1 +
    /// saltInput2 on iOS, prfFirst + prfSecond on Android); the Dart
    /// side internally falls back to looping per-salt where the
    /// platform doesn't expose the fast path.
    derive_seeds_fn:
        Arc<dyn Fn(DeriveSeedsRequest) -> DartFnFuture<anyhow::Result<Vec<Vec<u8>>>> + Send + Sync>,
    is_supported_fn: Arc<dyn Fn() -> DartFnFuture<anyhow::Result<bool>> + Send + Sync>,
    create_passkey_fn: Arc<
        dyn Fn(Vec<Vec<u8>>) -> DartFnFuture<anyhow::Result<RegisteredCredential>> + Send + Sync,
    >,
    get_known_credential_ids_fn:
        Arc<dyn Fn() -> DartFnFuture<anyhow::Result<Vec<Vec<u8>>>> + Send + Sync>,
    remove_known_credential_id_fn:
        Arc<dyn Fn(Vec<u8>) -> DartFnFuture<anyhow::Result<()>> + Send + Sync>,
    clear_known_credential_ids_fn: Arc<dyn Fn() -> DartFnFuture<anyhow::Result<()>> + Send + Sync>,
}

/// Convert a Dart-thrown error into a [`PrfProviderError`]. The Dart
/// side raises `PasskeyPrfException` with a structured `code` field
/// embedded in the message; we substring-match it back to the typed
/// variant so callers can pattern-match instead of parsing strings.
fn dart_error_to_prf(err: anyhow::Error) -> PrfProviderError {
    let msg = format!("{err}");
    let lower = msg.to_lowercase();
    if lower.contains("usercancelled") {
        PrfProviderError::UserCancelled
    } else if lower.contains("nocredential") {
        PrfProviderError::CredentialNotFound(msg)
    } else if lower.contains("prfnotsupported") {
        PrfProviderError::PrfNotSupported
    } else if lower.contains("credentialalreadyexists") {
        PrfProviderError::CredentialAlreadyExists(msg)
    } else if lower.contains("configuration") {
        PrfProviderError::Configuration(msg)
    } else {
        PrfProviderError::Generic(msg)
    }
}

#[async_trait::async_trait]
impl PrfProvider for CallbackPrfProvider {
    async fn derive_seeds(
        &self,
        request: DeriveSeedsRequest,
    ) -> Result<Vec<Vec<u8>>, PrfProviderError> {
        let bulk = AssertUnwindSafe((self.derive_seeds_fn)(request))
            .catch_unwind()
            .await
            .map_err(|e| PrfProviderError::Generic(panic_message(e)))?;
        bulk.map_err(dart_error_to_prf)
    }

    async fn is_supported(&self) -> Result<bool, PrfProviderError> {
        let result = AssertUnwindSafe((self.is_supported_fn)())
            .catch_unwind()
            .await
            .map_err(|e| PrfProviderError::Generic(panic_message(e)))?;
        result.map_err(dart_error_to_prf)
    }

    async fn create_passkey(
        &self,
        exclude_credential_ids: Vec<Vec<u8>>,
    ) -> Result<RegisteredCredential, PrfProviderError> {
        let result = AssertUnwindSafe((self.create_passkey_fn)(exclude_credential_ids))
            .catch_unwind()
            .await
            .map_err(|e| PrfProviderError::Generic(panic_message(e)))?;
        result.map_err(dart_error_to_prf)
    }

    async fn get_known_credential_ids(&self) -> Result<Vec<Vec<u8>>, PrfProviderError> {
        let result = AssertUnwindSafe((self.get_known_credential_ids_fn)())
            .catch_unwind()
            .await
            .map_err(|e| PrfProviderError::Generic(panic_message(e)))?;
        result.map_err(dart_error_to_prf)
    }

    async fn remove_known_credential_id(&self, id: Vec<u8>) -> Result<(), PrfProviderError> {
        let result = AssertUnwindSafe((self.remove_known_credential_id_fn)(id))
            .catch_unwind()
            .await
            .map_err(|e| PrfProviderError::Generic(panic_message(e)))?;
        result.map_err(dart_error_to_prf)
    }

    async fn clear_known_credential_ids(&self) -> Result<(), PrfProviderError> {
        let result = AssertUnwindSafe((self.clear_known_credential_ids_fn)())
            .catch_unwind()
            .await
            .map_err(|e| PrfProviderError::Generic(panic_message(e)))?;
        result.map_err(dart_error_to_prf)
    }
}

/// High-level orchestrator. See the [`breez_sdk_spark::passkey::PasskeyClient`]
/// docs for the register / sign_in semantics.
#[derive(Clone)]
#[frb(opaque)]
pub struct PasskeyClient {
    pub(crate) inner: breez_sdk_spark::passkey::PasskeyClient,
}

impl PasskeyClient {
    /// Construct using Dart callbacks for the underlying `PrfProvider`.
    /// Hosts that don't drive registration can have `create_passkey`
    /// throw `PrfProviderError.PrfNotSupported` on the Dart side. The
    /// three known-credential callbacks back `PasskeyClient.credentials()`;
    /// hosts without a registry can return empty / no-op.
    #[frb(sync)]
    pub fn new(
        derive_seeds: impl Fn(DeriveSeedsRequest) -> DartFnFuture<anyhow::Result<Vec<Vec<u8>>>>
        + Send
        + Sync
        + 'static,
        is_supported: impl Fn() -> DartFnFuture<anyhow::Result<bool>> + Send + Sync + 'static,
        create_passkey: impl Fn(Vec<Vec<u8>>) -> DartFnFuture<anyhow::Result<RegisteredCredential>>
        + Send
        + Sync
        + 'static,
        get_known_credential_ids: impl Fn() -> DartFnFuture<anyhow::Result<Vec<Vec<u8>>>>
        + Send
        + Sync
        + 'static,
        remove_known_credential_id: impl Fn(Vec<u8>) -> DartFnFuture<anyhow::Result<()>>
        + Send
        + Sync
        + 'static,
        clear_known_credential_ids: impl Fn() -> DartFnFuture<anyhow::Result<()>>
        + Send
        + Sync
        + 'static,
        breez_api_key: Option<String>,
        config: Option<PasskeyConfig>,
    ) -> Self {
        let provider = Arc::new(CallbackPrfProvider {
            derive_seeds_fn: Arc::new(derive_seeds),
            is_supported_fn: Arc::new(is_supported),
            create_passkey_fn: Arc::new(create_passkey),
            get_known_credential_ids_fn: Arc::new(get_known_credential_ids),
            remove_known_credential_id_fn: Arc::new(remove_known_credential_id),
            clear_known_credential_ids_fn: Arc::new(clear_known_credential_ids),
        });
        Self {
            inner: breez_sdk_spark::passkey::PasskeyClient::new(provider, breez_api_key, config),
        }
    }

    /// One-shot capability + configuration probe.
    pub async fn check_availability(&self) -> Result<PasskeyAvailability, PasskeyError> {
        self.inner.check_availability().await
    }

    /// First-time setup: drives the Dart-side `create_passkey` callback
    /// then derives the wallet seed.
    pub async fn register(
        &self,
        request: RegisterRequest,
    ) -> Result<RegisterResponse, PasskeyError> {
        self.inner.register(request).await
    }

    /// Returning-user sign-in. Fast path with `label` set; cold-restore
    /// with discovery when `label` is `None`.
    pub async fn sign_in(&self, request: SignInRequest) -> Result<SignInResponse, PasskeyError> {
        self.inner.sign_in(request).await
    }

    /// Single-CTA onboarding: silent sign-in, fall through to register
    /// on `CredentialNotFound`. Mobile-only (iOS 18+ / Android 9+);
    /// see the core SDK docs for the cross-browser limitation.
    pub async fn connect_with_passkey(
        &self,
        request: ConnectWithPasskeyRequest,
    ) -> Result<ConnectWithPasskeyResponse, PasskeyError> {
        self.inner.connect_with_passkey(request).await
    }

    /// Label sub-object: list / publish labels for this passkey's identity.
    #[frb(sync)]
    pub fn labels(&self) -> PasskeyLabels {
        PasskeyLabels {
            inner: self.inner.labels(),
        }
    }

    /// Credential sub-object: inspect / mutate the provider's persisted
    /// credential-ID set.
    #[frb(sync)]
    pub fn credentials(&self) -> PasskeyCredentials {
        PasskeyCredentials {
            inner: self.inner.credentials(),
        }
    }
}

/// Label sub-object surfaced from [`PasskeyClient::labels`].
#[derive(Clone)]
#[frb(opaque)]
pub struct PasskeyLabels {
    pub(crate) inner: Arc<breez_sdk_spark::passkey::PasskeyLabels>,
}

impl PasskeyLabels {
    /// List labels published for this passkey's identity.
    pub async fn list(&self) -> Result<Vec<String>, PasskeyError> {
        self.inner.list().await
    }

    /// Idempotently publish `label`.
    pub async fn store(&self, label: String) -> Result<(), PasskeyError> {
        self.inner.store(label).await
    }
}

/// Credential sub-object surfaced from [`PasskeyClient::credentials`].
#[derive(Clone)]
#[frb(opaque)]
pub struct PasskeyCredentials {
    pub(crate) inner: Arc<breez_sdk_spark::passkey::PasskeyCredentials>,
}

impl PasskeyCredentials {
    /// Read the persisted set of credential IDs for the current RP.
    pub async fn get(&self) -> Result<Vec<Vec<u8>>, PasskeyError> {
        self.inner.get().await
    }

    /// Drop a single credential ID from the persisted set.
    pub async fn remove(&self, credential_id: Vec<u8>) -> Result<(), PasskeyError> {
        self.inner.remove(credential_id).await
    }

    /// Clear the persisted credential-ID set for the current RP.
    pub async fn clear(&self) -> Result<(), PasskeyError> {
        self.inner.clear().await
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
        derive_bulk: impl Fn(DeriveSeedsRequest) -> DartFnFuture<anyhow::Result<Vec<Vec<u8>>>>
        + Send
        + Sync
        + 'static,
        is_available: impl Fn() -> DartFnFuture<anyhow::Result<bool>> + Send + Sync + 'static,
    ) -> CallbackPrfProvider {
        CallbackPrfProvider {
            derive_seeds_fn: Arc::new(derive_bulk),
            is_supported_fn: Arc::new(is_available),
            create_passkey_fn: Arc::new(|_req| {
                panicking::<anyhow::Result<RegisteredCredential>>("create_passkey not used")
            }),
            get_known_credential_ids_fn: Arc::new(|| ready(Ok(vec![]))),
            remove_known_credential_id_fn: Arc::new(|_id| ready(Ok(()))),
            clear_known_credential_ids_fn: Arc::new(|| ready(Ok(()))),
        }
    }

    fn req(salts: &[&str]) -> DeriveSeedsRequest {
        DeriveSeedsRequest {
            salts: salts.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_derive_seeds_success() {
        let expected = vec![42u8; 32];
        let expected_clone = expected.clone();
        let provider = make_provider(
            move |_request| ready(Ok(vec![expected_clone.clone()])),
            || ready(Ok(true)),
        );
        let seeds = provider.derive_seeds(req(&["test"])).await.unwrap();
        assert_eq!(seeds, vec![expected]);
    }

    #[tokio::test]
    async fn test_derive_seeds_panic_caught() {
        let provider = make_provider(
            |_request| panicking("Dart threw an exception"),
            || ready(Ok(true)),
        );
        let err = provider.derive_seeds(req(&["test"])).await.unwrap_err();
        assert!(
            matches!(err, PrfProviderError::Generic(ref msg) if msg.contains("Dart threw an exception")),
            "Expected Generic error with panic message, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_derive_seeds_dart_error_mapped() {
        let provider = make_provider(
            |_request| {
                ready(Err(anyhow::anyhow!(
                    "PasskeyPrfException(noCredential): not found"
                )))
            },
            || ready(Ok(true)),
        );
        let err = provider.derive_seeds(req(&["test"])).await.unwrap_err();
        assert!(
            matches!(err, PrfProviderError::CredentialNotFound(_)),
            "Expected CredentialNotFound, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_is_supported_success() {
        let provider = make_provider(|_request| ready(Ok(vec![])), || ready(Ok(false)));
        assert!(!provider.is_supported().await.unwrap());
    }

    #[tokio::test]
    async fn test_is_supported_panic_caught() {
        let provider = make_provider(
            |_request| ready(Ok(vec![])),
            || panicking("device check failed"),
        );
        let err = provider.is_supported().await.unwrap_err();
        assert!(
            matches!(err, PrfProviderError::Generic(ref msg) if msg.contains("device check failed")),
            "Expected Generic error with panic message, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_derive_seeds_propagates_non_prf_errors() {
        let provider = make_provider(
            move |_request| {
                ready(Err(anyhow::anyhow!(
                    "PasskeyPrfException(userCancelled): user dismissed"
                )))
            },
            || ready(Ok(true)),
        );
        let err = provider.derive_seeds(req(&["a", "b"])).await.unwrap_err();
        assert!(
            matches!(err, PrfProviderError::UserCancelled),
            "expected UserCancelled, got {err:?}"
        );
    }
}
