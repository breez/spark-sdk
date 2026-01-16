use std::sync::Arc;

use thiserror::Error;

use crate::{StorageError, persist::Storage};

#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct PluginStorage {
    plugin_id: String,
    storage: Arc<dyn Storage>,
}

type StorageResult<T> = Result<T, PluginStorageError>;

#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum PluginStorageError {
    #[error("Could not write to plugin storage: {err}")]
    Generic { err: String },
}

impl From<StorageError> for PluginStorageError {
    fn from(value: StorageError) -> Self {
        Self::Generic {
            err: value.to_string(),
        }
    }
}

impl PluginStorage {
    pub(crate) fn new(storage: Arc<dyn Storage>, plugin_id: String) -> Self {
        Self { plugin_id, storage }
    }

    pub(crate) fn scoped_key(&self, key: &str) -> String {
        format!("{}-{}", self.plugin_id, key)
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl PluginStorage {
    pub async fn set_item(&self, key: &str, value: String) -> StorageResult<()> {
        let scoped_key = self.scoped_key(key);
        self.storage
            .set_cached_item(scoped_key, value)
            .await
            .map_err(Into::into)
    }

    pub async fn get_item(&self, key: &str) -> StorageResult<Option<String>> {
        let scoped_key = self.scoped_key(key);
        self.storage
            .get_cached_item(scoped_key)
            .await
            .map_err(Into::into)
    }

    pub async fn remove_item(&self, key: &str) -> StorageResult<()> {
        let scoped_key = self.scoped_key(key);
        self.storage
            .delete_cached_item(scoped_key)
            .await
            .map_err(Into::into)
    }
}
