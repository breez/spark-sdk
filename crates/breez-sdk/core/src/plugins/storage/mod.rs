#[cfg(all(
    feature = "postgres",
    not(all(target_family = "wasm", target_os = "unknown"))
))]
mod postgres;
mod rt_sync;
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod sqlite;

use std::sync::Arc;

use breez_plugins::{PluginStorageController, PluginStorageError, StorageResult};

use crate::{Storage, StorageError};

impl From<StorageError> for PluginStorageError {
    fn from(value: StorageError) -> Self {
        match value {
            StorageError::DataTooOld => PluginStorageError::DataTooOld,
            other => PluginStorageError::Generic {
                err: other.to_string(),
            },
        }
    }
}

/// Wraps an `Arc<dyn Storage>` to implement `PluginStorageController`.
/// Used by `SdkBuilder` when attaching plugins to the SDK.
pub(crate) struct StorageController(Arc<dyn Storage>);

impl StorageController {
    pub(crate) fn new(storage: Arc<dyn Storage>) -> Self {
        Self(storage)
    }
}

#[macros::async_trait]
impl PluginStorageController for StorageController {
    async fn get_item(&self, key: String) -> StorageResult<Option<String>> {
        self.0.get_cached_item(key).await.map_err(Into::into)
    }

    async fn set_item(&self, key: String, value: String) -> StorageResult<()> {
        self.0.set_cached_item(key, value).await.map_err(Into::into)
    }

    async fn set_item_safe(
        &self,
        key: String,
        value: String,
        old_value: String,
    ) -> StorageResult<()> {
        self.0
            .set_cached_item_safe(key, value, old_value)
            .await
            .map_err(Into::into)
    }

    async fn remove_item(&self, key: String) -> StorageResult<()> {
        self.0.delete_cached_item(key).await.map_err(Into::into)
    }
}
