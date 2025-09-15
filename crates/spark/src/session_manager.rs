use std::collections::HashMap;
use web_time::{SystemTime, UNIX_EPOCH};

use bitcoin::secp256k1::PublicKey;
use thiserror::Error;
use tokio_with_wasm::alias as tokio;

#[derive(Debug, Error, Clone)]
pub enum SessionManagerError {
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
            .ok_or(SessionManagerError::Generic(
                "Session not found".to_string(),
            ))
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
