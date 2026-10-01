//! Adapter module bridging `spark-postgres` types with breez-sdk types.
//!
//! This module provides:
//! - Error conversions between `spark_postgres::PostgresError` and `StorageError`
//! - Error mapping helpers for `storage.rs`

use std::sync::Arc;

use spark_postgres::deadpool_postgres;
use spark_postgres::tokio_postgres;
use spark_wallet::{SessionStore, TokenOutputStore, TreeStore};

use super::PostgresStorageConfig;
use crate::persist::StorageError;

// ── Error conversions ─────────────────────────────────────────────────────────

impl From<spark_postgres::PostgresError> for StorageError {
    fn from(e: spark_postgres::PostgresError) -> Self {
        match e {
            spark_postgres::PostgresError::Connection(msg) => StorageError::Connection(msg),
            spark_postgres::PostgresError::Initialization(msg) => {
                StorageError::InitializationError(msg)
            }
            spark_postgres::PostgresError::Database(msg) => StorageError::Implementation(msg),
        }
    }
}

impl From<tokio_postgres::Error> for StorageError {
    fn from(value: tokio_postgres::Error) -> Self {
        let pg_err: spark_postgres::PostgresError = value.into();
        pg_err.into()
    }
}

/// Maps a deadpool-postgres pool error to `StorageError`.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_pool_error(e: deadpool_postgres::PoolError) -> StorageError {
    let pg_err = spark_postgres::map_pool_error(e);
    pg_err.into()
}

/// Maps a tokio-postgres database error to `StorageError`.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_db_error(e: tokio_postgres::Error) -> StorageError {
    let pg_err = spark_postgres::map_db_error(e);
    pg_err.into()
}

// ── Pool and migration wrappers ───────────────────────────────────────────────

/// Creates a `PostgreSQL` connection pool from the given configuration.
pub(crate) fn create_pool(
    config: &PostgresStorageConfig,
) -> Result<deadpool_postgres::Pool, StorageError> {
    let sp_config: spark_postgres::PostgresStorageConfig = config.clone().into();
    spark_postgres::create_pool(&sp_config).map_err(StorageError::from)
}

pub(super) use spark_postgres::SchemaRenames;

/// Wraps [`spark_postgres::run_migrations`] with `StorageError` mapping.
pub(super) async fn run_migrations(
    pool: &deadpool_postgres::Pool,
    migrations_table: &str,
    migrations: &[Vec<String>],
    renames: Option<&SchemaRenames<'_>>,
) -> Result<(), StorageError> {
    spark_postgres::run_migrations(pool, migrations_table, migrations, renames)
        .await
        .map_err(StorageError::from)
}

// ── Store factories ───────────────────────────────────────────────────────────

/// Creates a `PostgresTreeStore` instance for use with the SDK, using an existing pool.
pub(crate) async fn create_postgres_tree_store(
    pool: deadpool_postgres::Pool,
    identity: &[u8],
    run_migration: bool,
) -> Result<Arc<dyn TreeStore>, StorageError> {
    spark_postgres::create_postgres_tree_store_from_pool(pool, identity, run_migration)
        .await
        .map_err(StorageError::from)
}

/// Creates a `PostgresTokenStore` instance for use with the SDK, using an existing pool.
pub(crate) async fn create_postgres_token_store(
    pool: deadpool_postgres::Pool,
    identity: &[u8],
    run_migration: bool,
) -> Result<Arc<dyn TokenOutputStore>, StorageError> {
    spark_postgres::create_postgres_token_store_from_pool(pool, identity, run_migration)
        .await
        .map_err(StorageError::from)
}

/// Creates a `PostgresSessionStore` instance for use with the SDK, using an existing pool.
pub(crate) async fn create_postgres_session_store(
    pool: deadpool_postgres::Pool,
    identity: &[u8],
    run_migration: bool,
) -> Result<Arc<dyn SessionStore>, StorageError> {
    spark_postgres::create_postgres_session_store_from_pool(pool, identity, run_migration)
        .await
        .map_err(StorageError::from)
}
