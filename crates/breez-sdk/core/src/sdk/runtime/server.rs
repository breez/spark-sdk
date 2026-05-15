use tokio::sync::watch;

use crate::{GetInfoRequest, GetInfoResponse, error::SdkError};

use super::RuntimeProfile;
use crate::sdk::{BreezSdk, SyncRequest, SyncType};

pub(super) struct ServerRuntime;

#[macros::async_trait]
impl RuntimeProfile for ServerRuntime {
    fn starts_background_services(&self) -> bool {
        false
    }

    fn start_sdk_services(&self, sdk: &BreezSdk, _initial_synced_sender: watch::Sender<bool>) {
        sdk.spawn_jwt_init();
    }

    async fn run_user_sync(
        &self,
        sdk: &BreezSdk,
        sync_type: SyncType,
        force: bool,
    ) -> Result<(), SdkError> {
        let request = SyncRequest::fire_and_forget(sync_type, force);
        sdk.sync_wallet_internal(&request).await
    }

    async fn get_info(
        &self,
        sdk: &BreezSdk,
        _request: GetInfoRequest,
    ) -> Result<GetInfoResponse, SdkError> {
        let balance_sats = sdk.spark_wallet.get_balance().await?;
        let token_balances = sdk
            .spark_wallet
            .get_token_balances()
            .await?
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
