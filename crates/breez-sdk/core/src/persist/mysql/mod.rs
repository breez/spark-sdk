//! `MySQL` storage implementations for the Breez SDK.
//!
//! This module provides `MySQL`-backed storage for the SDK, using
//! `spark-mysql` for shared infrastructure, tree store, and token store
//! functionality.
//!
//! Targets `MySQL` 8.0+. See `crates/spark-mysql/src/tree_store.rs` for the
//! SQL syntax differences vs. `PostgreSQL`.

#[cfg(feature = "mysql")]
mod base;
mod config;
#[cfg(feature = "mysql")]
mod storage;

// Re-export public configuration types and functions (with UniFFI annotations).
#[allow(unused_imports)]
pub use config::{MysqlForeignKeyMode, MysqlStorageConfig, default_mysql_storage_config};

// Re-export store factories and the pool builder
#[cfg(feature = "mysql")]
pub(crate) use base::{
    create_mysql_session_store, create_mysql_token_store, create_mysql_tree_store, create_pool,
};

// Re-export storage implementation
#[cfg(feature = "mysql")]
pub(crate) use storage::MysqlStorage;
