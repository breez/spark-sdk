use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;

use super::SessionManager;

/// Internal adapter that exposes a user-supplied [`SessionManager`] to
/// [`spark_wallet`] (which has its own identical-shape trait).
///
/// Used only by the WASM bindings to plumb a JS-side session manager
/// (constructed from a JS storage backend) into the core wallet stack. The
/// public Rust API no longer accepts user-supplied session managers; the
/// canonical session manager is derived from the [`SdkContext`](crate::SdkContext)'s
/// DB pool (or defaulted to in-memory).
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
