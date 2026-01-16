use std::sync::Arc;

use crate::sdk::SdkServices;

mod storage;

pub use storage::*;

#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait Plugin: Send + Sync {
    fn id(&self) -> String;
    async fn on_start(&self, services: Arc<SdkServices>, storage: Arc<PluginStorage>);
    async fn on_stop(&self);
}
