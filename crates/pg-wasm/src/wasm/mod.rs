//! Wasm-only implementation of the tokio-postgres-shaped client.
//!
//! Module layout mirrors the public types: each user-visible type
//! (`Client`, `Row`, `Statement`, `Error`, `Config`) lives in its own
//! module, and `js` holds the wasm-bindgen extern bindings to the
//! `pg-wasm-bridge.cjs` JS shim.

mod client;
mod config;
mod error;
mod js;
pub mod pool;
mod row;
mod statement;
mod transaction;

pub use client::Client;
pub use config::{connect, Config};
pub use error::Error;
pub use pool::{Object, Pool};
pub use row::{Column, Row, RowIndex};
pub use statement::{Statement, ToStatement};
pub use transaction::Transaction;
