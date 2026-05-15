//! Shareable MySQL connection pool wrapper for WASM.

use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::error::WasmResult;
use crate::persist::pool::{JsPool, create_mysql_pool};
use crate::sdk_builder::{MysqlForeignKeyMode, MysqlStorageConfig};

/// A shareable MySQL connection pool. See [`PostgresConnectionPool`](super::postgres_pool::PostgresConnectionPool)
/// for sharing semantics and lifecycle.
///
/// Snapshots the `foreignKeyMode` from the originating config so every SDK
/// instance built on top of this pool migrates with the same FK policy.
#[wasm_bindgen]
pub struct MysqlConnectionPool {
    pub(crate) inner: Rc<JsPool>,
    pub(crate) run_migration: bool,
    pub(crate) foreign_key_mode: MysqlForeignKeyMode,
}

impl MysqlConnectionPool {
    pub(crate) fn cloned_inner(&self) -> Rc<JsPool> {
        Rc::clone(&self.inner)
    }

    pub(crate) fn run_migration(&self) -> bool {
        self.run_migration
    }

    pub(crate) fn foreign_key_mode(&self) -> MysqlForeignKeyMode {
        self.foreign_key_mode
    }
}

/// Creates a shareable MySQL connection pool from the given config.
#[wasm_bindgen(js_name = "createMysqlConnectionPool")]
pub fn create_mysql_connection_pool(config: MysqlStorageConfig) -> WasmResult<MysqlConnectionPool> {
    let run_migration = config.run_migration;
    let foreign_key_mode = config.foreign_key_mode;
    Ok(MysqlConnectionPool {
        inner: Rc::new(create_mysql_pool(config)?),
        run_migration,
        foreign_key_mode,
    })
}
