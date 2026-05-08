//! User-facing [`SessionManager`] surface for the Breez SDK.
//!
//! UniFFI-generated bindings can only export traits defined inside the crate
//! they're generated from, so we re-declare the trait + supporting types here
//! and bridge to [`spark_wallet`] via an internal adapter when the SDK is
//! built. Integrators implement *this* trait — typically backed by a shared
//! database — to let multiple SDK pods share authentication state.

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use thiserror::Error;

#[cfg(feature = "uniffi")]
uniffi::custom_type!(PublicKey, String, {
    remote,
    try_lift: |val| {
        use std::str::FromStr;
        PublicKey::from_str(&val).map_err(uniffi::deps::anyhow::Error::msg)
    },
    lower: |obj| obj.to_string(),
});

#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum SessionManagerError {
    #[error("Session not found")]
    NotFound,
    #[error("Generic error: {0}")]
    Generic(String),
}

impl From<spark_wallet::SessionManagerError> for SessionManagerError {
    fn from(e: spark_wallet::SessionManagerError) -> Self {
        match e {
            spark_wallet::SessionManagerError::NotFound => SessionManagerError::NotFound,
            spark_wallet::SessionManagerError::Generic(msg) => SessionManagerError::Generic(msg),
        }
    }
}

impl From<SessionManagerError> for spark_wallet::SessionManagerError {
    fn from(e: SessionManagerError) -> Self {
        match e {
            SessionManagerError::NotFound => spark_wallet::SessionManagerError::NotFound,
            SessionManagerError::Generic(msg) => spark_wallet::SessionManagerError::Generic(msg),
        }
    }
}

/// Cached authentication session for a single backend service identity.
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Session {
    pub token: String,
    pub expiration: u64,
}

impl From<spark_wallet::Session> for Session {
    fn from(s: spark_wallet::Session) -> Self {
        Self {
            token: s.token,
            expiration: s.expiration,
        }
    }
}

impl From<Session> for spark_wallet::Session {
    fn from(s: Session) -> Self {
        Self {
            token: s.token,
            expiration: s.expiration,
        }
    }
}

/// Persistent storage for authentication sessions, keyed by the service's
/// identity public key. Implementations should be thread-safe and may be
/// backed by an in-memory map (default) or a shared database for cross-pod
/// auth sharing.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait SessionManager: Send + Sync {
    async fn get_session(
        &self,
        service_identity_key: PublicKey,
    ) -> Result<Session, SessionManagerError>;

    async fn set_session(
        &self,
        service_identity_key: PublicKey,
        session: Session,
    ) -> Result<(), SessionManagerError>;
}

/// Internal adapter that exposes a user-supplied [`SessionManager`] to
/// [`spark_wallet`] (which has its own identical-shape trait).
///
/// When no session manager is provided, the SDK uses
/// [`spark_wallet::InMemorySessionManager`] directly without going through
/// this adapter — there's no point round-tripping in-memory state through a
/// wrapper trait.
pub(crate) struct SessionManagerAdapter(pub Arc<dyn SessionManager>);

#[macros::async_trait]
impl spark_wallet::SessionManager for SessionManagerAdapter {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<spark_wallet::Session, spark_wallet::SessionManagerError> {
        self.0
            .get_session(*service_identity_key)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: spark_wallet::Session,
    ) -> Result<(), spark_wallet::SessionManagerError> {
        self.0
            .set_session(*service_identity_key, session.into())
            .await
            .map_err(Into::into)
    }
}
