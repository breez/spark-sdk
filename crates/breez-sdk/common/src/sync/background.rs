use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, warn};
use web_time::SystemTime;

use crate::sync::{
    model::{IncomingChange, OutgoingChange, RecordId},
    signing_client::SigningClient,
    storage::SyncStorage,
};

#[allow(clippy::ref_option)]
#[cfg_attr(test, mockall::automock)]
#[macros::async_trait]
pub trait NewRecordHandler: Send + Sync {
    async fn on_incoming_change(&self, change: IncomingChange) -> anyhow::Result<RecordOutcome>;
    async fn on_replay_outgoing_change(&self, change: OutgoingChange) -> anyhow::Result<()>;
    async fn on_sync_completed(
        &self,
        incoming_count: Option<u32>,
        outgoing_count: Option<u32>,
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum RecordOutcome {
    Completed,
    Deferred,
}

/// Handles background synchronization with the data-sync service.
///
/// # Shared Storage Incompatibility
///
/// **WARNING**: Real-time sync is incompatible with shared storage deployments.
/// This implementation assumes each `SyncProcessor` has exclusive access to its
/// underlying storage. When multiple SDK instances share the same storage backend
/// (e.g., replicas of the same wallet connecting to a shared database), concurrent
/// processing of incoming records causes race conditions that can corrupt revision
/// tracking. These races span multiple storage operations,
/// so they cannot be fixed with simple database transactions.
///
/// Example: one instance may advance the sync cursor and update `sync_state` for an
/// incoming record while another instance is reading pending state for push decisions.
/// Because those steps span multiple storage operations, interleavings can leave each
/// instance with a different view of what is safe to push.
///
/// For server deployments with shared storage, disable rtsync and rely on
/// direct storage coordination instead.
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
        // Errors are non-fatal: pull_sync_once_local will be retried on the next iteration
        // of the sync loop, so transient failures self-heal.
        if let Err(e) = self.pull_sync_once_local().await {
            error!("Failed to process pending incoming records during real-time sync startup: {e}");
        }

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
            "Committing latest pending outgoing change for record {:?}, local queue id {}",
            record.change.id, record.change.local_revision
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
                self.schedule_backoff(
                    backoff_handle,
                    backoff_trigger_tx,
                    last_backoff.mul_f32(1.5),
                )
                .await;
                return (None, None);
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
                            self.schedule_backoff(
                                backoff_handle,
                                backoff_trigger_tx,
                                Duration::from_secs(1),
                            )
                            .await;
                            return (None, None);
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
            if remaining != Duration::ZERO && remaining < duration {
                debug!(
                    "Existing backoff of {:?} still in effect (remaining {:?}), not scheduling new backoff of {:?}",
                    existing.duration, remaining, duration
                );
                return;
            }
            existing.handle.abort();
            debug!(
                "New backoff of {:?} is better than existing backoff of {:?} (remaining {:?}), replacing it",
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

        let changes = self.storage.get_pending_outgoing_changes(u32::MAX).await?;
        if changes.is_empty() {
            return Ok(None);
        }
        let count = self.push_sync_batch(changes).await?;

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

        let incoming = self.storage.get_incoming_records(u32::MAX).await?;
        let pending_ids: HashSet<RecordId> = incoming.into_iter().map(|r| r.new_state.id).collect();

        let mut pushed_count: u32 = 0;
        for storage_change in changes {
            if pending_ids.contains(&storage_change.change.id) {
                debug!(
                    "Deferring outgoing push for record {:?} — pending incoming change exists",
                    storage_change.change.id
                );
                continue;
            }
            let change: OutgoingChange = storage_change.try_into()?;
            self.push_sync_record(change).await?;
            pushed_count = pushed_count.saturating_add(1);
        }

        Ok(pushed_count)
    }

    async fn push_sync_record(&self, change: OutgoingChange) -> anyhow::Result<()> {
        let local_revision = change.change.local_revision;
        let mut record = change.merge();
        debug!(
            "Pushing outgoing record {:?} to remote with parent revision {} (local queue id {})",
            record.id, record.revision, local_revision
        );
        // Pushes the record to the remote server.
        // TODO: If the remote server already has this exact revision, check what happens. We should continue then for idempotency.
        let reply = self.client.set_record(&record).await?;

        match reply.status() {
            crate::sync::proto::SetRecordStatus::Success => {
                debug!(
                    "Successfully pushed outgoing change with id {} for type {} (local queue id {}). Server returned revision {}.",
                    &record.id.data_id, &record.id.r#type, local_revision, reply.new_revision,
                );
            }
            crate::sync::proto::SetRecordStatus::Conflict => {
                warn!(
                    "Got conflict when trying to push outgoing change with id {} for type {}",
                    &record.id.data_id, &record.id.r#type
                );
                anyhow::bail!("Conflict when trying to push outgoing change");
            }
        }

        record.revision = reply.new_revision;

        // Removes the pending outgoing record and updates the existing record with the new one.
        self.storage
            .complete_outgoing_sync((&record).try_into()?, local_revision)
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
        // Fetch all pending incoming records at once. Records are small, so batching is likely
        // unnecessary. Batching would require cursor-based pagination (offset-based
        // breaks due to mid-iteration deletes).
        let incoming_records = self.storage.get_incoming_records(u32::MAX).await?;
        let total_count = u32::try_from(incoming_records.len())?;
        let mut incoming_applied: u32 = 0;
        let mut incoming_deferred: u32 = 0;
        let mut incoming_errored: u32 = 0;

        if total_count > 0 {
            debug!("Processing {} incoming records", total_count);
        }

        for incoming_record in incoming_records {
            // TODO: Ensure the incoming change revision number is correct according to our assumptions. But also... allow replays.
            let incoming_revision = incoming_record.new_state.revision;
            let existing_revision = incoming_record
                .old_state
                .as_ref()
                .map(|state| state.revision);

            // Update sync state if we haven't already. We don't remove the incoming record yet,
            // to ensure we'll update the relational database state if we turn off now.
            // We should do this even if we later fail to apply the change (e.g. due to a schema change)
            // to avoid refetching the same record later.
            let first_time_seen = existing_revision.is_none_or(|old| old < incoming_revision);
            if first_time_seen {
                debug!(
                    "Updating sync state from incoming record {:?}, revision {}",
                    incoming_record.new_state.id, incoming_revision
                );
                self.storage
                    .update_record_from_incoming(incoming_record.new_state.clone())
                    .await?;
            }

            // Now notify the relational database to update. Wait for it to be done. Note that this could be improved
            // in the future to also add actions to the pending outgoing changes for the same record. Like maybe delete
            // an action, or change its field values. Now it is not necessary yet, because there are only immutable
            // changes.
            debug!(
                "Invoking relational database callback for incoming record {:?}, revision {}",
                incoming_record.new_state.id, incoming_revision
            );
            match self
                .new_record_handler
                .on_incoming_change((&incoming_record).try_into()?)
                .await
            {
                Ok(RecordOutcome::Completed) => {
                    debug!(
                        "Removing incoming record after processing completion {:?}, revision {}",
                        incoming_record.new_state.id, incoming_revision
                    );
                    self.storage
                        .delete_incoming_record(incoming_record.new_state)
                        .await?;
                    incoming_applied = incoming_applied.saturating_add(1);
                }
                Ok(RecordOutcome::Deferred) => {
                    incoming_deferred = incoming_deferred.saturating_add(1);
                }
                Err(err) => {
                    incoming_errored = incoming_errored.saturating_add(1);
                    error!(
                        "Failed to apply incoming record {:?}, revision {}. Keeping record for retry: {}",
                        incoming_record.new_state.id, incoming_revision, err
                    );
                }
            }
        }

        debug!(
            incoming_total = total_count,
            incoming_applied,
            incoming_deferred,
            incoming_errored,
            "Incoming real-time sync processing summary"
        );
        Ok(incoming_applied)
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
    use crate::sync::proto::SetRecordReply;
    use crate::sync::storage::{self, MockSyncStorage};
    use crate::sync::{
        MockNewRecordHandler, MockSyncSigner, MockSyncerClient, RecordId, RecordOutcome,
        SigningClient, SyncProcessor,
    };

    use anyhow::anyhow;
    use mockall::{Sequence, predicate::eq};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{Mutex, broadcast, mpsc, watch};
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

    fn create_record_with_schema(
        id_type: &str,
        id_data: &str,
        revision: u64,
        schema_version: &str,
    ) -> crate::sync::storage::Record {
        crate::sync::storage::Record {
            id: RecordId::new(id_type, id_data),
            revision,
            schema_version: schema_version.to_string(),
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
            local_revision: revision,
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

        // For encrypt_ecies, which is used when encrypting record data in set_record
        // This should take the input data and return an "encrypted" version
        signer.expect_encrypt_ecies().returning(|data| {
            // In a real implementation, this would encrypt the data
            // For testing, we'll just prepend a marker to simulate encryption
            let mut encrypted = vec![0xE5, 0xE5]; // "Encryption" marker
            encrypted.extend_from_slice(&data);
            Ok(encrypted)
        });

        // For decrypt_ecies, which is used when decrypting record data in map_record
        // This needs to return a valid SyncData JSON that can be deserialized
        signer.expect_decrypt_ecies().returning(|data| {
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
            .with(eq(u32::MAX))
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

        mock_storage
            .expect_get_pending_outgoing_changes()
            .times(1)
            .with(eq(u32::MAX))
            .returning(move |_| Ok(vec![test_change.clone()]));

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
            .returning(|_, _| Ok(()));

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .returning(|_| Ok(Vec::new()));

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
            .with(eq(u32::MAX))
            .returning(move |_| Ok(vec![test_change.clone()]));

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .returning(|_| Ok(Vec::new()));

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
            .with(eq(u32::MAX))
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
            .expect_decrypt_ecies()
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
            .with(eq(u32::MAX))
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

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .returning(move |_| Ok(vec![incoming_record.clone()]));

        mock_storage
            .expect_update_record_from_incoming()
            .times(1)
            .returning(|_| Ok(()));

        let mut mock_handler = MockNewRecordHandler::new();
        mock_handler
            .expect_on_incoming_change()
            .times(1)
            .returning(|_| Ok(RecordOutcome::Completed));

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
        assert_eq!(result.unwrap(), 1);
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
            .with(eq(u32::MAX))
            .returning(move |_| Ok(vec![incoming_record.clone()]));

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
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[macros::async_test_all]
    async fn test_pull_sync_once_local_deferred_record_kept_for_retry() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();

        // Existing revision equals incoming revision: record was already persisted in sync_state,
        // and should only be retried in the relational handler.
        let incoming_record = crate::sync::storage::IncomingChange {
            new_state: create_record("test", "123", 6),
            old_state: Some(create_record("test", "123", 6)),
        };

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .returning(move |_| Ok(vec![incoming_record.clone()]));

        // No state update should happen when the incoming revision is already reflected.
        mock_storage.expect_update_record_from_incoming().times(0);

        let mut mock_handler = MockNewRecordHandler::new();
        mock_handler
            .expect_on_incoming_change()
            .times(1)
            .returning(|_| Ok(RecordOutcome::Deferred));

        // Deferred records should remain in sync_incoming for future retries.
        mock_storage.expect_delete_incoming_record().times(0);

        let (_tx, rx) = broadcast::channel(10);
        let client = create_signing_client(MockSyncerClient::new(), MockSyncSigner::new());

        let sync_processor =
            SyncProcessor::new(client, rx, Arc::new(mock_handler), Arc::new(mock_storage));

        // Execute
        let result = sync_processor.pull_sync_once_local().await;

        // Verify
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[macros::async_test_all]
    async fn test_push_sync_once_defers_records_with_pending_incoming() {
        // Setup: two outgoing changes, one has a matching pending incoming record
        let deferred_change = create_outgoing_change("test", "123", 1);
        let pushable_change = create_outgoing_change("test", "456", 2);

        let mut mock_storage = MockSyncStorage::new();
        mock_storage
            .expect_get_pending_outgoing_changes()
            .times(1)
            .with(eq(u32::MAX))
            .returning(move |_| Ok(vec![deferred_change.clone(), pushable_change.clone()]));

        // Return a pending incoming record for "test"/"123" — this should cause
        // the outgoing push for that record to be deferred.
        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .returning(|_| {
                Ok(vec![storage::IncomingChange {
                    new_state: storage::Record {
                        id: RecordId::new("test", "123"),
                        revision: 5,
                        schema_version: "0.2.6".to_string(),
                        data: HashMap::new(),
                    },
                    old_state: None,
                }])
            });

        mock_storage
            .expect_complete_outgoing_sync()
            .times(1)
            .withf(|record, local_revision| {
                record.id.r#type == "test" && record.id.data_id == "456" && *local_revision == 2
            })
            .returning(|_, _| Ok(()));

        let mut mock_client = MockSyncerClient::new();
        mock_client.expect_set_record().times(1).returning(|_| {
            Ok(crate::sync::proto::SetRecordReply {
                status: crate::sync::proto::SetRecordStatus::Success as i32,
                new_revision: 11,
            })
        });

        let sync_processor = SyncProcessor::new(
            create_signing_client(mock_client, MockSyncSigner::new()),
            broadcast::channel(10).1,
            Arc::new(MockNewRecordHandler::new()),
            Arc::new(mock_storage),
        );

        // Execute
        let result = sync_processor.push_sync_once().await;

        // Verify: only the non-conflicting record was pushed
        assert_eq!(result.unwrap(), Some(1));
    }

    #[macros::async_test_all]
    async fn test_handle_push_skips_outgoing_when_pull_fails() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        mock_storage
            .expect_get_last_revision()
            .times(1)
            .returning(|| Ok(5));
        mock_storage.expect_get_pending_outgoing_changes().times(0);

        let mut mock_client = MockSyncerClient::new();
        mock_client
            .expect_list_changes()
            .times(1)
            .returning(|_| Err(anyhow!("pull failed")));
        mock_client.expect_set_record().times(0);

        let sync_processor = SyncProcessor::new(
            create_signing_client(mock_client, MockSyncSigner::new()),
            broadcast::channel(10).1,
            Arc::new(MockNewRecordHandler::new()),
            Arc::new(mock_storage),
        );

        let (pull_trigger_tx, pull_trigger) = watch::channel(());
        let _ = pull_trigger_tx.send(());
        let backoff_handle = Mutex::new(None);
        let (backoff_trigger_tx, _backoff_trigger_rx) = mpsc::channel::<Duration>(1);

        // Execute
        let result = sync_processor
            .handle_push(
                Ok(RecordId::new("test", "123")),
                &pull_trigger,
                &backoff_handle,
                &backoff_trigger_tx,
            )
            .await;

        // Verify
        assert_eq!(result, (None, None));
    }

    #[macros::async_test_all]
    async fn test_handle_backoff_skips_outgoing_when_pull_fails() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        mock_storage
            .expect_get_last_revision()
            .times(1)
            .returning(|| Ok(5));
        mock_storage.expect_get_pending_outgoing_changes().times(0);

        let mut mock_client = MockSyncerClient::new();
        mock_client
            .expect_list_changes()
            .times(1)
            .returning(|_| Err(anyhow!("pull failed")));
        mock_client.expect_set_record().times(0);

        let sync_processor = SyncProcessor::new(
            create_signing_client(mock_client, MockSyncSigner::new()),
            broadcast::channel(10).1,
            Arc::new(MockNewRecordHandler::new()),
            Arc::new(mock_storage),
        );

        let backoff_handle = Mutex::new(None);
        let (backoff_trigger_tx, mut backoff_trigger_rx) = mpsc::channel::<Duration>(1);

        // Execute
        let result = sync_processor
            .handle_backoff(
                &backoff_handle,
                &mut backoff_trigger_rx,
                &backoff_trigger_tx,
                Duration::from_millis(10),
            )
            .await;

        // Verify
        assert_eq!(result, (None, None));
    }

    #[macros::async_test_all]
    #[allow(clippy::too_many_lines)]
    async fn test_upgrade_recovery_flow_defers_then_advances_cursor_then_applies() {
        // Phase 1: old client defers incompatible record and keeps it in incoming storage.
        let mut old_storage = MockSyncStorage::new();
        let initial_incoming = crate::sync::storage::IncomingChange {
            new_state: create_record_with_schema("FutureType", "abc", 6, "2.0.0"),
            old_state: None,
        };
        let deferred_retry_incoming = crate::sync::storage::IncomingChange {
            new_state: create_record_with_schema("FutureType", "abc", 6, "2.0.0"),
            old_state: Some(create_record_with_schema("FutureType", "abc", 6, "2.0.0")),
        };

        let mut revision_seq = Sequence::new();
        old_storage
            .expect_get_last_revision()
            .times(1)
            .in_sequence(&mut revision_seq)
            .return_once(|| Ok(5));
        old_storage
            .expect_get_last_revision()
            .times(1)
            .in_sequence(&mut revision_seq)
            .return_once(|| Ok(6));

        old_storage
            .expect_insert_incoming_records()
            .times(1)
            .returning(|_| Ok(()));

        let mut incoming_seq = Sequence::new();
        old_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .in_sequence(&mut incoming_seq)
            .return_once(move |_| Ok(vec![initial_incoming]));
        old_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .in_sequence(&mut incoming_seq)
            .return_once(move |_| Ok(vec![deferred_retry_incoming]));
        old_storage
            .expect_update_record_from_incoming()
            .times(1)
            .returning(|_| Ok(()));
        old_storage.expect_delete_incoming_record().times(0);

        let mut old_handler = MockNewRecordHandler::new();
        old_handler
            .expect_on_incoming_change()
            .times(2)
            .returning(|_| Ok(RecordOutcome::Deferred));

        let mut old_client = MockSyncerClient::new();
        let mut list_changes_seq = Sequence::new();
        let proto_record = crate::sync::proto::Record {
            id: "future:abc".to_string(),
            revision: 6,
            schema_version: "2.0.0".to_string(),
            data: Vec::new(),
        };
        old_client
            .expect_list_changes()
            .times(1)
            .withf(|req| req.since_revision == 5)
            .in_sequence(&mut list_changes_seq)
            .return_once(move |_| {
                Ok(crate::sync::proto::ListChangesReply {
                    changes: vec![proto_record],
                })
            });
        old_client
            .expect_list_changes()
            .times(1)
            .withf(|req| req.since_revision == 6)
            .in_sequence(&mut list_changes_seq)
            .return_once(|_| {
                Ok(crate::sync::proto::ListChangesReply {
                    changes: Vec::new(),
                })
            });

        let old_processor = SyncProcessor::new(
            create_signing_client(old_client, MockSyncSigner::new()),
            broadcast::channel(10).1,
            Arc::new(old_handler),
            Arc::new(old_storage),
        );

        let first_pull = old_processor.pull_sync_once().await.unwrap();
        let second_pull = old_processor.pull_sync_once().await.unwrap();
        assert_eq!(first_pull, Some(0));
        assert_eq!(second_pull, Some(0));

        // Phase 2: upgraded client can now apply and remove previously deferred row.
        let mut upgraded_storage = MockSyncStorage::new();
        let deferred_for_upgrade = crate::sync::storage::IncomingChange {
            new_state: create_record_with_schema("FutureType", "abc", 6, "2.0.0"),
            old_state: Some(create_record_with_schema("FutureType", "abc", 6, "2.0.0")),
        };
        upgraded_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .return_once(move |_| Ok(vec![deferred_for_upgrade]));
        upgraded_storage
            .expect_update_record_from_incoming()
            .times(0);
        upgraded_storage
            .expect_delete_incoming_record()
            .times(1)
            .returning(|_| Ok(()));

        let mut upgraded_handler = MockNewRecordHandler::new();
        upgraded_handler
            .expect_on_incoming_change()
            .times(1)
            .returning(|_| Ok(RecordOutcome::Completed));

        let upgraded_processor = SyncProcessor::new(
            create_signing_client(MockSyncerClient::new(), MockSyncSigner::new()),
            broadcast::channel(10).1,
            Arc::new(upgraded_handler),
            Arc::new(upgraded_storage),
        );

        let applied_count = upgraded_processor.pull_sync_once_local().await.unwrap();
        assert_eq!(applied_count, 1);
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
            .with(eq(u32::MAX))
            .returning(|_| Ok(Vec::new()));

        let (_tx, rx) = broadcast::channel::<RecordId>(10);
        let mock_handler = Arc::new(MockNewRecordHandler::new());
        let mut mock_client = MockSyncerClient::new();
        mock_client
            .expect_listen_changes()
            .returning(|_| Err(anyhow!("subscription not configured in this test")));
        let client = create_signing_client(mock_client, MockSyncSigner::new());

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
    async fn test_start_does_not_fail_when_pull_sync_once_local_fails() {
        // Setup
        let mut mock_storage = MockSyncStorage::new();
        let pending_incoming = crate::sync::storage::IncomingChange {
            new_state: create_record("test", "123", 3),
            old_state: Some(create_record("test", "123", 2)),
        };

        // For ensure_outgoing_record_committed (succeeds)
        mock_storage
            .expect_get_latest_outgoing_change()
            .times(1)
            .returning(|| Ok(None));

        // For pull_sync_once_local (handler fails)
        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .returning(move |_| Ok(vec![pending_incoming.clone()]));

        mock_storage
            .expect_update_record_from_incoming()
            .times(1)
            .returning(|_| Ok(()));

        let mut mock_handler = MockNewRecordHandler::new();
        mock_handler
            .expect_on_incoming_change()
            .times(1)
            .returning(|_| Err(anyhow!("incoming failed")));

        let (_tx, rx) = broadcast::channel::<RecordId>(10);
        let mut mock_client = MockSyncerClient::new();
        mock_client
            .expect_listen_changes()
            .returning(|_| Err(anyhow!("subscription not configured in this test")));
        let client = create_signing_client(mock_client, MockSyncSigner::new());

        let sync_processor = Arc::new(SyncProcessor::new(
            client,
            rx,
            Arc::new(mock_handler),
            Arc::new(mock_storage),
        ));

        let (shutdown_tx, shutdown_rx) = watch::channel(());

        // Execute
        let result = sync_processor.start(shutdown_rx).await;

        // Verify - pull_sync_once_local failure should not cause start to fail
        assert!(result.is_ok());

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
            .with(eq(u32::MAX))
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
                        local_revision: 1,
                    },
                }])
            });

        mock_storage
            .expect_complete_outgoing_sync()
            .times(1)
            .returning(|_, _| Ok(()));

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .returning(|_| Ok(Vec::new()));

        let (tx, rx) = broadcast::channel::<RecordId>(10);
        let mut mock_handler = MockNewRecordHandler::new();
        mock_handler
            .expect_on_sync_completed()
            .times(1)
            .returning(|_, _| Ok(()));
        let mock_handler = Arc::new(mock_handler);
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
        let mut mock_handler = MockNewRecordHandler::new();
        mock_handler
            .expect_on_sync_completed()
            .times(1)
            .returning(|_, _| Ok(()));
        let mock_handler = Arc::new(mock_handler);
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
            .with(eq(u32::MAX))
            .returning(move |_| Ok(vec![test_change.clone()]));

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .returning(|_| Ok(Vec::new()));

        let mock_client = MockSyncerClient::new();

        // Create mock signer that fails on encryption
        let mut mock_signer = MockSyncSigner::new();
        mock_signer
            .expect_encrypt_ecies()
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
            .expect_decrypt_ecies()
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

        mock_signer.expect_decrypt_ecies().times(1).returning(|_| {
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
            .with(eq(u32::MAX))
            .returning(move |_| Ok(vec![test_change.clone()]));

        mock_storage
            .expect_get_incoming_records()
            .times(1)
            .with(eq(u32::MAX))
            .returning(|_| Ok(Vec::new()));

        let mock_client = MockSyncerClient::new();

        // Create mock signer that fails on signing
        let mut mock_signer = MockSyncSigner::new();
        mock_signer.expect_encrypt_ecies().times(1).returning(Ok);

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
