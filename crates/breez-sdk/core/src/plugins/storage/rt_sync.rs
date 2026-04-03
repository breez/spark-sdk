use breez_plugins::{PluginStorageController, PluginStorageError, StorageResult};

use crate::realtime_sync::SyncedStorage;

#[macros::async_trait]
impl PluginStorageController for SyncedStorage {
    async fn get_item(&self, key: String) -> StorageResult<Option<String>> {
        self.inner
            .get_cached_item(key)
            .await
            .map_err(PluginStorageError::generic)
    }

    async fn set_item(&self, key: String, value: String) -> StorageResult<()> {
        self.inner
            .set_cached_item(key, value)
            .await
            .map_err(PluginStorageError::generic)
    }

    async fn set_item_safe(
        &self,
        key: String,
        value: String,
        old_value: String,
    ) -> StorageResult<()> {
        self.inner
            .set_cached_item_safe(key, value, old_value)
            .await
            .map_err(PluginStorageError::generic)
    }

    async fn remove_item(&self, key: String) -> StorageResult<()> {
        self.inner
            .delete_cached_item(key)
            .await
            .map_err(|e| breez_plugins::PluginStorageError::Generic { err: e.to_string() })
    }
}
