use breez_plugins::{PluginStorageController, StorageResult};

use crate::{Storage, persist::postgres::PostgresStorage};

#[macros::async_trait]
impl PluginStorageController for PostgresStorage {
    async fn get_item(&self, key: String) -> StorageResult<Option<String>> {
        self.get_cached_item(key).await.map_err(Into::into)
    }

    async fn set_item(&self, key: String, value: String) -> StorageResult<()> {
        self.set_cached_item(key, value).await.map_err(Into::into)
    }

    async fn set_item_safe(
        &self,
        key: String,
        value: String,
        old_value: String,
    ) -> StorageResult<()> {
        self.set_cached_item_safe(key, value, old_value)
            .await
            .map_err(Into::into)
    }

    async fn remove_item(&self, key: String) -> StorageResult<()> {
        self.delete_cached_item(key).await.map_err(Into::into)
    }
}
