use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use crate::persist::{Storage, WasmStorage};

wasm_bindgen_test_configure!(run_in_browser);

// Import the web-storage package
#[wasm_bindgen(module = "/js/web-storage/index.js")]
extern "C" {
    #[wasm_bindgen(js_name = "createDefaultStorage", catch)]
    async fn create_default_storage(
        data_dir: &str,
        logger: Option<&crate::logger::Logger>,
    ) -> Result<Storage, JsValue>;
}

// Helper to create a WasmStorage instance for testing using node-storage
async fn create_test_storage(db_name: &str) -> WasmStorage {
    let storage = create_default_storage(db_name, None)
        .await
        .expect("Failed to create node storage instance");
    WasmStorage { storage }
}

#[wasm_bindgen_test]
async fn test_sqlite_storage() {
    let storage = create_test_storage("sqlite_storage").await;

    breez_sdk_spark::storage_tests::test_sqlite_storage(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_unclaimed_deposits_crud() {
    let storage = create_test_storage("unclaimed_deposits_crud").await;

    breez_sdk_spark::storage_tests::test_unclaimed_deposits_crud(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_deposit_refunds() {
    let storage = create_test_storage("deposit_refunds").await;

    breez_sdk_spark::storage_tests::test_deposit_refunds(Box::new(storage)).await;
}
