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
        Ok(cache.fetch_lightning_address().await?)
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
        let Some(address_info) = cache.fetch_lightning_address().await? else {
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

        // Query settings directly from spark wallet to avoid recursion through get_user_settings()
        let spark_user_settings = self.spark_wallet.query_wallet_settings().await?;
        let nostr_pubkey = if spark_user_settings.private_enabled {
            Some(self.nostr_client.nostr_pubkey())
        } else {
            None
        };

        let params = crate::lnurl::RegisterLightningAddressRequest {
            username: username.clone(),
            description: description.clone(),
            nostr_pubkey,
            no_invoice_paid_support: self.config.no_invoice_paid_support,
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
