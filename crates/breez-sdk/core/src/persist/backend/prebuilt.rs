//! Backend over a caller-supplied [`CustomStorage`].

use macros::async_trait;
use spark_wallet::PublicKey;

use crate::SdkError;

use super::{CustomStorage, ResolvedStores, StorageBackend};

/// Returns the stores the caller already built. `identity` is ignored — the
/// caller is responsible for any tenant scoping.
pub(super) struct PrebuiltBackend {
    stores: CustomStorage,
}

impl PrebuiltBackend {
    pub(super) fn new(stores: CustomStorage) -> Self {
        Self { stores }
    }
}

#[async_trait]
impl StorageBackend for PrebuiltBackend {
    async fn create_stores(&self, _identity: &PublicKey) -> Result<ResolvedStores, SdkError> {
        Ok(ResolvedStores {
            storage: self.stores.storage.clone(),
            tree_store: self.stores.tree_store.clone(),
            token_output_store: self.stores.token_output_store.clone(),
            session_store: self.stores.session_store.clone(),
        })
    }
}
