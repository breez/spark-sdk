use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;

use super::SessionStore;

/// Internal adapter that exposes a user-supplied [`SessionStore`] to
/// [`spark_wallet`] (which has its own identical-shape trait).
///
/// Used only by the WASM bindings to plumb a JS-side session store
/// (constructed from a JS storage backend) into the core wallet stack. The
/// public Rust API no longer accepts user-supplied session stores; the
/// canonical session store is derived from the [`SdkContext`](crate::SdkContext)'s
/// DB pool (or defaulted to in-memory).
pub(crate) struct SessionStoreAdapter(pub Arc<dyn SessionStore>);

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
