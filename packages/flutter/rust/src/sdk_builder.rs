use std::sync::Arc;

pub use breez_sdk_spark::Storage;
use breez_sdk_spark::{Config, Credentials, SdkError};
use flutter_rust_bridge::frb;

use crate::sdk::BreezSdk;

pub struct SdkBuilder {
    inner: Arc<breez_sdk_spark::SdkBuilder>,
}

impl SdkBuilder {
    #[frb(sync)]
    pub fn new(config: Config, mnemonic: String, storage: Arc<dyn Storage>) -> Self {
        Self {
            inner: Arc::new(breez_sdk_spark::SdkBuilder::new(config, mnemonic, storage)),
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

    pub async fn build(&self) -> Result<BreezSdk, SdkError> {
        let sdk = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .build()
            .await?;
        Ok(BreezSdk {
            inner: Arc::new(sdk),
        })
    }
}
