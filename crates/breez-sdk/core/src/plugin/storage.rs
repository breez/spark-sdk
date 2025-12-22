use std::sync::{Arc, Weak};

use thiserror::Error;

use crate::{StorageError, persist::Storage};

#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct PluginStorage {
    plugin_id: String,
    storage: Weak<dyn Storage>,
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
    pub(crate) fn new(storage: Weak<dyn Storage>, plugin_id: String) -> StorageResult<Self> {
        if plugin_id.is_empty() {
            tracing::error!("Plugin ID cannot be an empty string!");
            return Err(PluginStorageError::Generic {
                err: "Plugin ID cannot be an empty string!".to_string(),
            });
        }

        Ok(Self { storage, plugin_id })
    }

    fn get_persister(&self) -> StorageResult<Arc<dyn Storage>> {
        self.storage.upgrade().ok_or(PluginStorageError::Generic {
            err: "SDK is not running.".to_string(),
        })
    }

    pub(crate) fn scoped_key(&self, key: &str) -> String {
        format!("{}-{}", self.plugin_id, key)
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl PluginStorage {
    pub async fn set_item(&self, key: &str, value: String) -> StorageResult<()> {
        let scoped_key = self.scoped_key(key);
        self.get_persister()?
            .set_cached_item(scoped_key.to_string(), value)
            .await
            .map_err(Into::into)
    }

    pub async fn get_item(&self, key: &str) -> StorageResult<Option<String>> {
        let scoped_key = self.scoped_key(key);
        self.get_persister()?
            .get_cached_item(scoped_key.to_string())
            .await
            .map_err(Into::into)
    }

    pub async fn remove_item(&self, key: &str) -> StorageResult<()> {
        let scoped_key = self.scoped_key(key);
        self.get_persister()?
            .delete_cached_item(scoped_key.to_string())
            .await
            .map_err(Into::into)
    }
}
