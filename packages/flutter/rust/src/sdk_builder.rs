use std::sync::Arc;

use breez_sdk_spark::{ChainApiType, Config, Credentials, SdkError, Seed};
use flutter_rust_bridge::frb;

use crate::{
    chain_service::BitcoinChainServiceHandle, sdk::BreezSdk, sdk_context::SdkContext,
};

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
    pub fn with_account_number(self, account_number: u32) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_account_number(account_number);
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

    /// Sets a Rust-built chain service. Pass a handle from
    /// [`new_rest_chain_service`](crate::chain_service::new_rest_chain_service)
    /// to multiple `SdkBuilder`s to share one HTTP client across SDK instances.
    #[frb(sync)]
    pub fn with_chain_service(self, handle: &BitcoinChainServiceHandle) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_chain_service(handle.inner.clone());
        Self {
            inner: Arc::new(builder),
        }
    }

    /// Threads a shared [`SdkContext`] into the builder.
    ///
    /// Construct the context once via
    /// [`new_shared_sdk_context`](crate::sdk_context::new_shared_sdk_context)
    /// and pass the same handle to every `SdkBuilder` whose SDKs should share
    /// its HTTP client, operator gRPC channels, and Breez backend gRPC client.
    #[frb(sync)]
    pub fn with_shared_context(self, context: &SdkContext) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_shared_context(context.inner.clone());
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
