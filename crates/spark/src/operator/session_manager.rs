use std::collections::HashMap;

use bitcoin::secp256k1::PublicKey;
use thiserror::Error;
use tokio_with_wasm::alias as tokio;

#[derive(Debug, Error, Clone)]
pub enum SessionManagerError {
    #[error("Generic error: {0}")]
    Generic(String),
}

#[derive(Clone)]
pub struct OperatorSession {
    pub token: String,
    pub expiration: u64,
}

#[macros::async_trait]
pub trait SessionManager: Send + Sync {
    async fn get_session(
        &self,
        operator_identity_key: &PublicKey,
    ) -> Result<OperatorSession, SessionManagerError>;
    async fn set_session(
        &self,
        operator_identity_key: &PublicKey,
        session: OperatorSession,
    ) -> Result<(), SessionManagerError>;
}

#[derive(Default)]
pub struct InMemorySessionManager {
    sessions: tokio::sync::Mutex<HashMap<PublicKey, OperatorSession>>,
}

#[macros::async_trait]
impl SessionManager for InMemorySessionManager {
    async fn get_session(
        &self,
        operator_identity_key: &PublicKey,
    ) -> Result<OperatorSession, SessionManagerError> {
        self.sessions
            .lock()
            .await
            .get(operator_identity_key)
            .cloned()
            .ok_or(SessionManagerError::Generic(
                "Session not found".to_string(),
            ))
    }

    async fn set_session(
        &self,
        operator_identity_key: &PublicKey,
        session: OperatorSession,
    ) -> Result<(), SessionManagerError> {
        self.sessions
            .lock()
            .await
            .insert(*operator_identity_key, session);
        Ok(())
    }
}
