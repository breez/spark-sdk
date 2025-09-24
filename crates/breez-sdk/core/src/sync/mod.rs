#![allow(dead_code)]
use crate::SdkError;

mod spark;
mod sparkscan;

pub use spark::SparkSyncService;
pub use sparkscan::SparkscanSyncService;

#[macros::async_trait]
pub trait SyncService: Send + Sync {
    async fn sync_payments(&self) -> Result<(), SdkError>;
    async fn sync_historical_payments(&self) -> Result<(), SdkError>;
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

    pub async fn sync_historical_payments(&self) -> Result<(), SdkError> {
        match self {
            SyncStrategy::Spark(service) => service.sync_historical_payments().await,
            SyncStrategy::Sparkscan(service) => service.sync_historical_payments().await,
        }
    }
}
