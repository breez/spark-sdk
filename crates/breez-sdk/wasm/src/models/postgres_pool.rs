//! Shareable Postgres connection pool wrapper for WASM.

use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::error::WasmResult;
use crate::persist::pool::{JsPool, create_postgres_pool};
use crate::sdk_builder::PostgresStorageConfig;

/// A shareable Postgres connection pool.
///
/// Construct via [`create_postgres_connection_pool`] and pass the same handle to multiple
/// `SdkBuilder`s via `withPostgresConnectionPool` to share connections across SDKs.
/// Per-tenant scoping is derived from each SDK's seed.
///
/// The pool's lifecycle is controlled by the integrator: it stays alive as
/// long as any reference is held. `disconnect()` does **not** close the pool.
#[wasm_bindgen]
pub struct PostgresConnectionPool {
    pub(crate) inner: Rc<JsPool>,
}

impl PostgresConnectionPool {
    pub(crate) fn cloned_inner(&self) -> Rc<JsPool> {
        Rc::clone(&self.inner)
    }
}

/// Creates a shareable Postgres connection pool from the given config.
#[wasm_bindgen(js_name = "createPostgresConnectionPool")]
pub fn create_postgres_connection_pool(
    config: PostgresStorageConfig,
) -> WasmResult<PostgresConnectionPool> {
    Ok(PostgresConnectionPool {
        inner: Rc::new(create_postgres_pool(config)?),
    })
}
