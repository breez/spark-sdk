//! `PostgreSQL` storage implementations for the Breez SDK.
//!
//! This module provides `PostgreSQL`-backed storage implementations for the
//! main SDK storage (`PostgresStorage`), the tree store (`PostgresTreeStore`),
//! and the token output store (`PostgresTokenStore`).
//!
//! All implementations share common infrastructure for connection pooling,
//! TLS configuration, and error mapping.

mod base;
mod storage;
mod token_store;
mod tree_store;

// Re-export public configuration types and functions
pub use base::{
    PoolQueueMode, PostgresStorageConfig, create_pool, default_postgres_storage_config,
};

// Re-export storage implementations
pub(crate) use storage::PostgresStorage;
pub(crate) use token_store::create_postgres_token_store;
pub(crate) use tree_store::create_postgres_tree_store;
