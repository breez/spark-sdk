//! Storage backend abstraction.
//!
//! A database (or caller-supplied) backend produces the four per-tenant stores
//! the SDK needs: the main [`Storage`], plus a [`TreeStore`],
//! [`TokenOutputStore`] and [`SessionStore`].
//! [`SdkBuilder::build`](crate::SdkBuilder::build) resolves exactly one
//! [`StorageBackend`] and calls [`StorageBackend::create_stores`] once — there
//! is no per-database branching outside this module.
//!
//! [`StorageConfig`] selects a built-in backend and is native-only: on WASM all
//! storage is JS-backed and reaches the SDK as a [`CustomStorage`].

use std::sync::Arc;

use macros::async_trait;
use spark_wallet::{PublicKey, SessionStore, TokenOutputStore, TreeStore};

use crate::{Network, SdkError, persist::Storage};

mod prebuilt;

#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(feature = "postgres")]
mod postgres;

#[cfg(feature = "mysql")]
mod mysql;

/// The four per-tenant stores produced by a [`StorageBackend`].
///
/// Only `storage` is mandatory. When `tree_store`, `token_output_store` or
/// `session_store` is `None`, the wallet builder substitutes its in-memory
/// default.
pub(crate) struct ResolvedStores {
    pub storage: Arc<dyn Storage>,
    pub tree_store: Option<Arc<dyn TreeStore>>,
    pub token_output_store: Option<Arc<dyn TokenOutputStore>>,
    pub session_store: Option<Arc<dyn SessionStore>>,
}

/// A factory for a tenant's storage.
///
/// A single backend may back many SDK instances; each
/// [`create_stores`](Self::create_stores) call yields the store set scoped to
/// one `identity`.
#[async_trait]
pub(crate) trait StorageBackend: Send + Sync {
    async fn create_stores(&self, identity: &PublicKey) -> Result<ResolvedStores, SdkError>;
}

/// A caller-supplied set of stores, for integrators that implement their own
/// persistence.
///
/// Only `storage` is required; `tree_store`, `token_output_store` and
/// `session_store` fall back to in-memory implementations when `None`. Pass it
/// to [`SdkBuilder::with_storage`](crate::SdkBuilder::with_storage).
#[derive(Clone)]
pub struct CustomStorage {
    pub storage: Arc<dyn Storage>,
    pub tree_store: Option<Arc<dyn TreeStore>>,
    pub token_output_store: Option<Arc<dyn TokenOutputStore>>,
    pub session_store: Option<Arc<dyn SessionStore>>,
}

impl From<Arc<dyn Storage>> for CustomStorage {
    fn from(storage: Arc<dyn Storage>) -> Self {
        Self::new(storage)
    }
}

impl CustomStorage {
    /// A custom storage that supplies only the main [`Storage`]. The tree,
    /// token-output and session stores use the in-memory defaults. Add them
    /// with the `with_*` methods.
    #[must_use]
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage,
            tree_store: None,
            token_output_store: None,
            session_store: None,
        }
    }

    /// Sets a custom tree store. Without one, an in-memory tree store is used.
    #[must_use]
    pub fn with_tree_store(mut self, tree_store: Arc<dyn TreeStore>) -> Self {
        self.tree_store = Some(tree_store);
        self
    }

    /// Sets a custom token-output store. Without one, an in-memory store is used.
    #[must_use]
    pub fn with_token_output_store(
        mut self,
        token_output_store: Arc<dyn TokenOutputStore>,
    ) -> Self {
        self.token_output_store = Some(token_output_store);
        self
    }

    /// Sets a custom session store. Without one, an in-memory store is used.
    #[must_use]
    pub fn with_session_store(mut self, session_store: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(session_store);
        self
    }
}

/// Selects a built-in storage backend.
///
/// Construct it via [`default_storage`], [`postgres_storage`] or
/// [`mysql_storage`] rather than naming variants directly. Pass it to
/// [`SdkBuilder::with_storage_backend`](crate::SdkBuilder::with_storage_backend)
/// or [`SdkContextConfig`](crate::SdkContextConfig).
///
/// Each variant is gated behind its storage feature — `sqlite`, `postgres` or
/// `mysql` — and all three are native-only (their Rust drivers don't build for
/// WASM). WASM builds supply storage as a [`CustomStorage`] instead.
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum StorageConfig {
    /// File-based `SQLite` storage rooted at `storage_dir`.
    #[cfg(feature = "sqlite")]
    Sqlite { storage_dir: String },
    #[cfg(feature = "postgres")]
    Postgres {
        config: crate::persist::postgres::PostgresStorageConfig,
    },
    #[cfg(feature = "mysql")]
    Mysql {
        config: crate::persist::mysql::MysqlStorageConfig,
    },
}

/// `SQLite` storage rooted at `storage_dir` — the default for mobile and
/// desktop apps. Each tenant gets its own database file under the directory.
#[cfg(feature = "sqlite")]
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn default_storage(storage_dir: String) -> StorageConfig {
    StorageConfig::Sqlite { storage_dir }
}

/// `PostgreSQL`-backed storage built from `config`.
#[cfg(feature = "postgres")]
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn postgres_storage(config: crate::persist::postgres::PostgresStorageConfig) -> StorageConfig {
    StorageConfig::Postgres { config }
}

/// `MySQL`-backed storage built from `config`.
#[cfg(feature = "mysql")]
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn mysql_storage(config: crate::persist::mysql::MysqlStorageConfig) -> StorageConfig {
    StorageConfig::Mysql { config }
}

impl StorageConfig {
    #[allow(unused_variables, clippy::unnecessary_wraps)]
    pub(crate) fn into_backend(
        self,
        network: Network,
    ) -> Result<Arc<dyn StorageBackend>, SdkError> {
        match self {
            #[cfg(feature = "sqlite")]
            StorageConfig::Sqlite { storage_dir } => {
                Ok(Arc::new(sqlite::SqliteBackend::new(storage_dir, network)))
            }
            #[cfg(feature = "postgres")]
            StorageConfig::Postgres { config } => {
                let run_migration = config.run_migration;
                let pool = crate::persist::postgres::create_pool(&config)?;
                Ok(Arc::new(postgres::PostgresBackend::new(
                    pool,
                    run_migration,
                )))
            }
            #[cfg(feature = "mysql")]
            StorageConfig::Mysql { config } => {
                let run_migration = config.run_migration;
                let foreign_key_mode = config.foreign_key_mode;
                let pool = crate::persist::mysql::create_pool(&config)?;
                Ok(Arc::new(mysql::MysqlBackend::new(
                    pool,
                    run_migration,
                    foreign_key_mode,
                )))
            }
        }
    }
}

/// The single storage source for one `SdkBuilder` — either a built-in backend
/// config or a caller-supplied store set.
#[derive(Clone)]
pub(crate) enum StorageSetup {
    Config(StorageConfig),
    Custom(CustomStorage),
}

impl StorageSetup {
    pub(crate) fn into_backend(
        self,
        network: Network,
    ) -> Result<Arc<dyn StorageBackend>, SdkError> {
        match self {
            StorageSetup::Config(config) => config.into_backend(network),
            StorageSetup::Custom(stores) => Ok(Arc::new(prebuilt::PrebuiltBackend::new(stores))),
        }
    }
}
