use std::sync::Arc;

use breez_sdk_common::sync::{
    BreezSyncerClient, SigningClient, SyncProcessor, SyncService, storage::SyncStorage,
};
use spark_wallet::Signer;
use tracing::debug;
use uuid::Uuid;

use crate::{
    Network,
    error::SdkError,
    persist::Storage,
    realtime_sync::{DefaultSyncSigner, SyncedStorage},
};

/// Creates the real-time sync components
///
/// # Arguments
///
/// * `server_url` - The URL of the sync server
/// * `api_key` - Optional API key for the sync server
/// * `network` - The network being used (Mainnet or Regtest)
/// * `signer` - The signer for wallet operations
/// * `storage` - The storage backend
/// * `sync_storage` - The sync storage backend
///
/// # Returns
///
/// A tuple containing the synced storage and sync processor
pub async fn init_and_start_real_time_sync(
    server_url: &str,
    api_key: Option<&str>,
    network: Network,
    signer: Arc<dyn Signer>,
    storage: Arc<dyn Storage>,
    sync_storage: Arc<dyn SyncStorage>,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
) -> Result<Arc<dyn Storage>, SdkError> {
    debug!("Real-time sync is enabled.");
    let sync_service = Arc::new(SyncService::new(Arc::clone(&sync_storage)));
    let synced_storage = Arc::new(SyncedStorage::new(
        Arc::clone(&storage),
        Arc::clone(&sync_service),
    ));

    synced_storage.start();
    let storage: Arc<dyn Storage> = synced_storage.clone();
    let sync_client = BreezSyncerClient::new(server_url, api_key)
        .map_err(|e| SdkError::Generic(e.to_string()))?;

    let sync_coin_type = match network {
        Network::Mainnet => "0",
        Network::Regtest => "1",
    };
    let sync_signer = DefaultSyncSigner::new(
        Arc::clone(&signer),
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
        Arc::clone(&sync_storage),
    ));

    sync_processor
        .start(shutdown_receiver)
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to start real-time sync processor: {e}")))?;
    Ok(storage)
}
