//! `PostgreSQL` storage implementations for the Breez SDK.
//!
//! This module provides `PostgreSQL`-backed storage for the SDK, using
//! `spark-postgres` for shared infrastructure, tree store, and token store
//! functionality.

#[cfg(feature = "postgres")]
mod base;
mod config;
#[cfg(feature = "postgres")]
mod storage;

// Re-export public configuration types and functions (with UniFFI annotations)
pub use config::{PoolQueueMode, PostgresStorageConfig, default_postgres_storage_config};

// Re-export store factories and the pool builder
#[cfg(feature = "postgres")]
pub(crate) use base::{
    create_pool, create_postgres_session_store, create_postgres_token_store,
    create_postgres_tree_store,
};

// Re-export storage implementation
#[cfg(feature = "postgres")]
pub(crate) use storage::PostgresStorage;
