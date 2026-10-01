//! Adapter module bridging `spark-mysql` types with breez-sdk types.
//!
//! Provides:
//! - Error conversions between `spark_mysql::MysqlError` and `StorageError`
//! - Error mapping helpers for `storage.rs`

use std::sync::Arc;

pub use spark_mysql::Migration;
use spark_mysql::mysql_async;
use spark_wallet::{SessionStore, TokenOutputStore, TreeStore};

use super::{MysqlForeignKeyMode, MysqlStorageConfig};
use crate::persist::StorageError;

// ── Error conversions ─────────────────────────────────────────────────────────

impl From<spark_mysql::MysqlError> for StorageError {
    fn from(e: spark_mysql::MysqlError) -> Self {
        match e {
            spark_mysql::MysqlError::Connection(msg) => StorageError::Connection(msg),
            spark_mysql::MysqlError::Initialization(msg) => StorageError::InitializationError(msg),
            spark_mysql::MysqlError::Database(msg) => StorageError::Implementation(msg),
        }
    }
}

impl From<mysql_async::Error> for StorageError {
    fn from(value: mysql_async::Error) -> Self {
        let my_err: spark_mysql::MysqlError = value.into();
        my_err.into()
    }
}

/// Maps a `mysql_async` error to `StorageError`.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_db_error(e: mysql_async::Error) -> StorageError {
    let my_err = spark_mysql::map_db_error(e);
    my_err.into()
}

// ── Pool and migration wrappers ───────────────────────────────────────────────

/// Creates a `MySQL` connection pool from the given configuration.
pub(crate) fn create_pool(config: &MysqlStorageConfig) -> Result<mysql_async::Pool, StorageError> {
    let sm_config: spark_mysql::MysqlStorageConfig = config.clone().into();
    spark_mysql::create_pool(&sm_config).map_err(StorageError::from)
}

pub(super) use spark_mysql::SchemaRenames;

/// Wraps [`spark_mysql::run_migrations`] with `StorageError` mapping.
pub(super) async fn run_migrations(
    pool: &mysql_async::Pool,
    migrations_table: &str,
    migrations: &[Vec<Migration>],
    renames: Option<&SchemaRenames<'_>>,
) -> Result<(), StorageError> {
    spark_mysql::run_migrations(pool, migrations_table, migrations, renames)
        .await
        .map_err(StorageError::from)
}

// ── Store factories ───────────────────────────────────────────────────────────

/// Creates a `MysqlTreeStore` instance for use with the SDK, using an existing pool.
pub(crate) async fn create_mysql_tree_store(
    pool: mysql_async::Pool,
    identity: &[u8],
    run_migration: bool,
    foreign_key_mode: MysqlForeignKeyMode,
) -> Result<Arc<dyn TreeStore>, StorageError> {
    spark_mysql::create_mysql_tree_store_from_pool(
        pool,
        identity,
        run_migration,
        foreign_key_mode.into(),
    )
    .await
    .map_err(StorageError::from)
}

/// Creates a `MysqlTokenStore` instance for use with the SDK, using an existing pool.
pub(crate) async fn create_mysql_token_store(
    pool: mysql_async::Pool,
    identity: &[u8],
    run_migration: bool,
    foreign_key_mode: MysqlForeignKeyMode,
) -> Result<Arc<dyn TokenOutputStore>, StorageError> {
    spark_mysql::create_mysql_token_store_from_pool(
        pool,
        identity,
        run_migration,
        foreign_key_mode.into(),
    )
    .await
    .map_err(StorageError::from)
}

/// Creates a `MysqlSessionStore` instance for use with the SDK, using an existing pool.
pub(crate) async fn create_mysql_session_store(
    pool: mysql_async::Pool,
    identity: &[u8],
    run_migration: bool,
) -> Result<Arc<dyn SessionStore>, StorageError> {
    spark_mysql::create_mysql_session_store_from_pool(pool, identity, run_migration)
        .await
        .map_err(StorageError::from)
}
