//! Dart-callback adapter for the Breez SDK [`SessionManager`] trait.
//!
//! Lets Flutter integrators provide a `SessionManager` implementation backed
//! by Dart functions (e.g. one that talks to a shared database via a Dart
//! plugin). Mirrors the `passkey::CallbackPrfProvider` pattern.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use breez_sdk_spark::{PublicKey, Session, SessionManager, SessionManagerError};
use flutter_rust_bridge::DartFnFuture;
use futures::FutureExt;

/// Extract a human-readable message from a panic payload.
fn panic_message(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "Dart callback panicked".to_string())
}

/// Callback-based [`SessionManager`] for Flutter.
///
/// Wraps two Dart callbacks — one to read a session, one to write it — and
/// exposes them as a `SessionManager` implementation for the SDK builder. The
/// Dart callbacks see the service identity public key as a hex string.
///
/// `getSession` returns `None` when no cached session exists (mapped to
/// `SessionManagerError::NotFound`). Any panic thrown from the Dart side is
/// caught and surfaced as `SessionManagerError::Generic`.
pub(crate) struct CallbackSessionManager {
    pub(crate) get_session_fn:
        Arc<dyn Fn(String) -> DartFnFuture<Option<Session>> + Send + Sync>,
    pub(crate) set_session_fn: Arc<dyn Fn(String, Session) -> DartFnFuture<()> + Send + Sync>,
}

#[async_trait::async_trait]
impl SessionManager for CallbackSessionManager {
    async fn get_session(
        &self,
        service_identity_key: PublicKey,
    ) -> Result<Session, SessionManagerError> {
        let pk = service_identity_key.to_string();
        let result = AssertUnwindSafe((self.get_session_fn)(pk))
            .catch_unwind()
            .await
            .map_err(|e| SessionManagerError::Generic(panic_message(e)))?;
        result.ok_or(SessionManagerError::NotFound)
    }

    async fn set_session(
        &self,
        service_identity_key: PublicKey,
        session: Session,
    ) -> Result<(), SessionManagerError> {
        let pk = service_identity_key.to_string();
        AssertUnwindSafe((self.set_session_fn)(pk, session))
            .catch_unwind()
            .await
            .map_err(|e| SessionManagerError::Generic(panic_message(e)))
    }
}

