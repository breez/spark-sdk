use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use crate::sdk_builder::MysqlStorageConfig;
use crate::token_store::{TokenStoreJs, WasmTokenStore};

// Import the mysql-token-store package
#[wasm_bindgen(module = "js/mysql-token-store")]
extern "C" {
    #[wasm_bindgen(js_name = "createMysqlTokenStore", catch)]
    async fn create_mysql_token_store(
        config: MysqlStorageConfig,
        identity: &[u8],
        logger: Option<&crate::logger::Logger>,
    ) -> Result<TokenStoreJs, JsValue>;
}

// Import test helpers
#[wasm_bindgen(module = "js/mysql-test-helpers.cjs")]
extern "C" {
    #[wasm_bindgen(js_name = "createTestConnectionString", catch)]
    async fn create_test_connection_string(test_name: &str) -> Result<JsValue, JsValue>;
}

/// Fixed 33-byte test identity. Each test gets its own isolated DB via
/// `createTestConnectionString`, so a single shared identity is fine.
const TEST_IDENTITY: [u8; 33] = [
    0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    0x20,
];

/// Helper to create a WasmTokenStore instance for testing
async fn create_test_token_store(test_name: &str) -> WasmTokenStore {
    let conn_string_js = create_test_connection_string(test_name)
        .await
        .expect("Failed to create test connection string");
    let conn_string = conn_string_js
        .as_string()
        .expect("Connection string should be a string");

    let config = crate::sdk_builder::default_mysql_storage_config(&conn_string);

    let token_store_js = create_mysql_token_store(config, &TEST_IDENTITY, None)
        .await
        .expect("Failed to create mysql token store instance");
    WasmTokenStore::new(token_store_js)
}

#[wasm_bindgen_test]
async fn test_remove_token_outputs_by_prev_tx_ref() {
    let store = create_test_token_store("pg_token_remove_outputs_prev_tx_ref").await;
    breez_sdk_spark::token_store_tests::test_remove_token_outputs_by_prev_tx_ref(&store).await;
}

#[wasm_bindgen_test]
async fn test_remove_token_outputs_prevents_refresh_re_add() {
    let store = create_test_token_store("pg_token_remove_outputs_prevents_refresh").await;
    breez_sdk_spark::token_store_tests::test_remove_token_outputs_prevents_refresh_re_add(&store)
        .await;
}
