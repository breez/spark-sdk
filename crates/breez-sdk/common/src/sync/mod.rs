mod background;
mod client;
mod model;
mod service;
mod signer;
mod signing_client;
pub mod storage;

pub use {background::*, client::*, model::*, service::*, signer::*, signing_client::*};

/// Trait for distributed lock operations via the sync service.
///
/// Used to coordinate actions across multiple SDK instances sharing the same identity.
/// All operations are best-effort — callers handle errors gracefully.
#[macros::async_trait]
pub trait SyncLockClient: Send + Sync {
    async fn set_lock(&self, lock_name: &str, acquire: bool) -> anyhow::Result<()>;
    async fn is_locked(&self, lock_name: &str) -> anyhow::Result<bool>;
}

#[allow(clippy::doc_markdown)]
pub mod proto {
    tonic::include_proto!("sync");
}
