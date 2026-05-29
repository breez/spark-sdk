//! `PostgreSQL` storage implementations for the Spark protocol.
//!
//! This crate provides `PostgreSQL`-backed implementations of the `TreeStore`,
//! `TokenOutputStore`, and `SessionStore` traits from `spark-wallet`,
//! suitable for server-side or multi-instance deployments.
//!
//! It also exposes shared `PostgreSQL` infrastructure (connection pooling, TLS
//! configuration, and a generic migration runner) that can be reused by downstream
//! crates for their own `PostgreSQL` storage needs.

mod advisory_lock;
pub mod config;
pub mod error;
pub mod migrations;
pub mod pool;
mod session_store;
mod time_compat;
mod token_store;
mod tree_store;

// Re-export main public API
pub use config::{PoolQueueMode, PostgresStorageConfig, default_postgres_storage_config};
pub use error::PostgresError;
pub use session_store::{
    PostgresSessionStore, create_postgres_session_store, create_postgres_session_store_from_pool,
};
pub use token_store::{
    PostgresTokenStore, create_postgres_token_store, create_postgres_token_store_from_pool,
};
pub use tree_store::{
    PostgresTreeStore, create_postgres_tree_store, create_postgres_tree_store_from_pool,
};

// Re-export pool infrastructure for downstream crates
pub use migrations::{SchemaRenames, run_migrations};
pub use pool::{create_pool, map_db_error, map_pool_error};

// ── Target-routed re-exports of the Postgres protocol crates ─────────────────
//
// Downstream callers (and our own internal modules) should refer to
// `spark_postgres::tokio_postgres` and `spark_postgres::deadpool_postgres`
// (or, internally, `crate::tokio_postgres` / `crate::deadpool_postgres`)
// rather than the upstream crates by name. On native targets these are
// the real crates; on `wasm32-unknown-unknown` they point at `pg-wasm`,
// whose surface mirrors the subset of those crates that the SDK uses.

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use ::deadpool_postgres;
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use ::tokio_postgres;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use pg_wasm::pool as deadpool_postgres;
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use pg_wasm as tokio_postgres;
