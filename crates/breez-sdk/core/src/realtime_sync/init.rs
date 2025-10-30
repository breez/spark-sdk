use std::sync::Arc;

use breez_sdk_common::sync::{
    BreezSyncerClient, SigningClient, SyncProcessor, SyncService, storage::SyncStorage,
};
use spark_wallet::Signer;
use tracing::debug;
use uuid::Uuid;

use crate::{
    EventEmitter, Network,
    error::SdkError,
    persist::Storage,
    realtime_sync::{DefaultSyncSigner, SyncedStorage},
};

pub struct RealTimeSyncParams {
    pub server_url: String,
    pub api_key: Option<String>,
    pub network: Network,
    pub signer: Arc<dyn Signer>,
    pub storage: Arc<dyn Storage>,
    pub sync_storage: Arc<dyn SyncStorage>,
    pub shutdown_receiver: tokio::sync::watch::Receiver<()>,
    pub event_emitter: Arc<EventEmitter>,
}

pub async fn init_and_start_real_time_sync(
    params: RealTimeSyncParams,
) -> Result<Arc<dyn Storage>, SdkError> {
    debug!("Real-time sync is enabled.");
    let sync_service = Arc::new(SyncService::new(Arc::clone(&params.sync_storage)));
    let synced_storage = Arc::new(SyncedStorage::new(
        Arc::clone(&params.storage),
        Arc::clone(&sync_service),
        params.event_emitter,
    ));

    synced_storage.initial_setup();
    let storage: Arc<dyn Storage> = synced_storage.clone();
    let sync_client = BreezSyncerClient::new(&params.server_url, params.api_key.as_deref())
        .map_err(|e| SdkError::Generic(e.to_string()))?;

    let sync_coin_type = match params.network {
        Network::Mainnet => "0",
        Network::Regtest => "1",
    };
    let sync_signer = DefaultSyncSigner::new(
        Arc::clone(&params.signer),
        // This derivation path ensures no other software uses the same key for our storage with the same mnemonic.
        format!("m/448201320'/{sync_coin_type}'/0'/0/0")
            .parse()
            .map_err(|_| SdkError::Generic("Invalid sync signer derivation path".to_string()))?,
    );

    let signing_sync_client = SigningClient::new(
        Arc::new(sync_client),
        Arc::new(sync_signer),
        Uuid::now_v7().to_string(),
    );
    let sync_processor = Arc::new(SyncProcessor::new(
        signing_sync_client,
        sync_service.get_sync_trigger(),
        synced_storage,
        Arc::clone(&params.sync_storage),
    ));

    sync_processor
        .start(params.shutdown_receiver)
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to start real-time sync processor: {e}")))?;
    Ok(storage)
}
