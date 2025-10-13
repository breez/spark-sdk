use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::js_sys::Promise;
use wasm_bindgen_test::*;

use crate::persist::{Storage, WasmStorage};

// Import the node-storage package
#[wasm_bindgen(module = "js/node-storage")]
extern "C" {
    #[wasm_bindgen(js_name = "createDefaultStorage", catch)]
    async fn create_default_storage(
        data_dir: &str,
        logger: Option<&crate::logger::Logger>,
    ) -> Result<Storage, JsValue>;
}

// Import file system utilities
#[wasm_bindgen(module = "js/fs-utils.cjs")]
extern "C" {
    #[wasm_bindgen(js_name = "removeDirAll", catch)]
    fn remove_dir_all(dir_path: &str) -> Result<Promise, JsValue>;
}

// Helper to create a WasmStorage instance for testing using node-storage
async fn create_test_storage(dir_name: &str) -> WasmStorage {
    let data_dir = format!("/tmp/breez-sdk-node-storage-test-{}", dir_name);

    // Ensure the data_dir is cleared before each test
    let future = JsFuture::from(remove_dir_all(&data_dir).expect("Failed to remove test data_dir"));
    let _ = future.await.expect("Failed to remove test data_dir");

    let storage = create_default_storage(&data_dir, None)
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

#[wasm_bindgen_test]
async fn test_payment_type_filtering() {
    let storage = create_test_storage("payment_type_filtering").await;

    breez_sdk_spark::storage_tests::test_payment_type_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_status_filtering() {
    let storage = create_test_storage("payment_status_filtering").await;

    breez_sdk_spark::storage_tests::test_payment_status_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_asset_filtering() {
    let storage = create_test_storage("asset_filtering").await;

    breez_sdk_spark::storage_tests::test_asset_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_timestamp_filtering() {
    let storage = create_test_storage("timestamp_filtering").await;

    breez_sdk_spark::storage_tests::test_timestamp_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_combined_filters() {
    let storage = create_test_storage("combined_filters").await;

    breez_sdk_spark::storage_tests::test_combined_filters(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_sort_order() {
    let storage = create_test_storage("sort_order").await;

    breez_sdk_spark::storage_tests::test_sort_order(Box::new(storage)).await;
}
