use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use crate::persist::{Storage, WasmStorage};

use crate::sdk_builder::MysqlStorageConfig;

// Import the mysql-storage package
#[wasm_bindgen(module = "js/mysql-storage")]
extern "C" {
    #[wasm_bindgen(js_name = "createMysqlStorage", catch)]
    async fn create_mysql_storage(
        config: MysqlStorageConfig,
        identity: &[u8],
        logger: Option<&crate::logger::Logger>,
    ) -> Result<Storage, JsValue>;
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

/// Helper to create a `WasmStorage` instance for testing using mysql-storage.
async fn create_test_storage(test_name: &str) -> WasmStorage {
    let conn_string_js = create_test_connection_string(test_name)
        .await
        .expect("Failed to create test connection string");
    let conn_string = conn_string_js
        .as_string()
        .expect("Connection string should be a string");

    let config = crate::sdk_builder::default_mysql_storage_config(&conn_string);

    let storage = create_mysql_storage(config, &TEST_IDENTITY, None)
        .await
        .expect("Failed to create mysql storage instance");
    WasmStorage { storage }
}

#[wasm_bindgen_test]
async fn test_storage() {
    let storage = create_test_storage("my_storage").await;
    breez_sdk_spark::storage_tests::test_storage(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_unclaimed_deposits_crud() {
    let storage = create_test_storage("my_unclaimed_deposits_crud").await;
    breez_sdk_spark::storage_tests::test_unclaimed_deposits_crud(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_deposit_refunds() {
    let storage = create_test_storage("my_deposit_refunds").await;
    breez_sdk_spark::storage_tests::test_deposit_refunds(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_type_filtering() {
    let storage = create_test_storage("my_payment_type_filtering").await;
    breez_sdk_spark::storage_tests::test_payment_type_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_status_filtering() {
    let storage = create_test_storage("my_payment_status_filtering").await;
    breez_sdk_spark::storage_tests::test_payment_status_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_asset_filtering() {
    let storage = create_test_storage("my_asset_filtering").await;
    breez_sdk_spark::storage_tests::test_asset_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_timestamp_filtering() {
    let storage = create_test_storage("my_timestamp_filtering").await;
    breez_sdk_spark::storage_tests::test_timestamp_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_combined_filters() {
    let storage = create_test_storage("my_combined_filters").await;
    breez_sdk_spark::storage_tests::test_combined_filters(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_sort_order() {
    let storage = create_test_storage("my_sort_order").await;
    breez_sdk_spark::storage_tests::test_sort_order(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_metadata() {
    let storage = create_test_storage("my_payment_metadata").await;
    breez_sdk_spark::storage_tests::test_payment_metadata(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_metadata_merge() {
    let storage = create_test_storage("my_payment_metadata_merge").await;
    breez_sdk_spark::storage_tests::test_payment_metadata_merge(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_details_update_persistence() {
    let storage = create_test_storage("my_payment_details_update").await;
    breez_sdk_spark::storage_tests::test_payment_details_update_persistence(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn test_spark_htlc_status_filtering() {
    let storage = create_test_storage("my_spark_htlc_status_filtering").await;
    breez_sdk_spark::storage_tests::test_spark_htlc_status_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_lightning_htlc_details_and_status_filtering() {
    let storage = create_test_storage("my_lightning_htlc_details").await;
    breez_sdk_spark::storage_tests::test_lightning_htlc_details_and_status_filtering(Box::new(
        storage,
    ))
    .await;
}

#[wasm_bindgen_test]
async fn test_conversion_refund_needed_filtering() {
    let storage = create_test_storage("my_conversion_refund_needed_filtering").await;
    breez_sdk_spark::storage_tests::test_conversion_refund_needed_filtering(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn test_token_transaction_type_filtering() {
    let storage = create_test_storage("my_token_tx_type_filtering").await;
    breez_sdk_spark::storage_tests::test_token_transaction_type_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_contacts_crud() {
    let storage = create_test_storage("my_contacts_crud").await;
    breez_sdk_spark::storage_tests::test_contacts_crud(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_conversion_status_persistence() {
    let storage = create_test_storage("my_conversion_status_persistence").await;
    breez_sdk_spark::storage_tests::test_conversion_status_persistence(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_sync_storage() {
    let storage = create_test_storage("my_sync_storage").await;
    breez_sdk_spark::storage_tests::test_sync_storage(Box::new(storage)).await;
}
