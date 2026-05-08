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
    pub(crate) schema_managed_externally: bool,
}

impl MysqlConnectionPool {
    pub(crate) fn cloned_inner(&self) -> Rc<JsPool> {
        Rc::clone(&self.inner)
    }

    pub(crate) fn schema_managed_externally(&self) -> bool {
        self.schema_managed_externally
    }
}

/// Creates a shareable MySQL connection pool from the given config.
#[wasm_bindgen(js_name = "createMysqlConnectionPool")]
pub fn create_mysql_connection_pool(config: MysqlStorageConfig) -> WasmResult<MysqlConnectionPool> {
    let schema_managed_externally = config.schema_managed_externally;
    Ok(MysqlConnectionPool {
        inner: Rc::new(create_mysql_pool(config)?),
        schema_managed_externally,
    })
}
