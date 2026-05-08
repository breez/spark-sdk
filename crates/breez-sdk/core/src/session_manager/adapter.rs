use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;

use super::SessionManager;

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
