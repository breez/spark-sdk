use std::sync::Arc;

use tokio::sync::watch;

use crate::{GetInfoRequest, GetInfoResponse, error::SdkError, models::Config};

use super::{BreezSdk, SyncType};

mod client;
mod server;

use client::ClientRuntime;
use server::ServerRuntime;

pub(crate) type SdkRuntime = Arc<dyn RuntimeProfile>;

/// Internal runtime events consumed by the selected runtime profile.
///
/// These events are not forwarded to external SDK listeners; they decouple
/// non-runtime-owned modules from runtime-specific background behavior.
#[derive(Debug, Clone)]
pub(crate) enum RuntimeEvent {
    StableBalanceConversionCompleted,
    DepositClaimed {
        payment: Box<crate::models::Payment>,
        should_emit_event: bool,
    },
}

pub(crate) fn runtime_from_config(config: &Config) -> SdkRuntime {
    if config.background_tasks_enabled {
        Arc::new(ClientRuntime)
    } else {
        Arc::new(ServerRuntime)
    }
}

#[macros::async_trait]
pub(crate) trait RuntimeProfile: Send + Sync {
    fn starts_background_services(&self) -> bool;

    async fn start_sdk_services(&self, sdk: &BreezSdk, initial_synced_sender: watch::Sender<bool>);

    async fn run_user_sync(
        &self,
        sdk: &BreezSdk,
        sync_type: SyncType,
        force: bool,
    ) -> Result<(), SdkError>;

    async fn get_info(
        &self,
        sdk: &BreezSdk,
        request: GetInfoRequest,
    ) -> Result<GetInfoResponse, SdkError>;

    async fn maybe_ensure_spark_private_mode_initialized(
        &self,
        sdk: &BreezSdk,
    ) -> Result<(), SdkError>;
}

#[cfg(test)]
mod tests {
    use crate::{Network, default_config, default_server_config};

    use super::runtime_from_config;

    #[test]
    fn runtime_from_default_config_starts_background_services() {
        let runtime = runtime_from_config(&default_config(Network::Regtest));

        assert!(runtime.starts_background_services());
    }

    #[test]
    fn server_runtime_gates_background_services() {
        let config = default_server_config(Network::Regtest);
        let runtime = runtime_from_config(&config);

        assert!(!runtime.starts_background_services());
    }
}
