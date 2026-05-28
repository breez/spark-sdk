//! User-facing [`SessionStore`] surface for the Breez SDK.
//!
//! UniFFI-generated bindings can only export traits defined inside the crate
//! they're generated from, so we re-declare the trait + supporting types
//! here. The DB-backed implementations (`PostgresSessionStore`,
//! `MysqlSessionStore`) implement `spark_wallet::SessionStore` directly
//! and are picked up by `SdkBuilder::build()` when a corresponding pool is
//! configured on the `SdkContext`.
//!
//! Internal layering applied automatically by `SdkBuilder::build()`:
//!
//! ```text
//! auth providers (SO / SSP)
//!     │ plaintext
//!     ▼
//! CachingSessionStore   ← in-memory hot path
//!     │ plaintext
//!     ▼
//! EncryptingSessionStore ← ECIES on Session::token
//!     │ ciphertext (base64)
//!     ▼
//! PostgresSessionStore | MysqlSessionStore | InMemorySessionStore
//! ```

mod adapter;
mod caching;
mod encrypting;

use bitcoin::secp256k1::PublicKey;
use thiserror::Error;

pub use adapter::SessionStoreAdapter;
pub(crate) use caching::CachingSessionStore;
pub(crate) use encrypting::EncryptingSessionStore;

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
pub enum SessionStoreError {
    #[error("Session not found")]
    NotFound,
    #[error("Generic error: {0}")]
    Generic(String),
}

impl From<spark_wallet::SessionStoreError> for SessionStoreError {
    fn from(e: spark_wallet::SessionStoreError) -> Self {
        match e {
            spark_wallet::SessionStoreError::NotFound => SessionStoreError::NotFound,
            spark_wallet::SessionStoreError::Generic(msg) => SessionStoreError::Generic(msg),
        }
    }
}

impl From<SessionStoreError> for spark_wallet::SessionStoreError {
    fn from(e: SessionStoreError) -> Self {
        match e {
            SessionStoreError::NotFound => spark_wallet::SessionStoreError::NotFound,
            SessionStoreError::Generic(msg) => spark_wallet::SessionStoreError::Generic(msg),
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
pub trait SessionStore: Send + Sync {
    async fn get_session(
        &self,
        service_identity_key: PublicKey,
    ) -> Result<Session, SessionStoreError>;

    async fn set_session(
        &self,
        service_identity_key: PublicKey,
        session: Session,
    ) -> Result<(), SessionStoreError>;
}
