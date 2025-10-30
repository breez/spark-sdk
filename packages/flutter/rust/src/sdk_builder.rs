use std::sync::Arc;

use breez_sdk_spark::{Config, Credentials, SdkError};
pub use breez_sdk_spark::{Seed, Storage, sync_storage::SyncStorage};
use flutter_rust_bridge::frb;

use crate::{models::KeySetType, sdk::BreezSdk};

pub struct SdkBuilder {
    inner: Arc<breez_sdk_spark::SdkBuilder>,
}

impl SdkBuilder {
    #[frb(sync)]
    pub fn new(config: Config, seed: Seed, storage: Arc<dyn Storage>) -> Self {
        Self {
            inner: Arc::new(breez_sdk_spark::SdkBuilder::new(config, seed, storage)),
        }
    }

    #[frb(sync)]
    pub fn with_key_set(
        self,
        key_set_type: KeySetType,
        use_address_index: bool,
        account_number: Option<u32>,
    ) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner).with_key_set(
            key_set_type,
            use_address_index,
            account_number,
        );
        Self {
            inner: Arc::new(builder),
        }
    }

    #[frb(sync)]
    pub fn with_rest_chain_service(self, url: String, credentials: Option<Credentials>) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_rest_chain_service(url, credentials);
        Self {
            inner: Arc::new(builder),
        }
    }

    #[frb(sync)]
    pub fn with_real_time_sync_storage(self, storage: Arc<dyn SyncStorage>) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_real_time_sync_storage(storage);
        Self {
            inner: Arc::new(builder),
        }
    }

    pub async fn build(&self) -> Result<BreezSdk, SdkError> {
        let sdk = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .build()
            .await?;
        Ok(BreezSdk {
            inner: Arc::new(sdk),
        })
    }
}
