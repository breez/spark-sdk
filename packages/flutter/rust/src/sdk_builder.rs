use std::sync::Arc;

use breez_sdk_spark::{ChainApiType, Config, Credentials, SdkError, Seed, Session};
use flutter_rust_bridge::{DartFnFuture, frb};

use crate::{
    chain_service::BitcoinChainServiceHandle, connection_manager::ConnectionManager,
    sdk::BreezSdk, session_manager::CallbackSessionManager, ssp_connection_manager::SspConnectionManager,
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

    #[frb(sync)]
    pub fn with_ssp_connection_manager(self, manager: SspConnectionManager) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_ssp_connection_manager(manager.inner);
        Self {
            inner: Arc::new(builder),
        }
    }

    #[frb(sync)]
    pub fn with_connection_manager(self, connection_manager: &ConnectionManager) -> Self {
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_connection_manager(connection_manager.inner.clone());
        Self {
            inner: Arc::new(builder),
        }
    }

    /// Provide a custom session manager backed by Dart callbacks.
    ///
    /// Both callbacks receive the service identity public key as a
    /// hex-encoded string. `getSession` returns `null` when no session is
    /// cached (which the SDK treats as "needs authentication"). Throwing from
    /// either callback surfaces as a generic session manager error.
    #[frb(sync)]
    pub fn with_session_manager(
        self,
        get_session: impl Fn(String) -> DartFnFuture<Option<Session>> + Send + Sync + 'static,
        set_session: impl Fn(String, Session) -> DartFnFuture<()> + Send + Sync + 'static,
    ) -> Self {
        let session_manager = Arc::new(CallbackSessionManager {
            get_session_fn: Arc::new(get_session),
            set_session_fn: Arc::new(set_session),
        });
        let builder = <breez_sdk_spark::SdkBuilder as Clone>::clone(&self.inner)
            .with_session_manager(session_manager);
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
