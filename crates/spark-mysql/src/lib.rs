//! `MySQL` storage implementations for the Spark protocol.
//!
//! This crate provides `MySQL`-backed implementations of the `TreeStore` and
//! `TokenOutputStore` traits from `spark-wallet`, suitable for server-side or
//! multi-instance deployments.
//!
//! It also exposes shared `MySQL` infrastructure (connection pooling and a generic
//! migration runner) that can be reused by downstream crates for their own `MySQL`
//! storage needs.
//!
//! Targets `MySQL` 8.0+ (uses native `JSON` type, `CHECK` constraints, and `GET_LOCK`
//! for application-level write serialization).

mod advisory_lock;
pub mod config;
pub mod error;
pub mod migrations;
pub mod pool;
mod token_store;
mod tree_store;

pub use config::{MysqlStorageConfig, default_mysql_storage_config};
pub use error::MysqlError;
pub use token_store::{
    MysqlTokenStore, create_mysql_token_store, create_mysql_token_store_from_pool,
};
pub use tree_store::{MysqlTreeStore, create_mysql_tree_store, create_mysql_tree_store_from_pool};

pub use migrations::{Migration, run_migrations};
pub use pool::{create_pool, map_db_error};

pub use mysql_async;
