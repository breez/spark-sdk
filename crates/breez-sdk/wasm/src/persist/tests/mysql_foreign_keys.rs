//! Verifies that the JS-side MySQL tree-store and token-store migrations
//! honor `MysqlForeignKeyMode` in both directions: `Disabled` leaves no FK
//! constraints behind, `Enforced` creates the composite multi-tenant FKs.
//! Mirrors the native `test_new_with_*_foreign_key_mode` tests in
//! [`spark-mysql`](../../../../../../spark-mysql/src/tree_store.rs).

use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use crate::sdk_builder::{MysqlForeignKeyMode, MysqlStorageConfig};
use crate::token_store::TokenStoreJs;
use crate::tree_store::TreeStoreJs;

#[wasm_bindgen(module = "js/mysql-tree-store")]
extern "C" {
    #[wasm_bindgen(js_name = "createMysqlTreeStore", catch)]
    async fn create_mysql_tree_store(
        config: MysqlStorageConfig,
        identity: &[u8],
        logger: Option<&crate::logger::Logger>,
    ) -> Result<TreeStoreJs, JsValue>;
}

#[wasm_bindgen(module = "js/mysql-token-store")]
extern "C" {
    #[wasm_bindgen(js_name = "createMysqlTokenStore", catch)]
    async fn create_mysql_token_store(
        config: MysqlStorageConfig,
        identity: &[u8],
        logger: Option<&crate::logger::Logger>,
    ) -> Result<TokenStoreJs, JsValue>;
}

#[wasm_bindgen(module = "js/mysql-test-helpers.cjs")]
extern "C" {
    #[wasm_bindgen(js_name = "createTestConnectionString", catch)]
    async fn create_test_connection_string(test_name: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = "countMysqlForeignKeys", catch)]
    async fn count_mysql_foreign_keys(
        connection_string: &str,
        tables: Vec<JsValue>,
    ) -> Result<JsValue, JsValue>;
}

const TEST_IDENTITY: [u8; 33] = [
    0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    0x20,
];

async fn fresh_test_db(test_name: &str) -> String {
    let conn_string_js = create_test_connection_string(test_name)
        .await
        .expect("Failed to create test connection string");
    conn_string_js
        .as_string()
        .expect("Connection string should be a string")
}

fn config_with_mode(connection_string: &str, mode: MysqlForeignKeyMode) -> MysqlStorageConfig {
    let mut config = crate::sdk_builder::default_mysql_storage_config(connection_string);
    config.foreign_key_mode = mode;
    config
}

async fn count_fks(connection_string: &str, tables: &[&str]) -> u64 {
    let table_values: Vec<JsValue> = tables.iter().map(|t| JsValue::from_str(t)).collect();
    let count_js = count_mysql_foreign_keys(connection_string, table_values)
        .await
        .expect("Failed to count foreign keys");
    count_js
        .as_f64()
        .expect("countMysqlForeignKeys should return a number") as u64
}

#[wasm_bindgen_test]
async fn test_tree_store_disabled_foreign_key_mode() {
    let conn_string = fresh_test_db("my_tree_fk_disabled").await;
    let config = config_with_mode(&conn_string, MysqlForeignKeyMode::Disabled);
    let _tree_store = create_mysql_tree_store(config, &TEST_IDENTITY, None)
        .await
        .expect("Failed to create mysql tree store");

    let count = count_fks(
        &conn_string,
        &[
            "brz_tree_leaves",
            "brz_tree_reservations",
            "brz_tree_spent_leaves",
            "brz_tree_swap_status",
        ],
    )
    .await;
    assert_eq!(count, 0, "tree store left foreign keys behind");
}

#[wasm_bindgen_test]
async fn test_token_store_disabled_foreign_key_mode() {
    let conn_string = fresh_test_db("my_token_fk_disabled").await;
    let config = config_with_mode(&conn_string, MysqlForeignKeyMode::Disabled);
    let _token_store = create_mysql_token_store(config, &TEST_IDENTITY, None)
        .await
        .expect("Failed to create mysql token store");

    let count = count_fks(
        &conn_string,
        &[
            "brz_token_metadata",
            "brz_token_outputs",
            "brz_token_reservations",
            "brz_token_spent_outputs",
            "brz_token_swap_status",
        ],
    )
    .await;
    assert_eq!(count, 0, "token store left foreign keys behind");
}

/// `Enforced` mode runs both initial-FK adds and the multi-tenant rewrites:
/// the originals are dropped in migration 2 and replaced by the composite
/// `*_user` variants — final FK count is 1 on brz_tree_leaves, 2 on brz_token_outputs.
#[wasm_bindgen_test]
async fn test_tree_store_enforced_foreign_key_mode() {
    let conn_string = fresh_test_db("my_tree_fk_enforced").await;
    let config = config_with_mode(&conn_string, MysqlForeignKeyMode::Enforced);
    let _tree_store = create_mysql_tree_store(config, &TEST_IDENTITY, None)
        .await
        .expect("Failed to create mysql tree store");

    let count = count_fks(
        &conn_string,
        &[
            "brz_tree_leaves",
            "brz_tree_reservations",
            "brz_tree_spent_leaves",
            "brz_tree_swap_status",
        ],
    )
    .await;
    assert_eq!(count, 1, "tree store did not create expected foreign key");
}

#[wasm_bindgen_test]
async fn test_token_store_enforced_foreign_key_mode() {
    let conn_string = fresh_test_db("my_token_fk_enforced").await;
    let config = config_with_mode(&conn_string, MysqlForeignKeyMode::Enforced);
    let _token_store = create_mysql_token_store(config, &TEST_IDENTITY, None)
        .await
        .expect("Failed to create mysql token store");

    let count = count_fks(
        &conn_string,
        &[
            "brz_token_metadata",
            "brz_token_outputs",
            "brz_token_reservations",
            "brz_token_spent_outputs",
            "brz_token_swap_status",
        ],
    )
    .await;
    assert_eq!(count, 2, "token store did not create expected foreign keys");
}
