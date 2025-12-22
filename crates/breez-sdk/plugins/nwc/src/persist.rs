use std::collections::HashMap;

use breez_sdk_spark::PluginStorage;

use crate::error::{NwcError, NwcResult};

const KEY_NWC_URIS: &str = "nwc_uris";
const KEY_NWC_SECKEY: &str = "nwc_seckey";

pub(crate) struct Persister {
    storage: PluginStorage,
}

impl Persister {
    pub(crate) fn new(storage: PluginStorage) -> Self {
        Self { storage }
    }

    pub(crate) async fn set_nwc_seckey(&self, key: String) -> NwcResult<()> {
        self.storage
            .set_item(KEY_NWC_SECKEY, key)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn get_nwc_seckey(&self) -> NwcResult<Option<String>> {
        self.storage
            .get_item(KEY_NWC_SECKEY)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn set_nwc_uri(&self, name: String, uri: String) -> NwcResult<()> {
        let mut nwc_uris = self.list_nwc_uris().await?;
        nwc_uris.insert(name, uri);
        self.storage
            .set_item(KEY_NWC_URIS, serde_json::to_string(&nwc_uris)?)
            .await?;
        Ok(())
    }

    pub(crate) async fn list_nwc_uris(&self) -> NwcResult<HashMap<String, String>> {
        let raw_uris = self
            .storage
            .get_item(KEY_NWC_URIS)
            .await?
            .unwrap_or("{}".to_string());
        let uris = serde_json::from_str(&raw_uris)?;
        Ok(uris)
    }

    pub(crate) async fn remove_nwc_uri(&self, name: String) -> NwcResult<()> {
        let mut nwc_uris = self.list_nwc_uris().await?;
        if nwc_uris.remove(&name).is_none() {
            NwcError::generic("Connection string not found.");
        }
        self.storage
            .set_item(KEY_NWC_URIS, serde_json::to_string(&nwc_uris)?)
            .await?;
        Ok(())
    }
}
