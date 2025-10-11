use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::{error, debug, info};

use crate::sync::{model::{Record, RecordId}, signing_client::SigningClient, storage::SyncStorage};

const SYNC_BATCH_SIZE: usize = 10;

pub struct SyncProcessor {
    push_sync_trigger: broadcast::Receiver<RecordId>,
    client: SigningClient,
    storage: Arc<dyn SyncStorage>,
}

impl SyncProcessor {
    pub fn new(client: SigningClient, push_sync_trigger: broadcast::Receiver<RecordId>, storage: Arc<dyn SyncStorage>) -> Self {
        SyncProcessor {
            client,
            push_sync_trigger,
            storage
        }
    }

    pub async fn start(self: &Arc<Self>) {
        info!("Starting sync processor");
        let clone = Arc::clone(self);
        tokio::spawn(async move { 
            clone.push_sync_loop().await
        });
    }

    async fn push_sync_loop(&self) {
        let mut push_trigger = self.push_sync_trigger.resubscribe();
        while let Ok(record_id) = push_trigger.recv().await {
            debug!("Received sync trigger for record id {}", record_id);
            if let Err(e) = self.push_sync_once().await {
                error!("Failed to sync once: {}", e);
            }
        }
    }

    async fn push_sync_once(&self) -> anyhow::Result<()> {
        debug!("Syncing once");

        while let records = self.storage.get_pending_outgoing_records(SYNC_BATCH_SIZE).await? && !records.is_empty() {
            self.push_sync_batch(records).await?;
        }

        Ok(())
    }

    async fn push_sync_batch(&self, records: Vec<Record>) -> anyhow::Result<()> {
        let mut tasks = Vec::new();
        for record in records {
            tasks.push(self.push_sync_record(record));
        }

        let results = futures::future::join_all(tasks).await;
        for result in results {
            result?;
        }
        Ok(())
    }

    async fn push_sync_record(&self, record: Record) -> anyhow::Result<()> {
        self.client.set_record(record).await?;
        Ok(())
    }
}