use std::sync::Arc;

use tokio::sync::{broadcast, watch};
use tracing::{debug, error, info, warn};

use crate::{Storage, persist::OutgoingRecordParent};
use breez_sdk_common::sync::{
    model::{OutgoingRecord, Record, RecordId},
    signing_client::SigningClient,
};

const SYNC_BATCH_SIZE: u32 = 10;

pub struct SyncProcessor {
    push_sync_trigger: broadcast::Receiver<RecordId>,
    client: SigningClient,
    storage: Arc<dyn Storage>,
}

impl SyncProcessor {
    pub fn new(
        client: SigningClient,
        push_sync_trigger: broadcast::Receiver<RecordId>,
        storage: Arc<dyn Storage>,
    ) -> Self {
        SyncProcessor {
            push_sync_trigger,
            client,
            storage,
        }
    }

    pub fn start(self: &Arc<Self>, shutdown_receiver: watch::Receiver<()>) {
        info!("Starting sync processor");
        let clone = Arc::clone(self);
        tokio::spawn(async move { clone.sync_loop(shutdown_receiver).await });
    }

    async fn sync_loop(&self, mut shutdown_receiver: watch::Receiver<()>) {
        let mut push_trigger = self.push_sync_trigger.resubscribe();
        loop {
            tokio::select! {
                _ = shutdown_receiver.changed() => {
                    debug!("Shutdown signal received, stopping push sync loop");
                    break;
                }

                result = push_trigger.recv() => {
                    match result {
                        Ok(record_id) => {
                            debug!("Received sync trigger for record id {}", record_id);
                            if let Err(e) = self.push_sync_once().await {
                                error!("Failed to sync once: {}", e);
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

        // Pushes the record to the remote server.
        // TODO: If the remote server already has this exact revision, check what happens. We should continue then for idempotency.
        self.client.set_record(&record).await?;

        // Removes the pending outgoing record and updates the existing record with the new one.
        self.storage
            .sync_complete_outgoing_sync(record.try_into()?)
            .await?;
        Ok(())
    }
}
