//! Bindings to the JS-side `pg.Pool` / `mysql2.Pool` factories used by the
//! WASM SDK. The actual pool objects live on the JS side; on the Rust side
//! we hold opaque handles via the `JsPool` extern type.

use wasm_bindgen::prelude::*;

use crate::logger::Logger;
use crate::models::session_store::SessionStore;
use crate::sdk_builder::{MysqlForeignKeyMode, MysqlStorageConfig, PostgresStorageConfig};
use crate::token_store::TokenStoreJs;
use crate::tree_store::TreeStoreJs;

#[wasm_bindgen]
extern "C" {
    /// JS type representing a `pg.Pool` / `mysql2.Pool` instance.
    pub type JsPool;

    #[wasm_bindgen(js_name = "createPostgresPool", catch)]
    pub fn create_postgres_pool(config: PostgresStorageConfig) -> Result<JsPool, JsValue>;

    #[wasm_bindgen(js_name = "createPostgresStorageWithPool", catch)]
    pub async fn create_postgres_storage_with_pool(
        pool: &JsPool,
        identity: &[u8],
        logger: Option<&Logger>,
        run_migration: bool,
    ) -> Result<crate::persist::Storage, JsValue>;

    #[wasm_bindgen(js_name = "createPostgresTreeStoreWithPool", catch)]
    pub async fn create_postgres_tree_store_with_pool(
        pool: &JsPool,
        identity: &[u8],
        logger: Option<&Logger>,
        run_migration: bool,
    ) -> Result<TreeStoreJs, JsValue>;

    #[wasm_bindgen(js_name = "createPostgresTokenStoreWithPool", catch)]
    pub async fn create_postgres_token_store_with_pool(
        pool: &JsPool,
        identity: &[u8],
        logger: Option<&Logger>,
        run_migration: bool,
    ) -> Result<TokenStoreJs, JsValue>;

    #[wasm_bindgen(js_name = "createPostgresSessionStoreWithPool", catch)]
    pub async fn create_postgres_session_store_with_pool(
        pool: &JsPool,
        identity: &[u8],
        logger: Option<&Logger>,
        run_migration: bool,
    ) -> Result<SessionStore, JsValue>;

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
