//! `PostgreSQL` storage implementations for the Breez SDK.
//!
//! This module provides `PostgreSQL`-backed storage for the SDK, using
//! `spark-postgres` for shared infrastructure, tree store, and token store
//! functionality.

mod base;
mod storage;

// Re-export public configuration types and functions (with UniFFI annotations)
pub use base::{PoolQueueMode, PostgresStorageConfig, default_postgres_storage_config};

// Re-export store factories and the pool builder
pub(crate) use base::{
    create_pool, create_postgres_session_store, create_postgres_token_store,
    create_postgres_tree_store,
};

// Re-export storage implementation. Public under `test-utils` so wasm
// integration tests can construct a `PostgresStorage` directly against
// pg-wasm and run the shared storage test suite.
#[cfg(not(feature = "test-utils"))]
pub(crate) use storage::PostgresStorage;
#[cfg(feature = "test-utils")]
pub use storage::PostgresStorage;
