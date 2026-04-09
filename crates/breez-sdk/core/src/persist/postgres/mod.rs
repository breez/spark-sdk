//! `PostgreSQL` storage implementations for the Breez SDK.
//!
//! This module provides `PostgreSQL`-backed storage for the SDK, using
//! `spark-postgres` for shared infrastructure, tree store, and token store
//! functionality.

mod base;
mod storage;

// Re-export public configuration types and functions (with UniFFI annotations)
pub use base::{PoolQueueMode, PostgresStorageConfig, default_postgres_storage_config};

// Re-export pool factory and store factories
pub(crate) use base::{create_pool, create_postgres_token_store, create_postgres_tree_store};

// Re-export storage implementation
pub(crate) use storage::PostgresStorage;
