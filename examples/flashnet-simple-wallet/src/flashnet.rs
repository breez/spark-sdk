use bitcoin::secp256k1::PublicKey;
use spark_wallet::{Session, SessionManager, SessionManagerError};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub enum StoreError {
    Generic,
}

pub trait SignerStore {
    fn get_seed(&self, public_key: &PublicKey) -> Result<Vec<u8>, StoreError>;

    fn insert_seed(&self, public_key: PublicKey, seed: Vec<u8>) -> Result<(), StoreError>;
}

pub struct SimpleSignerStore {
    store: Mutex<HashMap<PublicKey, Vec<u8>>>,
}

impl SimpleSignerStore {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }
}

impl SignerStore for SimpleSignerStore {
    fn get_seed(&self, public_key: &PublicKey) -> Result<Vec<u8>, StoreError> {
        let store = self.store.lock().unwrap();
        store
            .get(public_key)
            .ok_or(StoreError::Generic)
            .map(|v| v.clone())
    }

    fn insert_seed(&self, public_key: PublicKey, seed: Vec<u8>) -> Result<(), StoreError> {
        let mut store = self.store.lock().unwrap();
        store.insert(public_key, seed);
        Ok(())
    }
}

#[derive(Eq, Hash, PartialEq)]
pub struct SessionKey {
    user_public_key: PublicKey,
    operator_pub_key: PublicKey,
}

#[derive(Default)]
pub struct SessionStore {
    store: Mutex<HashMap<SessionKey, Session>>,
}

pub struct UserSessionManager {
    pub user_public_key: PublicKey,
    pub session_store: Arc<SessionStore>,
}

#[async_trait::async_trait]
impl SessionManager for UserSessionManager {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionManagerError> {
        let store = self.session_store.store.lock().unwrap();
        store
            .get(&SessionKey {
                user_public_key: self.user_public_key,
                operator_pub_key: *service_identity_key,
            })
            .ok_or(SessionManagerError::Generic(
                "Session not found".to_string(),
            ))
            .map(|v| v.clone())
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionManagerError> {
        let mut store = self.session_store.store.lock().unwrap();
        store.insert(
            SessionKey {
                user_public_key: self.user_public_key,
                operator_pub_key: *service_identity_key,
            },
            session,
        );
        Ok(())
    }
}
