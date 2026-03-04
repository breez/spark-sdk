//! `PostgreSQL` storage implementations for the Breez SDK.
//!
//! This module provides `PostgreSQL`-backed storage implementations for both
//! the main SDK storage (`PostgresStorage`) and the tree store (`PostgresTreeStore`).
//!
//! Both implementations share common infrastructure for connection pooling,
//! TLS configuration, and error mapping.

mod base;
mod storage;
mod tree_store;

// Re-export public configuration types and functions
pub use base::{PoolQueueMode, PostgresStorageConfig, default_postgres_storage_config};

// Re-export storage implementations
pub(crate) use storage::PostgresStorage;
pub(crate) use tree_store::create_postgres_tree_store;
