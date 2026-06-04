//! Bindings to the JS-side `mysql2.Pool` factory used by the WASM SDK.
//! The actual pool object lives on the JS side; on the Rust side we
//! hold an opaque handle via the `JsPool` extern type.
//!
//! The Postgres backend used to live here too. It now goes through
//! `breez_sdk_spark::postgres_storage`, which on wasm dispatches via
//! `spark-postgres` → `pg-wasm` → node-postgres directly from Rust —
//! no JS pool, no per-store extern bindings.

use wasm_bindgen::prelude::*;

use crate::logger::Logger;
use crate::models::session_store::SessionStore;
use crate::sdk_builder::{MysqlForeignKeyMode, MysqlStorageConfig};
use crate::token_store::TokenStoreJs;
use crate::tree_store::TreeStoreJs;

#[wasm_bindgen]
extern "C" {
    /// JS type representing a `mysql2.Pool` instance.
    pub type JsPool;

    #[wasm_bindgen(js_name = "createMysqlPool", catch)]
    pub fn create_mysql_pool(config: MysqlStorageConfig) -> Result<JsPool, JsValue>;

    #[wasm_bindgen(js_name = "createMysqlStorageWithPool", catch)]
    pub async fn create_mysql_storage_with_pool(
        pool: &JsPool,
        identity: &[u8],
        logger: Option<&Logger>,
        run_migration: bool,
    ) -> Result<crate::persist::Storage, JsValue>;

    #[wasm_bindgen(js_name = "createMysqlTreeStoreWithPool", catch)]
    pub async fn create_mysql_tree_store_with_pool(
        pool: &JsPool,
        identity: &[u8],
        foreign_key_mode: MysqlForeignKeyMode,
        logger: Option<&Logger>,
        run_migration: bool,
    ) -> Result<TreeStoreJs, JsValue>;

    #[wasm_bindgen(js_name = "createMysqlTokenStoreWithPool", catch)]
    pub async fn create_mysql_token_store_with_pool(
        pool: &JsPool,
        identity: &[u8],
        foreign_key_mode: MysqlForeignKeyMode,
        logger: Option<&Logger>,
        run_migration: bool,
    ) -> Result<TokenStoreJs, JsValue>;

    #[wasm_bindgen(js_name = "createMysqlSessionStoreWithPool", catch)]
    pub async fn create_mysql_session_store_with_pool(
        pool: &JsPool,
        identity: &[u8],
        logger: Option<&Logger>,
        run_migration: bool,
    ) -> Result<SessionStore, JsValue>;
}
