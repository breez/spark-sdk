use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;

use super::SessionStore;

/// Adapts an SDK-facing [`SessionStore`] to the [`spark_wallet`] session-store
/// trait (which has its own identical-shape trait).
///
/// Used by the WASM bindings to plumb a JS-side session store into a
/// caller-supplied [`StorageBackend`](crate::StorageBackend).
pub struct SessionStoreAdapter(pub Arc<dyn SessionStore>);

impl SessionStoreAdapter {
    /// Wraps an SDK-facing [`SessionStore`] so it can be used as a
    /// [`spark_wallet`] session store.
    #[must_use]
    pub fn new(inner: Arc<dyn SessionStore>) -> Self {
        Self(inner)
    }
}

#[macros::async_trait]
impl spark_wallet::SessionStore for SessionStoreAdapter {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<spark_wallet::Session, spark_wallet::SessionStoreError> {
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
    ) -> Result<(), spark_wallet::SessionStoreError> {
        self.0
            .set_session(*service_identity_key, session.into())
            .await
            .map_err(Into::into)
    }
}
