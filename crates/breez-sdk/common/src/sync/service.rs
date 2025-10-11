use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::warn;

use crate::sync::{model::{Record, RecordId}, storage::SyncStorage};

// TODO: Name properly.
pub struct SyncService {
    storage: Arc<dyn SyncStorage>,
    sync_trigger: broadcast::Sender<RecordId>,
}

// TODO: Fix errors
impl SyncService {
    pub fn new(storage: Arc<dyn SyncStorage>) -> Self {
        SyncService { storage , sync_trigger: broadcast::channel(16).0 }
    }

    pub async fn set_outgoing_record(&self, record: &Record) -> anyhow::Result<()> {
        self.storage.add_outgoing_record(record).await?;
        if self.sync_trigger.send(record.id.clone()).is_err() {
            warn!("Real-time sync failed to trigger for record id {}", record.id);
        }

        Ok(())
    }
}
