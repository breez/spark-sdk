//! File-based `SQLite` backend.

use std::sync::Arc;

use macros::async_trait;
use spark_wallet::PublicKey;

use crate::{Network, SdkError, SqliteStorage};

use super::{ResolvedStores, StorageBackend};

/// `SQLite` backend. The database path is derived per-tenant from the network
/// and identity public key, so one `storage_dir` can hold many tenants'
/// databases.
pub(super) struct SqliteBackend {
    storage_dir: String,
}

impl SqliteBackend {
    pub(super) fn new(storage_dir: String) -> Self {
        Self { storage_dir }
    }
}

#[async_trait]
impl StorageBackend for SqliteBackend {
    async fn create_stores(
        &self,
        network: Network,
        identity: Vec<u8>,
    ) -> Result<Arc<ResolvedStores>, SdkError> {
        let identity =
            PublicKey::from_slice(&identity).map_err(|e| SdkError::Generic(e.to_string()))?;
        let db_path = crate::default_storage_path(&self.storage_dir, &network, &identity)?;
        let storage = Arc::new(SqliteStorage::new(&db_path)?);
        Ok(Arc::new(ResolvedStores {
            storage,
            tree_store: None,
            token_output_store: None,
            session_store: None,
        }))
    }
}
