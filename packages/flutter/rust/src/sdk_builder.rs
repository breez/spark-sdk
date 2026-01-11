use std::sync::Arc;

use breez_sdk_spark::{ChainApiType, Config, Credentials, SdkError, Seed};
use flutter_rust_bridge::frb;

use crate::sdk::BreezSdk;

pub struct SdkBuilder {
    inner: Arc<breez_sdk_spark::SdkBuilder>,
}

impl SdkBuilder {
    #[frb(sync)]
    pub fn new(config: Config, seed: Seed) -> Self {
        Self {
            inner: Arc::new(breez_sdk_spark::SdkBuilder::new(config, seed)),
        }
    }

    #[frb(sync)]
    pub fn with_default_storage(self, storage_dir: String) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_default_storage(storage_dir);
        Self {
            inner: Arc::new(builder),
        }
    }

    #[frb(sync)]
    pub fn with_key_set(self, config: breez_sdk_spark::KeySetConfig) -> Self {
        let builder =
            <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner).with_key_set(config);
        Self {
            inner: Arc::new(builder),
        }
    }

    #[frb(sync)]
    pub fn with_rest_chain_service(
        self,
        url: String,
        api_type: ChainApiType,
        credentials: Option<Credentials>,
    ) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_rest_chain_service(url, api_type, credentials);
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
