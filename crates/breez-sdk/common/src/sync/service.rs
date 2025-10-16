use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use tracing::warn;

use crate::sync::{model::RecordId, storage::SyncStorage, OutgoingRecordRequest};

// TODO: Name properly.
pub struct SyncService {
    storage: Arc<dyn SyncStorage>,
    sync_trigger: broadcast::Sender<RecordId>,
    mtx: RwLock<()>,
}

// TODO: Fix errors
impl SyncService {
    pub fn new(storage: Arc<dyn SyncStorage>) -> Self {
        SyncService { storage , sync_trigger: broadcast::channel(16).0, mtx: RwLock::new(()) }
    }

    pub async fn set_outgoing_record(&self, record: &OutgoingRecordRequest) -> anyhow::Result<()> {
        let _guard = self.mtx.write().await;

        // TODO: When there are changes to existing records, they would have to be merged here.
        let record = record.try_into().map_err(|e|anyhow::Error::msg(e))?;
        self.storage.add_outgoing_record(&record).await?;
        if self.sync_trigger.send(record.id.clone()).is_err() {
            warn!("Real-time sync failed to trigger for record id {}", record.id);
        }

        Ok(())
    }
}
