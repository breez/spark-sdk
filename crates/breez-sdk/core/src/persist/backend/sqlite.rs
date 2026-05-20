//! File-based `SQLite` backend.

use std::sync::Arc;

use macros::async_trait;
use spark_wallet::PublicKey;

use crate::{Network, SdkError, SqliteStorage};

use super::{ResolvedStores, StorageBackend};

/// `SQLite` backend. The database path is derived per-tenant from the identity
/// public key, so one `storage_dir` can hold many tenants' databases.
pub(super) struct SqliteBackend {
    storage_dir: String,
    network: Network,
}

impl SqliteBackend {
    pub(super) fn new(storage_dir: String, network: Network) -> Self {
        Self {
            storage_dir,
            network,
        }
    }
}

#[async_trait]
impl StorageBackend for SqliteBackend {
    async fn create_stores(&self, identity: &PublicKey) -> Result<ResolvedStores, SdkError> {
        let db_path = crate::default_storage_path(&self.storage_dir, &self.network, identity)?;
        let storage = Arc::new(SqliteStorage::new(&db_path)?);
        Ok(ResolvedStores {
            storage,
            tree_store: None,
            token_output_store: None,
            session_store: None,
        })
    }
}
