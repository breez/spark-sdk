//! Shareable MySQL connection pool wrapper for WASM.

use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::error::WasmResult;
use crate::persist::pool::{JsPool, create_mysql_pool};
use crate::sdk_builder::MysqlStorageConfig;

/// A shareable MySQL connection pool. See [`PostgresConnectionPool`](super::postgres_pool::PostgresConnectionPool)
/// for sharing semantics and lifecycle.
#[wasm_bindgen]
pub struct MysqlConnectionPool {
    pub(crate) inner: Rc<JsPool>,
    pub(crate) run_migration: bool,
}

impl MysqlConnectionPool {
    pub(crate) fn cloned_inner(&self) -> Rc<JsPool> {
        Rc::clone(&self.inner)
    }

    pub(crate) fn run_migration(&self) -> bool {
        self.run_migration
    }
}

/// Creates a shareable MySQL connection pool from the given config.
#[wasm_bindgen(js_name = "createMysqlConnectionPool")]
pub fn create_mysql_connection_pool(config: MysqlStorageConfig) -> WasmResult<MysqlConnectionPool> {
    let run_migration = config.run_migration;
    Ok(MysqlConnectionPool {
        inner: Rc::new(create_mysql_pool(config)?),
        run_migration,
    })
}
