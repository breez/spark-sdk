//! Storage parity tests for the **Rust** Postgres path.
//!
//! The sibling `postgres` module runs the SDK storage test suite against
//! the JS-side `postgres-storage` package. This module runs the *same*
//! suite — every test — against the Rust-side `PostgresStorage`, which
//! on wasm dispatches through `spark_postgres` → `pg-wasm` →
//! node-postgres.
//!
//! Both paths share a single Postgres container (started by
//! `postgres-test-helpers.cjs`) and a single test identity. Each test
//! call gets its own fresh database via `createTestConnectionString`,
//! so the two paths coexist without colliding.

use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use spark_postgres::create_pool;
use breez_sdk_spark::PostgresStorage;
use breez_sdk_spark::PostgresStorageConfig;

#[wasm_bindgen(module = "js/postgres-test-helpers.cjs")]
extern "C" {
    #[wasm_bindgen(js_name = "createTestConnectionString", catch)]
    async fn create_test_connection_string(test_name: &str) -> Result<JsValue, JsValue>;
}

const TEST_IDENTITY: [u8; 33] = [
    0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    0x20,
];

async fn make_rust_storage(test_name: &str) -> PostgresStorage {
    let conn_string = create_test_connection_string(test_name)
        .await
        .expect("test connection string")
        .as_string()
        .expect("connection string is a string");

    let config = PostgresStorageConfig::with_defaults(conn_string);
    // The core's `PostgresStorageConfig` is a UniFFI wrapper around
    // `spark_postgres::PostgresStorageConfig`; the `From` impl that
    // bridges them lives in the core's `persist::postgres::base` module.
    let sp_config: spark_postgres::PostgresStorageConfig = config.clone().into();
    let pool = create_pool(&sp_config).expect("create pool");
    PostgresStorage::new_with_pool(pool, &TEST_IDENTITY, true)
        .await
        .expect("PostgresStorage::new_with_pool")
}

#[wasm_bindgen_test]
async fn pgwrust_test_storage() {
    let storage = make_rust_storage("pgwrust_storage").await;
    breez_sdk_spark::storage_tests::test_storage(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_unclaimed_deposits_crud() {
    let storage = make_rust_storage("pgwrust_unclaimed_deposits_crud").await;
    breez_sdk_spark::storage_tests::test_unclaimed_deposits_crud(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_deposit_refunds() {
    let storage = make_rust_storage("pgwrust_deposit_refunds").await;
    breez_sdk_spark::storage_tests::test_deposit_refunds(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_payment_type_filtering() {
    let storage = make_rust_storage("pgwrust_payment_type_filtering").await;
    breez_sdk_spark::storage_tests::test_payment_type_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_payment_status_filtering() {
    let storage = make_rust_storage("pgwrust_payment_status_filtering").await;
    breez_sdk_spark::storage_tests::test_payment_status_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_asset_filtering() {
    let storage = make_rust_storage("pgwrust_asset_filtering").await;
    breez_sdk_spark::storage_tests::test_asset_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_timestamp_filtering() {
    let storage = make_rust_storage("pgwrust_timestamp_filtering").await;
    breez_sdk_spark::storage_tests::test_timestamp_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_combined_filters() {
    let storage = make_rust_storage("pgwrust_combined_filters").await;
    breez_sdk_spark::storage_tests::test_combined_filters(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_sort_order() {
    let storage = make_rust_storage("pgwrust_sort_order").await;
    breez_sdk_spark::storage_tests::test_sort_order(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_payment_metadata() {
    let storage = make_rust_storage("pgwrust_payment_metadata").await;
    breez_sdk_spark::storage_tests::test_payment_metadata(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_payment_metadata_merge() {
    let storage = make_rust_storage("pgwrust_payment_metadata_merge").await;
    breez_sdk_spark::storage_tests::test_payment_metadata_merge(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_payment_details_update_persistence() {
    let storage = make_rust_storage("pgwrust_payment_details_update_persistence").await;
    breez_sdk_spark::storage_tests::test_payment_details_update_persistence(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_payment_terminal_status_is_not_replaced() {
    let storage = make_rust_storage("pgwrust_payment_terminal_status_is_not_replaced").await;
    breez_sdk_spark::storage_tests::test_payment_terminal_status_is_not_replaced(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_spark_htlc_status_filtering() {
    let storage = make_rust_storage("pgwrust_spark_htlc_status_filtering").await;
    breez_sdk_spark::storage_tests::test_spark_htlc_status_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_lightning_htlc_details_and_status_filtering() {
    let storage = make_rust_storage("pgwrust_lightning_htlc_details_and_status_filtering").await;
    breez_sdk_spark::storage_tests::test_lightning_htlc_details_and_status_filtering(Box::new(
        storage,
    ))
    .await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_conversion_refund_needed_filtering() {
    let storage = make_rust_storage("pgwrust_conversion_refund_needed_filtering").await;
    breez_sdk_spark::storage_tests::test_conversion_refund_needed_filtering(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_token_transaction_type_filtering() {
    let storage = make_rust_storage("pgwrust_token_transaction_type_filtering").await;
    breez_sdk_spark::storage_tests::test_token_transaction_type_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_contacts_crud() {
    let storage = make_rust_storage("pgwrust_contacts_crud").await;
    breez_sdk_spark::storage_tests::test_contacts_crud(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_conversion_status_persistence() {
    let storage = make_rust_storage("pgwrust_conversion_status_persistence").await;
    breez_sdk_spark::storage_tests::test_conversion_status_persistence(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn pgwrust_test_sync_storage() {
    let storage = make_rust_storage("pgwrust_sync_storage").await;
    breez_sdk_spark::storage_tests::test_sync_storage(Box::new(storage)).await;
}
