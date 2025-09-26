#![allow(dead_code)]
use crate::SdkError;

mod spark;
mod sparkscan;

pub(crate) use spark::SparkSyncService;
pub(crate) use sparkscan::SparkscanSyncService;

#[macros::async_trait]
pub trait SyncService: Send + Sync {
    async fn sync_payments(&self) -> Result<(), SdkError>;
}

pub enum SyncStrategy {
    Sparkscan(SparkscanSyncService),
    Spark(SparkSyncService),
}

impl SyncStrategy {
    pub async fn sync_payments(&self) -> Result<(), SdkError> {
        match self {
            SyncStrategy::Spark(service) => service.sync_payments().await,
            SyncStrategy::Sparkscan(service) => service.sync_payments().await,
        }
    }
}
