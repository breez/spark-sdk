use platform_utils::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;

use bitcoin::secp256k1::PublicKey;
use platform_utils::tokio;
use thiserror::Error;

#[cfg(any(test, feature = "test-utils"))]
pub mod tests;

#[derive(Debug, Error, Clone)]
pub enum SessionManagerError {
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

impl Session {
    pub fn is_valid(&self) -> bool {
        let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return false;
        };
        self.expiration > duration.as_secs()
    }
}

#[macros::async_trait]
pub trait SessionManager: Send + Sync {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionManagerError>;
    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionManagerError>;
}

#[derive(Default)]
pub struct InMemorySessionManager {
    sessions: tokio::sync::Mutex<HashMap<PublicKey, Session>>,
}

#[macros::async_trait]
impl SessionManager for InMemorySessionManager {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionManagerError> {
        self.sessions
            .lock()
            .await
            .get(service_identity_key)
            .cloned()
            .ok_or(SessionManagerError::NotFound)
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionManagerError> {
        self.sessions
            .lock()
            .await
            .insert(*service_identity_key, session);
        Ok(())
    }
}

#[cfg(test)]
mod in_memory_tests {
    use super::*;
    use crate::session_manager::tests as shared_tests;
    use macros::async_test_all;

    #[async_test_all]
    async fn test_get_session_not_found() {
        shared_tests::test_get_session_not_found(&InMemorySessionManager::default()).await;
    }

    #[async_test_all]
    async fn test_set_and_get() {
        shared_tests::test_set_and_get(&InMemorySessionManager::default()).await;
    }

    #[async_test_all]
    async fn test_overwrite_session() {
        shared_tests::test_overwrite_session(&InMemorySessionManager::default()).await;
    }

    #[async_test_all]
    async fn test_sessions_are_isolated_by_key() {
        shared_tests::test_sessions_are_isolated_by_key(&InMemorySessionManager::default()).await;
    }

    #[async_test_all]
    async fn test_get_after_unrelated_set() {
        shared_tests::test_get_after_unrelated_set(&InMemorySessionManager::default()).await;
    }
}
