use std::sync::{Arc, Weak};

use maybe_sync::{MaybeSend, MaybeSync};

use crate::sdk::BreezSdk;
use tracing::warn;

mod storage;

pub use storage::*;

#[macros::async_trait]
pub trait RustPlugin: MaybeSend + MaybeSync {
    fn id(&self) -> String;
    async fn on_start(&self, sdk: Weak<BreezSdk>, storage: PluginStorage);
    async fn on_stop(&self);
}

#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait Plugin: MaybeSend + MaybeSync {
    fn id(&self) -> String;
    async fn on_start(&self, sdk: Arc<BreezSdk>, storage: Arc<PluginStorage>);
    async fn on_stop(&self);
}

#[derive(Clone)]
pub(crate) struct PluginWrapper {
    inner: Arc<dyn Plugin>,
}

impl PluginWrapper {
    pub(crate) fn new(plugin: Arc<dyn Plugin>) -> Self {
        Self { inner: plugin }
    }
}

#[macros::async_trait]
impl RustPlugin for PluginWrapper {
    fn id(&self) -> String {
        self.inner.id()
    }

    async fn on_start(&self, sdk: Weak<BreezSdk>, storage: PluginStorage) {
        let Some(sdk) = sdk.upgrade() else {
            warn!(
                "Tried to start plugin {} while SDK was unavailable",
                self.id()
            );
            return;
        };
        self.inner.on_start(sdk, Arc::new(storage)).await;
    }

    async fn on_stop(&self) {
        self.inner.on_stop().await;
    }
}
