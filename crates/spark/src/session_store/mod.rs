use platform_utils::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;

use bitcoin::secp256k1::PublicKey;
use platform_utils::tokio;
use thiserror::Error;

#[cfg(any(test, feature = "test-utils"))]
pub mod tests;

#[derive(Debug, Error, Clone)]
pub enum SessionStoreError {
    #[error("Session not found")]
    NotFound,
    #[error("Generic error: {0}")]
    Generic(String),
}

#[derive(Clone)]
pub struct Session {
    pub token: String,
    pub expiration: u64,
}

/// Sessions within this margin of expiry are treated as already invalid, so a
/// token that would die mid-call is refreshed up front instead of failing the
/// call it authenticates.
const EXPIRY_MARGIN_SECS: u64 = 30;

impl Session {
    pub fn is_valid(&self) -> bool {
        let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return false;
        };
        self.expiration > duration.as_secs().saturating_add(EXPIRY_MARGIN_SECS)
    }
}

#[macros::async_trait]
pub trait SessionStore: Send + Sync {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionStoreError>;
    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionStoreError>;
}

#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: tokio::sync::Mutex<HashMap<PublicKey, Session>>,
}

#[macros::async_trait]
impl SessionStore for InMemorySessionStore {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionStoreError> {
        self.sessions
            .lock()
            .await
            .get(service_identity_key)
            .cloned()
            .ok_or(SessionStoreError::NotFound)
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionStoreError> {
        self.sessions
            .lock()
            .await
            .insert(*service_identity_key, session);
        Ok(())
    }
}

#[cfg(test)]
mod session_validity_tests {
    use super::*;
    use macros::test_all;

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn session(expiration: u64) -> Session {
        Session {
            token: "token".to_string(),
            expiration,
        }
    }

    #[test_all]
    fn expired_session_is_invalid() {
        assert!(!session(now_secs().saturating_sub(10)).is_valid());
    }

    #[test_all]
    fn session_expiring_within_margin_is_invalid() {
        assert!(!session(now_secs() + EXPIRY_MARGIN_SECS / 2).is_valid());
    }

    #[test_all]
    fn session_outliving_margin_is_valid() {
        assert!(session(now_secs() + EXPIRY_MARGIN_SECS + 60).is_valid());
    }
}

#[cfg(test)]
mod in_memory_tests {
    use super::*;
    use crate::session_store::tests as shared_tests;
    use macros::async_test_all;

    #[async_test_all]
    async fn test_get_session_not_found() {
        shared_tests::test_get_session_not_found(&InMemorySessionStore::default()).await;
    }

    #[async_test_all]
    async fn test_set_and_get() {
        shared_tests::test_set_and_get(&InMemorySessionStore::default()).await;
    }

    #[async_test_all]
    async fn test_overwrite_session() {
        shared_tests::test_overwrite_session(&InMemorySessionStore::default()).await;
    }

    #[async_test_all]
    async fn test_sessions_are_isolated_by_key() {
        shared_tests::test_sessions_are_isolated_by_key(&InMemorySessionStore::default()).await;
    }

    #[async_test_all]
    async fn test_get_after_unrelated_set() {
        shared_tests::test_get_after_unrelated_set(&InMemorySessionStore::default()).await;
    }
}
