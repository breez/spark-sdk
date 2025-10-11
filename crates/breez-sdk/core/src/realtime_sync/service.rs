use std::sync::Arc;

use breez_sdk_common::sync::model::{RecordChangeRequest, RecordId, UnversionedRecordChange};
use tokio::sync::{RwLock, broadcast};
use tracing::warn;

use crate::Storage;

pub struct SyncService {
    storage: Arc<dyn Storage>,
    sync_trigger: broadcast::Sender<RecordId>,
    mtx: RwLock<()>,
}

// TODO: Fix errors
impl SyncService {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
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
        let _guard = self.mtx.write().await;
        let record: UnversionedRecordChange = record.into();
        let record_id = record.id.clone();
        self.storage
            .sync_add_outgoing_change(record.try_into()?)
            .await?;
        if self.sync_trigger.send(record_id.clone()).is_err() {
            warn!(
                "Real-time sync failed to trigger for record id {}",
                record_id
            );
        }

        Ok(())
    }
}
