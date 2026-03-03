use lnurl_models::sanitize_username;

use crate::{
    CheckLightningAddressRequest, LightningAddressInfo, LnurlInfo, RegisterLightningAddressRequest,
    error::SdkError, persist::ObjectCacheRepository,
};

use super::BreezSdk;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    pub async fn check_lightning_address_available(
        &self,
        req: CheckLightningAddressRequest,
    ) -> Result<bool, SdkError> {
        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let username = sanitize_username(&req.username);
        let available = client.check_username_available(&username).await?;
        Ok(available)
    }

    pub async fn get_lightning_address(&self) -> Result<Option<LightningAddressInfo>, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let cached = cache.fetch_lightning_address().await?;
        if cached.is_none() && self.lnurl_server_client.is_some() {
            return self.recover_lightning_address().await;
        }
        Ok(cached.flatten())
    }

    pub async fn register_lightning_address(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> Result<LightningAddressInfo, SdkError> {
        // Ensure spark private mode is initialized before registering
        self.ensure_spark_private_mode_initialized().await?;

        self.register_lightning_address_internal(request).await
    }

    pub async fn delete_lightning_address(&self) -> Result<(), SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let Some(address_info) = cache.fetch_lightning_address().await?.flatten() else {
            return Ok(());
        };

        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let params = crate::lnurl::UnregisterLightningAddressRequest {
            username: address_info.username,
        };

        client.unregister_lightning_address(&params).await?;
        cache.delete_lightning_address().await?;
        Ok(())
    }
}

// Private lightning address methods
impl BreezSdk {
    /// Attempts to recover a lightning address from the lnurl server.
    pub(super) async fn recover_lightning_address(
        &self,
    ) -> Result<Option<LightningAddressInfo>, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());

        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };
        let resp = client.recover_lightning_address().await?;

        let result = if let Some(resp) = resp {
            let address_info = resp.into();
            cache.save_lightning_address(&address_info).await?;
            Some(address_info)
        } else {
            cache.delete_lightning_address().await?;
            None
        };

        Ok(result)
    }

    pub(super) async fn register_lightning_address_internal(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> Result<LightningAddressInfo, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let username = sanitize_username(&request.username);

        let description = match request.description {
            Some(description) => description,
            None => format!("Pay to {}@{}", username, client.domain()),
        };

        let params = crate::lnurl::RegisterLightningAddressRequest {
            username: username.clone(),
            description: description.clone(),
            lnurl_private_mode_enabled: !self.config.support_lnurl_verify,
        };

        let response = client.register_lightning_address(&params).await?;
        let address_info = LightningAddressInfo {
            lightning_address: response.lightning_address,
            description,
            lnurl: LnurlInfo::new(response.lnurl),
            username,
        };
        cache.save_lightning_address(&address_info).await?;
        Ok(address_info)
    }
}

#[cfg(test)]
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use crate::{
        LightningAddressInfo, LnurlInfo, persist::Storage, persist::sqlite::SqliteStorage,
    };

    use crate::persist::ObjectCacheRepository;

    fn create_temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("breez-test-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn create_temp_storage(name: &str) -> (Arc<SqliteStorage>, PathBuf) {
        let dir = create_temp_dir(name);
        let storage = SqliteStorage::new(&dir).expect("Failed to create storage");
        (Arc::new(storage), dir)
    }

    fn sample_address_info() -> LightningAddressInfo {
        LightningAddressInfo {
            lightning_address: "test@example.com".to_string(),
            username: "test".to_string(),
            description: "Test address".to_string(),
            lnurl: LnurlInfo::new("https://example.com/.well-known/lnurlp/test".to_string()),
        }
    }

    #[tokio::test]
    async fn test_fetch_returns_none_when_never_recovered() {
        let (storage, _dir) = create_temp_storage("never_recovered");
        let cache = ObjectCacheRepository::new(storage as Arc<_>);

        // Key absent -> None (never recovered)
        let result = cache.fetch_lightning_address().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_fetch_returns_some_none_after_delete() {
        let (storage, _dir) = create_temp_storage("after_delete");
        let cache = ObjectCacheRepository::new(storage as Arc<_>);

        // Save an address, then delete it
        cache
            .save_lightning_address(&sample_address_info())
            .await
            .unwrap();
        cache.delete_lightning_address().await.unwrap();

        // Key present, value null -> Some(None) (recovered, no address)
        let result = cache.fetch_lightning_address().await.unwrap();
        assert!(
            matches!(result, Some(None)),
            "Expected Some(None) after delete"
        );
    }

    #[tokio::test]
    async fn test_fetch_returns_some_some_after_save() {
        let (storage, _dir) = create_temp_storage("after_save");
        let cache = ObjectCacheRepository::new(storage as Arc<_>);

        cache
            .save_lightning_address(&sample_address_info())
            .await
            .unwrap();

        // Key present, value non-null -> Some(Some(info))
        let result = cache.fetch_lightning_address().await.unwrap();
        let info = result
            .flatten()
            .expect("Expected Some(Some(info)) after save");
        assert_eq!(info.lightning_address, "test@example.com");
    }

    #[tokio::test]
    async fn test_backward_compat_old_cached_json() {
        let (storage, _dir) = create_temp_storage("backward_compat");

        // Simulate old cache format: raw JSON object without Option wrapper
        let old_value = serde_json::to_string(&sample_address_info()).unwrap();
        storage
            .set_cached_item("lightning_address".to_string(), old_value)
            .await
            .unwrap();

        let cache = ObjectCacheRepository::new(storage as Arc<_>);
        let result = cache.fetch_lightning_address().await.unwrap();
        let info = result
            .flatten()
            .expect("Expected old cached JSON to deserialize as Some(info)");
        assert_eq!(info.lightning_address, "test@example.com");
    }
}
