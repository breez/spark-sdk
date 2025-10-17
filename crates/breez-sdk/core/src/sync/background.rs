use std::{sync::Arc, time::Duration};

use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, info, warn};

use crate::{Storage, persist::OutgoingRecordParent};
use breez_sdk_common::sync::{
    model::{OutgoingRecord, Record, RecordId},
    signing_client::SigningClient,
};

const SYNC_BATCH_SIZE: u32 = 10;

pub struct SyncProcessor {
    push_sync_trigger: broadcast::Receiver<RecordId>,
    incoming_record_callback: mpsc::Sender<(Record, oneshot::Sender<anyhow::Result<()>>)>,
    outgoing_record_callback: mpsc::Sender<(Record, oneshot::Sender<anyhow::Result<()>>)>,
    client: SigningClient,
    storage: Arc<dyn Storage>,
}

enum BackoffType {
    Push,
    Pull,
}

impl SyncProcessor {
    pub fn new(
        client: SigningClient,
        push_sync_trigger: broadcast::Receiver<RecordId>,
        incoming_record_callback: mpsc::Sender<(Record, oneshot::Sender<()>)>,
        storage: Arc<dyn Storage>,
    ) -> Self {
        SyncProcessor {
            push_sync_trigger,
            incoming_record_callback,
            client,
            storage,
        }
    }

    pub async fn start(self: &Arc<Self>, shutdown_receiver: watch::Receiver<()>) -> anyhow::Result<()> {
        info!("Starting sync processor");
        
        // NOTE: THe order here is important. First handle any existing incoming records, because they are always handled immediately.
        self.pull_sync_once_local().await?;

        // Apply the LATEST outgoing record to the relational data store before starting the sync loops.
        // It's possible this outgoing record was inserted, while the database update after that was not completed yet.
        self.ensure_outgoing_record_committed().await?;

        let (pull_trigger_tx, pull_trigger) = watch::channel(());
        self.start_subscribe_updates_task(shutdown_receiver.clone(), pull_trigger_tx);
        self.start_sync_loop_task(shutdown_receiver, pull_trigger);
        Ok(())
    }

    fn start_subscribe_updates_task(self: &Arc<Self>, shutdown_receiver: watch::Receiver<()>, pull_trigger_tx: watch::Sender<()>) {
        let clone = Arc::clone(self);
        tokio::spawn(async move { clone.subscribe_updates_forever(shutdown_receiver, pull_trigger_tx).await });
    }

    async fn ensure_outgoing_record_committed(&self) -> anyhow::Result<()> {
        let Some(record) = self.storage.sync_get_latest_outgoing_record().await? else {
            return Ok(());
        };

        let (tx, rx) = oneshot::channel();
        self.outgoing_record_callback.send((record, tx)).await?;
        rx.await??;
        Ok(())
    }

    async fn subscribe_updates_forever(&self, shutdown_receiver: watch::Receiver<()>, pull_trigger_tx: watch::Sender<()>) {
        loop {
            self.subscribe_updates(shutdown_receiver.clone(), pull_trigger_tx.clone()).await;
            debug!("Re-establishing update subscription after disconnection");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    async fn subscribe_updates(
        &self,
        mut shutdown_receiver: watch::Receiver<()>,
        mut tx: watch::Sender<()>,
    ) {
        let stream = self.client.listen_changes().await?;
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

    fn start_sync_loop_task(self: &Arc<Self>, shutdown_receiver: watch::Receiver<()>, pull_trigger: watch::Receiver<()>) {
        let clone = Arc::clone(self);
        tokio::spawn(async move { clone.sync_loop(shutdown_receiver, pull_trigger).await });
    }

    async fn sync_loop(&self, mut shutdown_receiver: watch::Receiver<()>, mut pull_trigger: watch::Receiver<()>) {
        let mut push_trigger = self.push_sync_trigger.resubscribe();
        let (backoff_trigger_tx, mut backoff_trigger_rx) = mpsc::channel::<Duration>(10);

        let send_backoff_trigger = |duration: Duration| {
            tokio::spawn({
                let backoff_trigger_tx = backoff_trigger_tx.clone();
                async move {
                    tokio::time::sleep(duration).await;
                    if let Err(e) = backoff_trigger_tx.send(duration).await {
                        error!("Failed to send backoff trigger: {}", e);
                    }
                }
            });
        };

        loop {
            tokio::select! {
                _ = shutdown_receiver.changed() => {
                    debug!("Shutdown signal received, stopping push sync loop");
                    break;
                }

                _ = pull_trigger.changed() => {
                    debug!("Received incoming sync notification");
                    if let Err(e) = self.pull_sync_once().await {
                        error!("Failed to sync once: {}", e);
                    }
                }

                Some(last_backoff) = backoff_trigger_rx.recv() => {
                    debug!("Backoff trigger received, waiting before next sync attempt");
                    if let Err(e) = self.pull_sync_once().await {
                        error!("Failed to sync once: {}", e);
                        send_backoff_trigger(last_backoff * 2);
                        continue;
                    }
                    if let Err(e) = self.push_sync_once().await {
                        error!("Failed to sync once: {}", e);
                        send_backoff_trigger(last_backoff * 2);
                        continue;
                    }
                }

                result = push_trigger.recv() => {
                    match result {
                        Ok(record_id) => {
                            debug!("Received sync trigger for record id {}", record_id);
                            // If there was also a pull trigger, handle that first, because the push wouldn't work.
                            if pull_trigger.has_changed().unwrap_or(false) {
                                if let Err(e) = self.pull_sync_once().await {
                                    error!("Failed to sync once: {}", e);
                                }
                            }

                            if let Err(e) = self.push_sync_once().await {
                                error!("Failed to sync once: {}", e);
                                send_backoff_trigger(Duration::from_secs(1));
                                continue;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            debug!("Push sync trigger channel closed, stopping push sync loop");
                            break;
                        }
                        Err(broadcast::error::RecvError::Lagged(count)) => {
                            warn!("Lagged {} messages in push sync trigger channel", count);
                        }
                    }
                }
            }
        }
    }

    async fn push_sync_once(&self) -> anyhow::Result<()> {
        debug!("Syncing once");

        while let records = self
            .storage
            .sync_get_pending_outgoing_records(SYNC_BATCH_SIZE)
            .await?
            && !records.is_empty()
        {
            self.push_sync_batch(records).await?;
        }

        Ok(())
    }

    async fn push_sync_batch(&self, records: Vec<OutgoingRecordParent>) -> anyhow::Result<()> {
        for storage_record in records {
            let record = storage_record.record.try_into()?;
            let existing_record = storage_record.parent.map(Record::try_from).transpose()?;
            self.push_sync_record(record, existing_record).await?;
        }

        Ok(())
    }

    async fn push_sync_record(
        &self,
        record: OutgoingRecord,
        existing_record: Option<Record>,
    ) -> anyhow::Result<()> {
        // Merges the updated fields with the existing record data in the local sync state to form the new record.
        let record = record.with_parent(existing_record);

        // TODO: Encrypt.
        // Pushes the record to the remote server.
        // TODO: If the remote server already has this exact revision, check what happens. We should continue then for idempotency.
        self.client.set_record(&record).await?;

        // Removes the pending outgoing record and updates the existing record with the new one.
        self.storage
            .sync_complete_outgoing_sync((&record).try_into()?)
            .await?;
        Ok(())
    }

    async fn pull_sync_once(&self) -> anyhow::Result<()> {
        debug!("Pull syncing once");

        let since_revision = self.storage.sync_get_last_revision().await?;

        let reply = self.client.list_changes(since_revision).await?;

        let records = reply.changes.into_iter().map(Record::try_from).collect::<Result<Vec<_>, _>>()?;
        records.sort_by(|a, b|a.revision.cmp(&b.revision));
        let db_records = records.iter().map(crate::persist::Record::try_from).collect::<Result<Vec<_>, _>>()?;
        self.storage.sync_insert_incoming_records(db_records).await?;

        self.pull_sync_once_local().await?;

        Ok(())
    }

    async fn pull_sync_once_local(&self) -> anyhow::Result<()> {
        loop {
            let records = self.storage.sync_get_incoming_records(SYNC_BATCH_SIZE).await?;
            for record in records {
                // NOTE: The incoming record will have the same revision number as a pending outgoing record (if any).
                // A rebase now means just updating the pending outgoing records to have a higher revision number.
                // If data becomes more complex, we might need to do a proper rebase with conflict resolution here.
                self.storage.sync_rebase_pending_outgoing_records(record.revision).await?;

                // First update the sync state from the incoming record. The sync state will have to change anyway, 
                // there is no going back if there is a remote change. We don't remove the incoming record yet,
                // to ensure we'll update the relational database state if we turn off now.
                self.storage.sync_update_record_from_incoming(&record).await?;

                let (tx, rx) = oneshot::channel();
                self.incoming_record_callback.send((record, tx)).await?;
                rx.await??;

                self.storage.sync_delete_incoming_record(&record).await?;
            }
        }
    }
}
