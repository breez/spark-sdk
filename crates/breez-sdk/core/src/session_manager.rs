use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use platform_utils::tokio::sync::RwLock;
use spark_wallet::{Session, SessionManager, SessionManagerError};

const PARTNER_ID_HEADER: &str = "x-partner-jwt";

pub(crate) struct BreezSessionManager {
    inner: Arc<dyn SessionManager>,
    token: RwLock<Option<String>>,
}

impl BreezSessionManager {
    pub(crate) fn new(inner: Arc<dyn SessionManager>) -> Self {
        Self {
            inner,
            token: RwLock::new(None),
        }
    }

    pub(crate) async fn get_token(&self) -> Option<String> {
        self.token.read().await.clone()
    }

    pub(crate) async fn set_token(&self, new_token: String) {
        *self.token.write().await = Some(new_token);
    }

    async fn set_headers(&self, session: &mut Session) {
        if let Some(token) = self.token.read().await.as_ref() {
            if Some(token) != session.so_headers.get(PARTNER_ID_HEADER) {
                session
                    .so_headers
                    .insert(PARTNER_ID_HEADER.to_string(), token.clone());
            }
            if Some(token) != session.ssp_headers.get(PARTNER_ID_HEADER) {
                session
                    .ssp_headers
                    .insert(PARTNER_ID_HEADER.to_string(), token.clone());
            }
        }
    }
}

#[macros::async_trait]
impl SessionManager for BreezSessionManager {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionManagerError> {
        let mut session = self.inner.get_session(service_identity_key).await?;
        self.set_headers(&mut session).await;
        Ok(session)
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        mut session: Session,
    ) -> Result<(), SessionManagerError> {
        self.set_headers(&mut session).await;
        self.inner.set_session(service_identity_key, session).await
    }
}
