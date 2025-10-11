use std::{sync::Arc, time::Duration};

use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, warn};

use crate::Storage;
use breez_sdk_common::sync::{
    model::{IncomingChange, OutgoingChange, Record, RecordId},
    signing_client::SigningClient,
};

const SYNC_BATCH_SIZE: u32 = 10;

pub struct Callback<T> {
    pub args: T,
    pub responder: oneshot::Sender<anyhow::Result<()>>,
}
pub type CallbackSender<T> = mpsc::Sender<Callback<T>>;
pub type CallbackReceiver<T> = mpsc::Receiver<Callback<T>>;

pub struct SyncProcessor {
    push_sync_trigger: broadcast::Receiver<RecordId>,
    incoming_record_callback: CallbackSender<IncomingChange>,
    outgoing_record_callback: CallbackSender<OutgoingChange>,
    client: SigningClient,
    storage: Arc<dyn Storage>,
}

impl SyncProcessor {
    pub fn new(
        client: SigningClient,
        push_sync_trigger: broadcast::Receiver<RecordId>,
        incoming_record_callback: CallbackSender<IncomingChange>,
        outgoing_record_callback: CallbackSender<OutgoingChange>,
        storage: Arc<dyn Storage>,
    ) -> Self {
        SyncProcessor {
            push_sync_trigger,
            incoming_record_callback,
            outgoing_record_callback,
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
        let Some(record) = self.storage.sync_get_latest_outgoing_change().await? else {
            debug!("There is no pending outgoing change to commit");
            return Ok(());
        };

        debug!(
            "Committing latest pending outgoing change for record {:?}, revision {}",
            record.change.id, record.change.revision
        );
        let (tx, rx) = oneshot::channel();
        self.outgoing_record_callback
            .send(Callback {
                args: record.try_into()?,
                responder: tx,
            })
            .await?;
        rx.await??;
        Ok(())
    }

    async fn subscribe_updates_forever(
        &self,
        shutdown_receiver: watch::Receiver<()>,
        pull_trigger_tx: watch::Sender<()>,
    ) {
        loop {
            self.subscribe_updates(shutdown_receiver.clone(), pull_trigger_tx.clone())
                .await;
            debug!("Re-establishing update subscription after disconnection");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    async fn subscribe_updates(
        &self,
        mut shutdown_receiver: watch::Receiver<()>,
        tx: watch::Sender<()>,
    ) {
        let Ok(mut stream) = self.client.listen_changes().await else {
            error!("Failed to establish update subscription");
            return;
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
                        send_backoff_trigger(last_backoff.mul_f32(1.5));
                        continue;
                    }
                    if let Err(e) = self.push_sync_once().await {
                        error!("Failed to sync once: {}", e);
                        send_backoff_trigger(last_backoff.mul_f32(1.5));
                        continue;
                    }
                    debug!("Backoff sync attempt succeeded, resuming normal operation");
                }

                result = push_trigger.recv() => {
                    match result {
                        Ok(record_id) => {
                            debug!("Received sync trigger for record id {:?}", record_id);
                            // If there was also a pull trigger, handle that first, because the push wouldn't work.
                            if pull_trigger.has_changed().unwrap_or(false) &&
                                let Err(e) = self.pull_sync_once().await {
                                    error!("Failed to sync once: {}", e);
                            }

                            if let Err(e) = self.push_sync_once().await {
                                error!("Failed to sync once: {}", e);
                                send_backoff_trigger(Duration::from_secs(1));
                                continue;
                            }
                            debug!("Push sync attempt succeeded");
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

        while let changes = self
            .storage
            .sync_get_pending_outgoing_changes(SYNC_BATCH_SIZE)
            .await?
            && !changes.is_empty()
        {
            self.push_sync_batch(changes).await?;
        }

        Ok(())
    }

    async fn push_sync_batch(
        &self,
        changes: Vec<crate::persist::OutgoingChange>,
    ) -> anyhow::Result<()> {
        debug!(
            "Processing sync batch of {} outgoing changes",
            changes.len()
        );
        for storage_change in changes {
            let change = storage_change.try_into()?;
            self.push_sync_record(change).await?;
        }

        Ok(())
    }

    async fn push_sync_record(&self, change: OutgoingChange) -> anyhow::Result<()> {
        // Merges the updated fields with the existing record data in the local sync state to form the new record.
        let record = change.merge();

        debug!(
            "Pushing outgoing record {:?}, revision {} to remote",
            record.id, record.revision
        );
        // TODO: Encrypt.
        // Pushes the record to the remote server.
        // TODO: If the remote server already has this exact revision, check what happens. We should continue then for idempotency.
        self.client.set_record(&record).await?;

        debug!(
            "Completing outgoing record {:?}, revision {}",
            record.id, record.revision
        );
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

        debug!(
            "real-time sync list_changes yielded {} results.",
            reply.changes.len()
        );
        let mut records = reply
            .changes
            .into_iter()
            .map(Record::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by(|a, b| a.revision.cmp(&b.revision));
        let db_records = records
            .iter()
            .map(crate::persist::Record::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        if !records.is_empty() {
            self.storage
                .sync_insert_incoming_records(db_records)
                .await?;
        }

        self.pull_sync_once_local().await?;

        Ok(())
    }

    async fn pull_sync_once_local(&self) -> anyhow::Result<()> {
        loop {
            let incoming_records = self
                .storage
                .sync_get_incoming_records(SYNC_BATCH_SIZE)
                .await?;
            if incoming_records.is_empty() {
                break;
            }

            debug!("Processing {} incoming records", incoming_records.len());
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
                    .sync_rebase_pending_outgoing_records(incoming_record.new_state.revision)
                    .await?;

                // First update the sync state from the incoming record. The sync state will have to change anyway,
                // there is no going back if there is a remote change. We don't remove the incoming record yet,
                // to ensure we'll update the relational database state if we turn off now.
                debug!(
                    "Updating sync state from incoming record {:?}, revision {}",
                    incoming_record.new_state.id, incoming_record.new_state.revision
                );
                self.storage
                    .sync_update_record_from_incoming(incoming_record.new_state.clone())
                    .await?;

                // Now notify the relational database to update. Wait for it to be done. Note that this could be improved
                // in the future to also add actions to the pending outgoing changes for the same record. Like maybe delete
                // an action, or change its field values. Now it is not necessary yet, because there are only immutable
                // changes.
                debug!(
                    "Invoking relational database callback for incoming record {:?}, revision {}",
                    incoming_record.new_state.id, incoming_record.new_state.revision
                );
                let (tx, rx) = oneshot::channel();
                self.incoming_record_callback
                    .send(Callback {
                        args: (&incoming_record).try_into()?,
                        responder: tx,
                    })
                    .await?;
                rx.await??;

                debug!(
                    "Removing incoming record after processing completion {:?}, revision {}",
                    incoming_record.new_state.id, incoming_record.new_state.revision
                );

                // Now it's safe to delete the incoming record.
                self.storage
                    .sync_delete_incoming_record(incoming_record.new_state)
                    .await?;
            }
        }

        Ok(())
    }
}
