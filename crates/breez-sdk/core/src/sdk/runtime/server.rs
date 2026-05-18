use tokio::sync::watch;

use crate::{GetInfoRequest, GetInfoResponse, error::SdkError};

use super::RuntimeProfile;
use crate::sdk::{BreezSdk, SyncType};

pub(super) struct ServerRuntime;

#[macros::async_trait]
impl RuntimeProfile for ServerRuntime {
    fn starts_background_services(&self) -> bool {
        false
    }

    async fn start_sdk_services(
        &self,
        sdk: &BreezSdk,
        _initial_synced_sender: watch::Sender<bool>,
    ) {
        sdk.spawn_jwt_init();
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

    async fn ensure_spark_private_mode_initialized(&self, _sdk: &BreezSdk) -> Result<(), SdkError> {
        Ok(())
    }
}
