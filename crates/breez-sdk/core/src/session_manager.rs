use std::{collections::HashMap, sync::Arc};

use bitcoin::secp256k1::PublicKey;
use spark_wallet::{Session, SessionManager, SessionManagerError};
use tracing::warn;

const PARTNER_ID_HEADER: &str = "partner_id";

pub(crate) struct BreezSessionManager {
    inner: Arc<dyn SessionManager>,
}

impl BreezSessionManager {
    pub(crate) fn new(inner: Arc<dyn SessionManager>) -> Self {
        Self { inner }
    }

    async fn get_or_set_partner_id(&self) -> Result<String, SessionManagerError> {
        todo!();
    }
}

#[macros::async_trait]
impl SessionManager for BreezSessionManager {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionManagerError> {
        let mut session = self.inner.get_session(service_identity_key).await?;
        let mut headers = HashMap::new();
        match self.get_or_set_partner_id().await {
            Ok(partner_id) => {
                headers.insert(PARTNER_ID_HEADER.to_string(), partner_id);
            }
            Err(err) => {
                warn!("Could not set partner_id: {err}");
            }
        }
        session.set_headers(headers);
        Ok(session)
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionManagerError> {
        self.inner.set_session(service_identity_key, session).await
    }
}
