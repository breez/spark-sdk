//! `PostgreSQL` storage implementations for the Spark protocol.
//!
//! This crate provides a `PostgreSQL`-backed implementation of the `TreeStore` trait
//! from `spark-wallet`, suitable for server-side or multi-instance deployments.
//!
//! It also exposes shared `PostgreSQL` infrastructure (connection pooling, TLS
//! configuration, and a generic migration runner) that can be reused by downstream
//! crates for their own `PostgreSQL` storage needs.

pub mod config;
pub mod error;
pub mod migrations;
pub mod pool;
mod tree_store;

// Re-export main public API
pub use config::{PoolQueueMode, PostgresStorageConfig, default_postgres_storage_config};
pub use error::PostgresError;
pub use tree_store::{
    PostgresTreeStore, create_postgres_tree_store, create_postgres_tree_store_from_pool,
};

// Re-export pool infrastructure for downstream crates
pub use migrations::run_migrations;
pub use pool::{create_pool, map_db_error, map_pool_error};

pub use deadpool_postgres;
pub use tokio_postgres;
