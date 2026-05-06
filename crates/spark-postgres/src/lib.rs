//! `PostgreSQL` storage implementations for the Spark protocol.
//!
//! This crate provides `PostgreSQL`-backed implementations of the `TreeStore` and
//! `TokenOutputStore` traits from `spark-wallet`, suitable for server-side or
//! multi-instance deployments.
//!
//! It also exposes shared `PostgreSQL` infrastructure (connection pooling, TLS
//! configuration, and a generic migration runner) that can be reused by downstream
//! crates for their own `PostgreSQL` storage needs.

mod advisory_lock;
pub mod config;
pub mod error;
pub mod migrations;
pub mod pool;
mod token_store;
mod tree_store;

// Re-export main public API
pub use config::{PoolQueueMode, PostgresStorageConfig, default_postgres_storage_config};
pub use error::PostgresError;
pub use token_store::{
    PostgresTokenStore, create_postgres_token_store, create_postgres_token_store_from_pool,
};
pub use tree_store::{
    PostgresTreeStore, create_postgres_tree_store, create_postgres_tree_store_from_pool,
};

// Re-export pool infrastructure for downstream crates
pub use migrations::run_migrations;
pub use pool::{create_pool, map_db_error, map_pool_error};

pub use deadpool_postgres;
pub use tokio_postgres;
