use std::sync::Arc;

use tokio::sync::watch;
use tracing::error;

use crate::{EventEmitter, GetInfoRequest, GetInfoResponse, Storage, error::SdkError};

use super::{RuntimeEvent, RuntimeProfile};
use crate::sdk::{BreezSdk, SyncType};
use crate::utils::payments::get_payment_and_emit_event;

pub(super) struct ServerRuntime;

#[macros::async_trait]
impl RuntimeProfile for ServerRuntime {
    fn starts_background_services(&self) -> bool {
        false
    }

    async fn start_sdk_services(&self, sdk: &BreezSdk, initial_synced_sender: watch::Sender<bool>) {
        if let Err(e) = initial_synced_sender.send(true) {
            error!("Failed to set initial synced signal in server mode: {e:?}");
        }

        sdk.event_emitter
            .add_runtime_event_handler(Box::new(ServerRuntimeEventHandler {
                storage: sdk.storage.clone(),
            }))
            .await;
    }

    async fn run_user_sync(
        &self,
        sdk: &BreezSdk,
        sync_type: SyncType,
        force: bool,
    ) -> Result<(), SdkError> {
        sdk.sync_wallet_internal(sync_type, force).await
    }

    async fn get_info(
        &self,
        sdk: &BreezSdk,
        request: GetInfoRequest,
    ) -> Result<GetInfoResponse, SdkError> {
        if request.ensure_synced.unwrap_or_default() {
            return Err(SdkError::InvalidInput(
                "ensure_synced is not supported when background_tasks_enabled is false; call sync_wallet explicitly instead".to_string(),
            ));
        }

        let (balance_sats, token_balances) = tokio::try_join!(
            sdk.spark_wallet.get_balance(),
            sdk.spark_wallet.get_token_balances(),
        )?;

        let token_balances = token_balances
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect();

        Ok(GetInfoResponse {
            identity_pubkey: sdk.spark_wallet.get_identity_public_key().to_string(),
            balance_sats,
            token_balances,
        })
    }

    async fn maybe_ensure_spark_private_mode_initialized(
        &self,
        _sdk: &BreezSdk,
    ) -> Result<(), SdkError> {
        Ok(())
    }
}

struct ServerRuntimeEventHandler {
    storage: Arc<dyn Storage>,
}

#[macros::async_trait]
impl crate::events::RuntimeEventHandler for ServerRuntimeEventHandler {
    async fn handle(&self, emitter: &EventEmitter, event: RuntimeEvent) {
        match event {
            RuntimeEvent::DepositClaimed {
                payment,
                should_emit_event,
            } => {
                if should_emit_event {
                    get_payment_and_emit_event(&self.storage, emitter, *payment).await;
                }
            }
            RuntimeEvent::StableBalanceConversionCompleted => {}
        }
    }
}
