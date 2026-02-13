use std::sync::Arc;

use breez_sdk_common::sync::{BreezSyncerClient, SigningClient, SyncProcessor, SyncService};
use tracing::debug;
use uuid::Uuid;

use crate::{
    EventEmitter, error::SdkError, persist::Storage, realtime_sync::SyncedStorage,
    sync_storage::SyncStorageWrapper,
};

pub struct RealTimeSyncParams {
    pub server_url: String,
    pub api_key: Option<String>,
    pub signer: Arc<dyn breez_sdk_common::sync::SyncSigner>,
    pub storage: Arc<dyn Storage>,
    pub shutdown_receiver: tokio::sync::watch::Receiver<()>,
    pub event_emitter: Arc<EventEmitter>,
}

pub struct RealTimeSyncResult {
    pub storage: Arc<dyn Storage>,
    pub signing_client: SigningClient,
}

pub async fn init_and_start_real_time_sync(
    params: RealTimeSyncParams,
) -> Result<RealTimeSyncResult, SdkError> {
    debug!("Real-time sync is enabled.");
    let sync_storage: Arc<dyn breez_sdk_common::sync::storage::SyncStorage> =
        Arc::new(SyncStorageWrapper::new(Arc::clone(&params.storage)));
    let sync_service = Arc::new(SyncService::new(Arc::clone(&sync_storage)));
    let synced_storage = Arc::new(SyncedStorage::new(
        Arc::clone(&params.storage),
        Arc::clone(&sync_service),
        params.event_emitter,
    ));

    synced_storage.initial_setup();
    let storage: Arc<dyn Storage> = synced_storage.clone();
    let sync_client: Arc<dyn breez_sdk_common::sync::SyncerClient> = Arc::new(
        BreezSyncerClient::new(&params.server_url, params.api_key.as_deref())
            .map_err(|e| SdkError::Generic(e.to_string()))?,
    );

    let signing_client = SigningClient::new(
        Arc::clone(&sync_client),
        Arc::clone(&params.signer),
        Uuid::now_v7().to_string(),
    );

    let sync_processor = Arc::new(SyncProcessor::new(
        signing_client.clone(),
        sync_service.get_sync_trigger(),
        synced_storage,
        Arc::clone(&sync_storage),
    ));

    sync_processor
        .start(params.shutdown_receiver)
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to start real-time sync processor: {e}")))?;
    Ok(RealTimeSyncResult {
        storage,
        signing_client,
    })
}
