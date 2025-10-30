use std::sync::Arc;

use crate::sync::{RecordChangeRequest, RecordId, UnversionedRecordChange, storage::SyncStorage};
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, warn};

pub struct SyncService {
    storage: Arc<dyn SyncStorage>,
    sync_trigger: broadcast::Sender<RecordId>,
    mtx: RwLock<()>,
}

impl SyncService {
    pub fn new(storage: Arc<dyn SyncStorage>) -> Self {
        SyncService {
            storage,
            sync_trigger: broadcast::channel(16).0,
            mtx: RwLock::new(()),
        }
    }

    pub fn get_sync_trigger(&self) -> broadcast::Receiver<RecordId> {
        self.sync_trigger.subscribe()
    }

    pub async fn set_outgoing_record(&self, record: &RecordChangeRequest) -> anyhow::Result<()> {
        debug!("Adding record for outgoing sync: {:?}", record);
        let _guard = self.mtx.write().await;
        let record: UnversionedRecordChange = record.into();
        let record_id = record.id.clone();
        self.storage.add_outgoing_change(record.try_into()?).await?;
        if self.sync_trigger.send(record_id.clone()).is_err() {
            warn!(
                "Real-time sync failed to trigger for record id {:?}",
                record_id
            );
        }

        Ok(())
    }
}
