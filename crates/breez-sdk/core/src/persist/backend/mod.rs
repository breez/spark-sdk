//! Storage backend abstraction.
//!
//! A [`StorageBackend`] produces the four per-tenant stores the SDK needs: the
//! main [`Storage`], plus a [`TreeStore`], [`TokenOutputStore`] and
//! [`SessionStore`]. Each built-in backend is an independent module — adding
//! one touches nothing else, and there is no central enum of storage options.
//!
//! [`SdkBuilder::with_storage_backend`](crate::SdkBuilder::with_storage_backend)
//! takes an `Arc<dyn StorageBackend>`; build one with [`default_storage`],
//! [`postgres_storage`], [`mysql_storage`] or
//! [`custom_storage`].

use std::sync::Arc;

use macros::async_trait;
use spark_wallet::{SessionStore, TokenOutputStore, TreeStore};

use crate::{Network, SdkError, persist::Storage};

mod prebuilt;

pub use prebuilt::PrebuiltBackend;

#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(feature = "postgres")]
mod postgres;

#[cfg(feature = "mysql")]
mod mysql;

/// The four per-tenant stores produced by a [`StorageBackend`].
///
/// An opaque handle: the SDK reads its stores internally; the fields never
/// cross the FFI boundary.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct ResolvedStores {
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) tree_store: Option<Arc<dyn TreeStore>>,
    pub(crate) token_output_store: Option<Arc<dyn TokenOutputStore>>,
    pub(crate) session_store: Option<Arc<dyn SessionStore>>,
}

/// A factory for a tenant's storage.
///
/// A single backend may back many SDK instances; each
/// [`create_stores`](Self::create_stores) call yields the store set scoped to
/// one tenant `identity` (a serialized public key). `network` lets file-based
/// backends segregate tenants by network; database backends ignore it.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn create_stores(
        &self,
        network: Network,
        identity: Vec<u8>,
    ) -> Result<Arc<ResolvedStores>, SdkError>;
}

/// Wraps a caller-supplied [`Storage`] implementation as a [`StorageBackend`].
/// The tree, token-output and session stores use the in-memory defaults.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn custom_storage(storage: Arc<dyn Storage>) -> Arc<dyn StorageBackend> {
    Arc::new(prebuilt::PrebuiltBackend::new(storage, None, None, None))
}

/// File-based `SQLite` storage rooted at `storage_dir` — the default for
/// mobile and desktop apps. Each tenant gets its own database file under the
/// directory.
#[cfg(feature = "sqlite")]
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn default_storage(storage_dir: String) -> Arc<dyn StorageBackend> {
    Arc::new(sqlite::SqliteBackend::new(storage_dir))
}

/// `PostgreSQL`-backed storage built from `config`. Opens the connection pool;
/// fails if `config` is invalid.
#[cfg(feature = "postgres")]
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[allow(clippy::needless_pass_by_value)]
pub fn postgres_storage(
    config: crate::persist::postgres::PostgresStorageConfig,
) -> Result<Arc<dyn StorageBackend>, SdkError> {
    let run_migration = config.run_migration;
    let pool = crate::persist::postgres::create_pool(&config)?;
    Ok(Arc::new(postgres::PostgresBackend::new(
        pool,
        run_migration,
    )))
}

/// `MySQL`-backed storage built from `config`. Opens the connection pool;
/// fails if `config` is invalid.
#[cfg(feature = "mysql")]
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[allow(clippy::needless_pass_by_value)]
pub fn mysql_storage(
    config: crate::persist::mysql::MysqlStorageConfig,
) -> Result<Arc<dyn StorageBackend>, SdkError> {
    let run_migration = config.run_migration;
    let foreign_key_mode = config.foreign_key_mode;
    let pool = crate::persist::mysql::create_pool(&config)?;
    Ok(Arc::new(mysql::MysqlBackend::new(
        pool,
        run_migration,
        foreign_key_mode,
    )))
}
