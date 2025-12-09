use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, warn};
use web_time::SystemTime;

use crate::sync::{
    model::{IncomingChange, OutgoingChange, RecordId},
    signing_client::SigningClient,
    storage::SyncStorage,
};

const SYNC_BATCH_SIZE: u32 = 10;

#[allow(clippy::ref_option)]
#[cfg_attr(test, mockall::automock)]
#[macros::async_trait]
pub trait NewRecordHandler: Send + Sync {
    async fn on_incoming_change(&self, change: IncomingChange) -> anyhow::Result<()>;
    async fn on_replay_outgoing_change(&self, change: OutgoingChange) -> anyhow::Result<()>;
    async fn on_sync_completed(
        &self,
        incoming_count: Option<u32>,
        outgoing_count: Option<u32>,
    ) -> anyhow::Result<()>;
}

pub struct SyncProcessor {
    push_sync_trigger: broadcast::Receiver<RecordId>,
    new_record_handler: Arc<dyn NewRecordHandler>,
    client: SigningClient,
    storage: Arc<dyn SyncStorage>,
}

impl SyncProcessor {
    pub fn new(
        client: SigningClient,
        push_sync_trigger: broadcast::Receiver<RecordId>,
        new_record_handler: Arc<dyn NewRecordHandler>,
        storage: Arc<dyn SyncStorage>,
    ) -> Self {
        SyncProcessor {
            push_sync_trigger,
            new_record_handler,
            client,
            storage,
        }
    }

    pub async fn start(
        self: &Arc<Self>,
        shutdown_receiver: watch::Receiver<()>,
    ) -> anyhow::Result<()> {
        debug!("Starting sync processor");

        // Apply the LATEST outgoing record to the relational data store before starting the sync loops.
        // It's possible this outgoing record was inserted, while the database update after that was not completed yet.
        // This goes first to ensure consistency.
        self.ensure_outgoing_record_committed().await?;

        // Handle pending incoming records that were already fetched and stored locally.
        self.pull_sync_once_local().await?;

        // Now the storage is consistent. Background services can start.
        let (pull_trigger_tx, pull_trigger) = watch::channel(());
        self.start_subscribe_updates_task(shutdown_receiver.clone(), pull_trigger_tx);
        self.start_sync_loop_task(shutdown_receiver, pull_trigger);
        Ok(())
    }

    /// Task that subscribes to remote updates and notifies the sync loop when remote changes are detected.
    fn start_subscribe_updates_task(
        self: &Arc<Self>,
        shutdown_receiver: watch::Receiver<()>,
        pull_trigger_tx: watch::Sender<()>,
    ) {
        let clone = Arc::clone(self);
        tokio::spawn(async move {
            clone
                .subscribe_updates_forever(shutdown_receiver, pull_trigger_tx)
                .await;
        });
    }

    /// Apply the LATEST outgoing record to the relational data store before starting the sync loops.
    /// It's possible this outgoing record was inserted, while the database update after that was not completed yet.
    async fn ensure_outgoing_record_committed(&self) -> anyhow::Result<()> {
        let Some(record) = self.storage.get_latest_outgoing_change().await? else {
            debug!("There is no pending outgoing change to commit");
            return Ok(());
        };

        debug!(
            "Committing latest pending outgoing change for record {:?}, revision {}",
            record.change.id, record.change.revision
        );
        self.new_record_handler
            .on_replay_outgoing_change(record.try_into()?)
            .await?;
        Ok(())
    }

    async fn subscribe_updates_forever(
        &self,
        mut shutdown_receiver: watch::Receiver<()>,
        pull_trigger_tx: watch::Sender<()>,
    ) {
        loop {
            self.subscribe_updates(shutdown_receiver.clone(), pull_trigger_tx.clone())
                .await;
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                    debug!("Re-establishing update subscription after disconnection");
                }

                _ = shutdown_receiver.changed() => {
                    return;
                }
            }
        }
    }

    async fn subscribe_updates(
        &self,
        mut shutdown_receiver: watch::Receiver<()>,
        tx: watch::Sender<()>,
    ) {
        debug!("Subscribing to real-time sync update subscription");
        let mut stream = match self.client.listen_changes().await {
            Ok(stream) => stream,
            Err(e) => {
                error!("Failed to establish update subscription: {e}");
                return;
            }
        };

        loop {
            tokio::select! {
                _ = shutdown_receiver.changed() => {
                    debug!("Shutdown signal received, stopping update subscription");
                    break;
                }

                maybe_notification = stream.message() => {
                    match maybe_notification {
                        Ok(Some(notification)) => {
                            debug!("Received notification for client id: {:?}", notification.client_id);
                            if let Some(client_id) = notification.client_id && client_id == self.client.client_id {
                                debug!("Ignoring notification for ourselves");
                                continue;
                            }

                            if let Err(e) = tx.send(()) {
                                error!("Failed to send update notification: {}", e);
                                break;
                            }
                        }
                        Ok(None) => {
                            debug!("Notification stream closed by server");
                            break;
                        }
                        Err(e) => {
                            error!("Error receiving notification: {}", e);
                            break;
                        }
                    }
                }
            }
        }
    }

    /// The main sync loop. Runs in an event loop to ensure no updates happen in parallel.
    fn start_sync_loop_task(
        self: &Arc<Self>,
        shutdown_receiver: watch::Receiver<()>,
        pull_trigger: watch::Receiver<()>,
    ) {
        let clone = Arc::clone(self);
        tokio::spawn(async move { clone.sync_loop(shutdown_receiver, pull_trigger).await });
    }

    async fn sync_loop(
        &self,
        mut shutdown_receiver: watch::Receiver<()>,
        mut pull_trigger: watch::Receiver<()>,
    ) {
        let mut push_trigger = self.push_sync_trigger.resubscribe();
        let (backoff_trigger_tx, mut backoff_trigger_rx) = mpsc::channel::<Duration>(10);

        // Mutex to ensure there is only one backoff running at a time.
        let backoff_handle: Mutex<Option<BackoffHandle>> = Mutex::new(None);

        loop {
            let (incoming_count, outgoing_count) = tokio::select! {
                _ = shutdown_receiver.changed() => {
                    debug!("Shutdown signal received, stopping push sync loop");
                    break;
                }
                _ = pull_trigger.changed() => {
                    debug!("Received incoming sync notification");
                    match self.pull_sync_once().await {
                        Ok(count) => (count, None),
                        Err(e) => {
                            error!("Failed to sync once: {}", e);
                            (None, None)
                        }
                    }
                }
                Some(last_backoff) = backoff_trigger_rx.recv() => {
                    self.handle_backoff(
                        &backoff_handle,
                        &mut backoff_trigger_rx,
                        &backoff_trigger_tx,
                        last_backoff
                    ).await
                }
                result = push_trigger.recv() => {
                    self.handle_push(
                        result,
                        &pull_trigger,
                        &backoff_handle,
                        &backoff_trigger_tx
                    ).await
                }
            };

            if (incoming_count.is_some() || outgoing_count.is_some())
                && let Err(e) = self
                    .new_record_handler
                    .on_sync_completed(incoming_count, outgoing_count)
                    .await
            {
                error!("Failed to notify of real-time sync completion: {e:?}");
            }
        }
    }

    async fn handle_backoff(
        &self,
        backoff_handle: &Mutex<Option<BackoffHandle>>,
        backoff_trigger_rx: &mut mpsc::Receiver<Duration>,
        backoff_trigger_tx: &mpsc::Sender<Duration>,
        mut last_backoff: Duration,
    ) -> (Option<u32>, Option<u32>) {
        // Clear the backoff queue to avoid piling up requests.
        while let Ok(new_last_backoff) = backoff_trigger_rx.try_recv() {
            last_backoff = last_backoff.min(new_last_backoff);
        }
        debug!("Backoff trigger received, waiting before next sync attempt");
        let incoming_count = match self.pull_sync_once().await {
            Ok(count) => count,
            Err(e) => {
                error!("Failed to pull sync once in backoff mode: {}", e);
                None
            }
        };
        let outgoing_count = match self.push_sync_once().await {
            Ok(count) => count,
            Err(e) => {
                error!("Failed to push sync once in backoff mode: {}", e);
                self.schedule_backoff(
                    backoff_handle,
                    backoff_trigger_tx,
                    last_backoff.mul_f32(1.5),
                )
                .await;
                return (None, None);
            }
        };
        debug!("Backoff sync attempt succeeded, resuming normal operation");
        (incoming_count, outgoing_count)
    }

    async fn handle_push(
        &self,
        result: Result<RecordId, broadcast::error::RecvError>,
        pull_trigger: &watch::Receiver<()>,
        backoff_handle: &Mutex<Option<BackoffHandle>>,
        backoff_trigger_tx: &mpsc::Sender<Duration>,
    ) -> (Option<u32>, Option<u32>) {
        match result {
            Ok(record_id) => {
                debug!("Received sync trigger for record id {:?}", record_id);
                let incoming_count = if pull_trigger.has_changed().unwrap_or(false) {
                    match self.pull_sync_once().await {
                        Ok(count) => count,
                        Err(e) => {
                            error!("Failed to sync once: {}", e);
                            None
                        }
                    }
                } else {
                    None
                };
                let outgoing_count = match self.push_sync_once().await {
                    Ok(count) => count,
                    Err(e) => {
                        error!("Failed to sync once: {}", e);
                        self.schedule_backoff(
                            backoff_handle,
                            backoff_trigger_tx,
                            Duration::from_secs(1),
                        )
                        .await;
                        return (None, None);
                    }
                };
                debug!("Push sync attempt succeeded");
                (incoming_count, outgoing_count)
            }
            Err(broadcast::error::RecvError::Closed) => {
                debug!("Push sync trigger channel closed, stopping push sync loop");
                (None, None)
            }
            Err(broadcast::error::RecvError::Lagged(count)) => {
                warn!("Lagged {} messages in push sync trigger channel", count);
                (None, None)
            }
        }
    }

    async fn schedule_backoff(
        &self,
        backoff_handle: &Mutex<Option<BackoffHandle>>,
        backoff_trigger_tx: &mpsc::Sender<Duration>,
        duration: Duration,
    ) {
        let now = SystemTime::now();
        let mut backoff_handle = backoff_handle.lock().await;
        if let Some(existing) = &*backoff_handle {
            let elapsed = now.duration_since(existing.started_at).unwrap_or_default();
            let remaining = existing.duration.saturating_sub(elapsed);
            if remaining < duration {
                debug!(
                    "Existing backoff of {:?} still in effect (remaining {:?}), not scheduling new backoff of {:?}",
                    existing.duration, remaining, duration
                );
                return;
            }
            existing.handle.abort();
            debug!(
                "New backoff of {:?} is shorter than existing backoff of {:?} (remaining {:?}), replacing it",
                duration, existing.duration, remaining
            );
        }

        debug!("Scheduling backoff trigger in {:?}", duration);
        let new_handle = tokio::spawn({
            let backoff_trigger_tx = backoff_trigger_tx.clone();
            async move {
                tokio::time::sleep(duration).await;
                if let Err(e) = backoff_trigger_tx.send(duration).await {
                    error!("Failed to send backoff trigger: {}", e);
                }
            }
        });
        *backoff_handle = Some(BackoffHandle {
            started_at: now,
            duration,
            handle: new_handle,
        });
    }

    async fn push_sync_once(&self) -> anyhow::Result<Option<u32>> {
        debug!("Push syncing once");

        let mut count: u32 = 0;
        while let changes = self
            .storage
            .get_pending_outgoing_changes(SYNC_BATCH_SIZE)
            .await?
            && !changes.is_empty()
        {
            let current_count = self.push_sync_batch(changes).await?;
            count = count.saturating_add(current_count);
        }

        Ok(match count {
            0 => None,
            other => Some(other),
        })
    }

    async fn push_sync_batch(
        &self,
        changes: Vec<crate::sync::storage::OutgoingChange>,
    ) -> anyhow::Result<u32> {
        debug!(
            "Processing sync batch of {} outgoing changes",
            changes.len()
        );

        let mut count: u32 = 0;
        for storage_change in changes {
            let change = storage_change.try_into()?;
            self.push_sync_record(change).await?;
            count = count.saturating_add(1);
        }

        Ok(count)
    }

    async fn push_sync_record(&self, change: OutgoingChange) -> anyhow::Result<()> {
        // Merges the updated fields with the existing record data in the local sync state to form the new record.
        let record = change.merge();

        debug!(
            "Pushing outgoing record {:?}, revision {} to remote",
            record.id, record.revision
        );
        // Pushes the record to the remote server.
        // TODO: If the remote server already has this exact revision, check what happens. We should continue then for idempotency.
        self.client.set_record(&record).await?;

        debug!(
            "Completing outgoing record {:?}, revision {}",
            record.id, record.revision
        );
        // Removes the pending outgoing record and updates the existing record with the new one.
        self.storage
            .complete_outgoing_sync((&record).try_into()?)
            .await?;
        Ok(())
    }

    async fn pull_sync_once(&self) -> anyhow::Result<Option<u32>> {
        debug!("Pull syncing once");

        let since_revision = self.storage.get_last_revision().await?;

        let mut records = self.client.list_changes(since_revision).await?;

        debug!(
            "real-time sync list_changes since {} yielded {} results.",
            since_revision,
            records.len()
        );
        records.sort_by(|a, b| a.revision.cmp(&b.revision));
        let db_records = records
            .iter()
            .map(crate::sync::storage::Record::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        if !records.is_empty() {
            self.storage.insert_incoming_records(db_records).await?;
        }

        let count = self.pull_sync_once_local().await?;

        // NOTE: Might return Some(0) if pull was successful without pulling records.
        Ok(Some(count))
    }

    async fn pull_sync_once_local(&self) -> anyhow::Result<u32> {
        let mut count: u32 = 0;

        loop {
            let incoming_records = self.storage.get_incoming_records(SYNC_BATCH_SIZE).await?;
            if incoming_records.is_empty() {
                break;
            }

            let current_count = u32::try_from(incoming_records.len())?;
            debug!("Processing {} incoming records", current_count);
            for incoming_record in incoming_records {
                // TODO: Ensure the incoming change revision number is correct according to our assumptions. But also... allow replays.

                // NOTE: The incoming record will have the same revision number as a pending outgoing record (if any).
                // A rebase now means just updating the pending outgoing records to have a higher revision number.
                // If data becomes more complex, we might need to do a proper rebase with conflict resolution here.
                debug!(
                    "Rebasing pending outgoing records to above revision {}",
                    incoming_record.new_state.revision
                );
                self.storage
                    .rebase_pending_outgoing_records(incoming_record.new_state.revision)
                    .await?;

                // First update the sync state from the incoming record. The sync state will have to change anyway,
                // there is no going back if there is a remote change. We don't remove the incoming record yet,
                // to ensure we'll update the relational database state if we turn off now.
                debug!(
                    "Updating sync state from incoming record {:?}, revision {}",
                    incoming_record.new_state.id, incoming_record.new_state.revision
                );
                self.storage
                    .update_record_from_incoming(incoming_record.new_state.clone())
                    .await?;

                // Now notify the relational database to update. Wait for it to be done. Note that this could be improved
                // in the future to also add actions to the pending outgoing changes for the same record. Like maybe delete
                // an action, or change its field values. Now it is not necessary yet, because there are only immutable
                // changes.
                debug!(
                    "Invoking relational database callback for incoming record {:?}, revision {}",
                    incoming_record.new_state.id, incoming_record.new_state.revision
                );
                self.new_record_handler
                    .on_incoming_change((&incoming_record).try_into()?)
                    .await?;

                debug!(
                    "Removing incoming record after processing completion {:?}, revision {}",
                    incoming_record.new_state.id, incoming_record.new_state.revision
                );

                // Now it's safe to delete the incoming record.
                self.storage
                    .delete_incoming_record(incoming_record.new_state)
                    .await?;
            }

            count = count.saturating_add(current_count);
        }

        Ok(count)
    }
}

/// Used for sync backoff timer management
struct BackoffHandle {
    started_at: SystemTime,
    duration: Duration,
    handle: tokio::task::JoinHandle<()>,
}

#[cfg(test)]
mod tests {
    use crate::sync::background::SYNC_BATCH_SIZE;
    use crate::sync::proto::SetRecordReply;
    use crate::sync::storage::{self, MockSyncStorage};
    use crate::sync::{
        MockNewRecordHandler, MockSyncSigner, MockSyncerClient, RecordId, SigningClient,
        SyncProcessor,
    };

    use anyhow::anyhow;
    use mockall::predicate::eq;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    use tokio::sync::{broadcast, watch};
    use tokio_with_wasm::alias as tokio;

    // Helper function to create test records
    fn create_record(id_type: &str, id_data: &str, revision: u64) -> crate::sync::storage::Record {
        crate::sync::storage::Record {
            id: RecordId::new(id_type, id_data),
            revision,
            schema_version: "0.2.6".to_string(),
            data: HashMap::new(),
        }
    }

    // Helper function to create test outgoing changes
    fn create_outgoing_change(
        id_type: &str,
        id_data: &str,
        revision: u64,
    ) -> crate::sync::storage::OutgoingChange {
        let change = crate::sync::storage::RecordChange {
            id: RecordId::new(id_type, id_data),
            schema_version: "0.2.6".to_string(),
            updated_fields: HashMap::new(),
            revision,
        };

        crate::sync::storage::OutgoingChange {
            change,
            parent: None,
        }
    }

    // Helper to create a SigningClient with mocks
    fn create_signing_client(
        client: MockSyncerClient,
        mut signer: MockSyncSigner,
    ) -> SigningClient {
        // Setup default expectations for the signer methods with .returning() to handle any number of calls

        // For sign_ecdsa_recoverable, which is used in sign_message for all client requests
        // The method needs to return a byte array that will be zbase32 encoded
        signer
            .expect_sign_ecdsa_recoverable()
            .returning(|_| Ok(vec![0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f])); // Return dummy signature

        // For ecies_encrypt, which is used when encrypting record data in set_record
        // This should take the input data and return an "encrypted" version
        signer.expect_ecies_encrypt().returning(|data| {
            // In a real implementation, this would encrypt the data
            // For testing, we'll just prepend a marker to simulate encryption
            let mut encrypted = vec![0xE5, 0xE5]; // "Encryption" marker
            encrypted.extend_from_slice(&data);
            Ok(encrypted)
        });

        // For ecies_decrypt, which is used when decrypting record data in map_record
        // This needs to return a valid SyncData JSON that can be deserialized
        signer.expect_ecies_decrypt().returning(|data| {
            // In tests with empty data or if it starts with our marker
            if data.is_empty() || (data.len() >= 2 && data[0] == 0xE5 && data[1] == 0xE5) {
                // Return a valid SyncData JSON that can be parsed
                let sync_data = r#"{"id":{"type":"test","data_id":"123"},"data":{}}"#;
                Ok(sync_data.as_bytes().to_vec())
            } else {
                // For other data, just pass through (assume it's already valid JSON)
                Ok(data)
            }
        });

        SigningClient::new(
            Arc::new(client),
            Arc::new(signer),
            "test-client-id".to_string(),
        )
    }

    #[macros::async_test_all]
    async fn test_ensure_outgoing_record_committed_no_record() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        mock_storage
            .expect_get_latest_outgoing_change()
            .times(1)
            .returning(|| Ok(None));

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let client = create_signing_client(MockSyncerClient::new(), MockSyncSigner::new());

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.ensure_outgoing_record_committed().await;

        // Verify
        assert!(result.is_ok());
    }

    #[macros::async_test_all]
    async fn test_ensure_outgoing_record_committed_with_record() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        let test_change = create_outgoing_change("test", "123", 1);

        mock_storage
            .expect_get_latest_outgoing_change()
            .times(1)
            .returning(move || Ok(Some(test_change.clone())));

        let mut mock_handler = MockNewRecordHandler::new();
        mock_handler
            .expect_on_replay_outgoing_change()
            .times(1)
            .returning(|_| Ok(()));

        let (_tx, rx) = broadcast::channel(10);
        let client = create_signing_client(MockSyncerClient::new(), MockSyncSigner::new());

        let sync_processor =
            SyncProcessor::new(client, rx, Arc::new(mock_handler), Arc::new(mock_storage));

        // Execute
        let result = sync_processor.ensure_outgoing_record_committed().await;

        // Verify
        assert!(result.is_ok());
    }

    #[macros::async_test_all]
    async fn test_ensure_outgoing_record_committed_handler_failure() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        let test_change = create_outgoing_change("test", "123", 1);

        mock_storage
            .expect_get_latest_outgoing_change()
            .times(1)
            .returning(move || Ok(Some(test_change.clone())));

        let mut mock_handler = MockNewRecordHandler::new();
        mock_handler
            .expect_on_replay_outgoing_change()
            .times(1)
            .returning(|_| Err(anyhow!("Handler error")));

        let (_tx, rx) = broadcast::channel(10);
        let client = create_signing_client(MockSyncerClient::new(), MockSyncSigner::new());

        let sync_processor =
            SyncProcessor::new(client, rx, Arc::new(mock_handler), Arc::new(mock_storage));

        // Execute
        let result = sync_processor.ensure_outgoing_record_committed().await;

        // Verify
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Handler error");
    }

    #[macros::async_test_all]
    async fn test_push_sync_once_no_pending_changes() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        mock_storage
            .expect_get_pending_outgoing_changes()
            .times(1)
            .with(eq(SYNC_BATCH_SIZE))
            .returning(|_| Ok(Vec::new()));

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let client = create_signing_client(MockSyncerClient::new(), MockSyncSigner::new());

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.push_sync_once().await;

        // Verify
        assert!(result.is_ok());
    }

    #[macros::async_test_all]
    async fn test_push_sync_once_with_pending_changes() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        let test_change = create_outgoing_change("test", "123", 1);

        // First call returns one change, second call returns empty
        mock_storage
            .expect_get_pending_outgoing_changes()
            .times(2)
            .with(eq(SYNC_BATCH_SIZE))
            .returning(move |_| {
                static COUNTER: AtomicU64 = AtomicU64::new(0);
                if COUNTER.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(vec![test_change.clone()])
                } else {
                    Ok(Vec::new())
                }
            });

        let mut mock_client = MockSyncerClient::new();
        mock_client.expect_set_record().times(1).returning(|_| {
            Ok(crate::sync::proto::SetRecordReply {
                status: crate::sync::proto::SetRecordStatus::Success as i32,
                new_revision: 1,
            })
        });

        mock_storage
            .expect_complete_outgoing_sync()
            .times(1)
            .returning(|_| Ok(()));

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let client = create_signing_client(mock_client, MockSyncSigner::new());

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.push_sync_once().await;

        // Verify
        assert!(result.is_ok());
    }

    #[macros::async_test_all]
    async fn test_push_sync_once_client_failure() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        let test_change = create_outgoing_change("test", "123", 1);

        mock_storage
            .expect_get_pending_outgoing_changes()
            .times(1)
            .with(eq(SYNC_BATCH_SIZE))
            .returning(move |_| Ok(vec![test_change.clone()]));

        let mut mock_client = MockSyncerClient::new();
        mock_client
            .expect_set_record()
            .times(1)
            .returning(|_| Err(anyhow!("Network error")));

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let client = create_signing_client(mock_client, MockSyncSigner::new());

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.push_sync_once().await;

        // Verify
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Network error");
    }

    #[macros::async_test_all]
    async fn test_pull_sync_once_no_changes() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        mock_storage
            .expect_get_last_revision()
            .times(1)
            .returning(|| Ok(5));

        let mut mock_client = MockSyncerClient::new();
        mock_client.expect_list_changes().times(1).returning(|_| {
            Ok(crate::sync::proto::ListChangesReply {
                changes: Vec::new(),
            })
        });

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(SYNC_BATCH_SIZE))
            .returning(|_| Ok(Vec::new()));

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let client = create_signing_client(mock_client, MockSyncSigner::new());

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.pull_sync_once().await;

        // Verify
        assert!(result.is_ok());
    }

    #[macros::async_test_all]
    async fn test_pull_sync_once_with_changes() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        mock_storage
            .expect_get_last_revision()
            .times(1)
            .returning(|| Ok(5));

        // Create a dummy proto record
        let proto_record = crate::sync::proto::Record {
            id: "test:123".to_string(),
            revision: 6,
            schema_version: "0.2.6".to_string(),
            data: Vec::new(),
        };

        let mut mock_client = MockSyncerClient::new();
        mock_client
            .expect_list_changes()
            .times(1)
            .returning(move |_| {
                Ok(crate::sync::proto::ListChangesReply {
                    changes: vec![proto_record.clone()],
                })
            });

        // Simulate successful decryption in the signer - explicitly setting expectation
        let mut mock_signer = MockSyncSigner::new();
        mock_signer
            .expect_sign_ecdsa_recoverable()
            .returning(|_| Ok(vec![0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f]));

        mock_signer
            .expect_ecies_decrypt()
            .times(1) // Exactly one call expected for this test
            .returning(|_| {
                // Create a valid JSON for SyncData
                let sync_data = r#"{"id":{"type":"test","data_id":"123"},"data":{}}"#;
                Ok(sync_data.as_bytes().to_vec())
            });

        mock_storage
            .expect_insert_incoming_records()
            .times(1)
            .returning(|_| Ok(()));

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(SYNC_BATCH_SIZE))
            .returning(|_| Ok(Vec::new()));

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let client = create_signing_client(mock_client, mock_signer);

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.pull_sync_once().await;

        // Verify
        assert!(result.is_ok());
    }

    #[macros::async_test_all]
    async fn test_pull_sync_once_local_with_incoming_changes() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();

        // Create test records
        let incoming_record = crate::sync::storage::IncomingChange {
            new_state: create_record("test", "123", 6),
            old_state: Some(create_record("test", "123", 5)),
        };

        // First call returns record, second returns empty
        mock_storage
            .expect_get_incoming_records()
            .times(2)
            .with(eq(SYNC_BATCH_SIZE))
            .returning(move |_| {
                static COUNTER: AtomicU64 = AtomicU64::new(0);
                if COUNTER.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(vec![incoming_record.clone()])
                } else {
                    Ok(Vec::new())
                }
            });

        mock_storage
            .expect_rebase_pending_outgoing_records()
            .times(1)
            .with(eq(6))
            .returning(|_| Ok(()));

        mock_storage
            .expect_update_record_from_incoming()
            .times(1)
            .returning(|_| Ok(()));

        let mut mock_handler = MockNewRecordHandler::new();
        mock_handler
            .expect_on_incoming_change()
            .times(1)
            .returning(|_| Ok(()));

        mock_storage
            .expect_delete_incoming_record()
            .times(1)
            .returning(|_| Ok(()));

        let (_tx, rx) = broadcast::channel(10);
        let client = create_signing_client(MockSyncerClient::new(), MockSyncSigner::new());

        let sync_processor =
            SyncProcessor::new(client, rx, Arc::new(mock_handler), Arc::new(mock_storage));

        // Execute
        let result = sync_processor.pull_sync_once_local().await;

        // Verify
        assert!(result.is_ok());
    }

    #[macros::async_test_all]
    async fn test_pull_sync_once_local_handler_error() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();

        // Create test records
        let incoming_record = crate::sync::storage::IncomingChange {
            new_state: create_record("test", "123", 6),
            old_state: Some(create_record("test", "123", 5)),
        };

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(SYNC_BATCH_SIZE))
            .returning(move |_| Ok(vec![incoming_record.clone()]));

        mock_storage
            .expect_rebase_pending_outgoing_records()
            .times(1)
            .with(eq(6))
            .returning(|_| Ok(()));

        mock_storage
            .expect_update_record_from_incoming()
            .times(1)
            .returning(|_| Ok(()));

        let mut mock_handler = MockNewRecordHandler::new();
        mock_handler
            .expect_on_incoming_change()
            .times(1)
            .returning(|_| Err(anyhow!("Handler error")));

        // No delete call because the handler failed

        let (_tx, rx) = broadcast::channel(10);
        let client = create_signing_client(MockSyncerClient::new(), MockSyncSigner::new());

        let sync_processor =
            SyncProcessor::new(client, rx, Arc::new(mock_handler), Arc::new(mock_storage));

        // Execute
        let result = sync_processor.pull_sync_once_local().await;

        // Verify
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Handler error");
    }

    #[macros::async_test_all]
    async fn test_start_includes_all_initialization_steps() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();

        // For ensure_outgoing_record_committed
        mock_storage
            .expect_get_latest_outgoing_change()
            .times(1)
            .returning(|| Ok(None));

        // For pull_sync_once_local
        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(SYNC_BATCH_SIZE))
            .returning(|_| Ok(Vec::new()));

        let (_tx, rx) = broadcast::channel::<RecordId>(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let client = create_signing_client(MockSyncerClient::new(), MockSyncSigner::new());

        let sync_processor = Arc::new(SyncProcessor::new(
            client,
            rx,
            mock_handler,
            Arc::new(mock_storage),
        ));

        let (shutdown_tx, shutdown_rx) = watch::channel(());

        // Execute
        let result = sync_processor.start(shutdown_rx).await;

        // Verify
        assert!(result.is_ok());

        // Allow background tasks to run briefly
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Send shutdown signal to clean up
        let _ = shutdown_tx.send(());
    }

    #[macros::async_test_all]
    async fn test_sync_loop_handles_push_trigger() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();

        // For push_sync_once
        mock_storage
            .expect_get_pending_outgoing_changes()
            .times(1)
            .returning(|_| {
                Ok(vec![storage::OutgoingChange {
                    parent: None,
                    change: storage::RecordChange {
                        id: RecordId {
                            r#type: "test".to_string(),
                            data_id: "123".to_string(),
                        },
                        schema_version: "1.0.0".to_string(),
                        updated_fields: [("field".to_string(), "\"value\"".to_string())].into(),
                        revision: 1,
                    },
                }])
            });

        let (tx, rx) = broadcast::channel::<RecordId>(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let mut syncer_client = MockSyncerClient::new();
        syncer_client.expect_set_record().times(1).returning(|_| {
            Ok(SetRecordReply {
                ..Default::default()
            })
        });
        let client = create_signing_client(syncer_client, MockSyncSigner::new());

        let sync_processor = Arc::new(SyncProcessor::new(
            client,
            rx,
            mock_handler,
            Arc::new(mock_storage),
        ));

        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let (_pull_trigger_sender, pull_trigger) = watch::channel(());

        // Start the sync loop in a separate task
        let sync_processor_clone = sync_processor.clone();
        let task_handle = tokio::spawn(async move {
            sync_processor_clone
                .sync_loop(shutdown_rx, pull_trigger)
                .await;
        });

        // Wait a bit to ensure the loop is listening to changes
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Send a push trigger
        tx.send(RecordId::new("test", "123")).unwrap();

        // Allow time for processing
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Shutdown and cleanup
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_millis(500), task_handle).await;
    }

    #[macros::async_test_all]
    async fn test_sync_loop_handles_pull_trigger() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();

        // For pull_sync_once
        mock_storage
            .expect_get_last_revision()
            .times(1)
            .returning(|| Ok(5));

        let mut mock_client = MockSyncerClient::new();
        mock_client.expect_list_changes().times(1).returning(|_| {
            Ok(crate::sync::proto::ListChangesReply {
                changes: Vec::new(),
            })
        });

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .returning(|_| Ok(Vec::new()));

        let (_tx, rx) = broadcast::channel::<RecordId>(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let client = create_signing_client(mock_client, MockSyncSigner::new());

        let sync_processor = Arc::new(SyncProcessor::new(
            client,
            rx,
            mock_handler,
            Arc::new(mock_storage),
        ));

        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let (pull_tx, pull_trigger) = watch::channel(());

        // Start the sync loop in a separate task
        let sync_processor_clone = sync_processor.clone();
        let task_handle = tokio::spawn(async move {
            sync_processor_clone
                .sync_loop(shutdown_rx, pull_trigger)
                .await;
        });

        // Send a pull trigger
        let _ = pull_tx.send(());

        // Allow time for processing
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Shutdown and cleanup
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_millis(500), task_handle).await;
    }

    #[macros::async_test_all]
    async fn test_sync_signer_encryption_failure() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        let test_change = create_outgoing_change("test", "123", 1);

        mock_storage
            .expect_get_pending_outgoing_changes()
            .times(1)
            .returning(move |_| Ok(vec![test_change.clone()]));

        let mock_client = MockSyncerClient::new();

        // Create mock signer that fails on encryption
        let mut mock_signer = MockSyncSigner::new();
        mock_signer
            .expect_ecies_encrypt()
            .times(1)
            .returning(|_| Err(anyhow!("Encryption failure")));

        mock_signer
            .expect_sign_ecdsa_recoverable()
            .returning(|_| Ok(vec![0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f]));

        let client = create_signing_client(mock_client, mock_signer);

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.push_sync_once().await;

        // Verify
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Encryption failure");
    }

    #[macros::async_test_all]
    async fn test_sync_signer_decryption_failure() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        mock_storage
            .expect_get_last_revision()
            .times(1)
            .returning(|| Ok(5));

        // Create a dummy proto record
        let proto_record = crate::sync::proto::Record {
            id: "test:123".to_string(),
            revision: 6,
            schema_version: "0.2.6".to_string(),
            data: vec![1, 2, 3, 4], // Some non-empty data
        };

        let mut mock_client = MockSyncerClient::new();
        mock_client
            .expect_list_changes()
            .times(1)
            .returning(move |_| {
                Ok(crate::sync::proto::ListChangesReply {
                    changes: vec![proto_record.clone()],
                })
            });

        // Simulate decryption failure in the signer
        let mut mock_signer = MockSyncSigner::new();
        mock_signer
            .expect_sign_ecdsa_recoverable()
            .returning(|_| Ok(vec![0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f]));

        mock_signer
            .expect_ecies_decrypt()
            .times(1)
            .returning(|_| Err(anyhow!("Decryption failure")));

        let client = create_signing_client(mock_client, mock_signer);

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.pull_sync_once().await;

        // Verify
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Decryption failure");
    }

    #[macros::async_test_all]
    async fn test_sync_signer_invalid_json_after_decryption() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        mock_storage
            .expect_get_last_revision()
            .times(1)
            .returning(|| Ok(5));

        // Create a dummy proto record
        let proto_record = crate::sync::proto::Record {
            id: "test:123".to_string(),
            revision: 6,
            schema_version: "0.2.6".to_string(),
            data: vec![1, 2, 3, 4], // Some non-empty data
        };

        let mut mock_client = MockSyncerClient::new();
        mock_client
            .expect_list_changes()
            .times(1)
            .returning(move |_| {
                Ok(crate::sync::proto::ListChangesReply {
                    changes: vec![proto_record.clone()],
                })
            });

        // Simulate successful decryption but with invalid JSON
        let mut mock_signer = MockSyncSigner::new();
        mock_signer
            .expect_sign_ecdsa_recoverable()
            .returning(|_| Ok(vec![0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f]));

        mock_signer.expect_ecies_decrypt().times(1).returning(|_| {
            // Return invalid JSON that will fail to parse as SyncData
            let invalid_json = r#"{"not_valid_sync_data": true}"#;
            Ok(invalid_json.as_bytes().to_vec())
        });

        let client = create_signing_client(mock_client, mock_signer);

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.pull_sync_once().await;

        // Verify
        assert!(result.is_err());
        // The error should be related to JSON deserialization
        assert!(result.unwrap_err().to_string().contains("missing field"));
    }

    #[macros::async_test_all]
    async fn test_signing_failure() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        let test_change = create_outgoing_change("test", "123", 1);

        mock_storage
            .expect_get_pending_outgoing_changes()
            .times(1)
            .returning(move |_| Ok(vec![test_change.clone()]));

        let mock_client = MockSyncerClient::new();

        // Create mock signer that fails on signing
        let mut mock_signer = MockSyncSigner::new();
        mock_signer.expect_ecies_encrypt().times(1).returning(Ok);

        mock_signer
            .expect_sign_ecdsa_recoverable()
            .times(1)
            .returning(|_| Err(anyhow!("Signing failure")));

        let client = create_signing_client(mock_client, mock_signer);

        let (_tx, rx) = broadcast::channel(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());

        let sync_processor = SyncProcessor::new(client, rx, mock_handler, Arc::new(mock_storage));

        // Execute
        let result = sync_processor.push_sync_once().await;

        // Verify
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Signing failure");
    }
}
