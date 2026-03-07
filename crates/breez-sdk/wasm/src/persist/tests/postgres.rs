use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use crate::persist::{Storage, WasmStorage};

use crate::sdk_builder::PostgresStorageConfig;

// Import the postgres-storage package
#[wasm_bindgen(module = "js/postgres-storage")]
extern "C" {
    #[wasm_bindgen(js_name = "createPostgresStorage", catch)]
    async fn create_postgres_storage(
        config: PostgresStorageConfig,
        logger: Option<&crate::logger::Logger>,
    ) -> Result<Storage, JsValue>;
}

// Import test helpers
#[wasm_bindgen(module = "js/postgres-test-helpers.cjs")]
extern "C" {
    #[wasm_bindgen(js_name = "createTestConnectionString", catch)]
    async fn create_test_connection_string(test_name: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = "createOldV1Database", catch)]
    async fn create_old_v1_database(test_name: &str) -> Result<JsValue, JsValue>;
}

/// Helper to create a WasmStorage instance for testing using postgres-storage
async fn create_test_storage(test_name: &str) -> WasmStorage {
    let conn_string_js = create_test_connection_string(test_name)
        .await
        .expect("Failed to create test connection string");
    let conn_string = conn_string_js
        .as_string()
        .expect("Connection string should be a string");

    let config = crate::sdk_builder::default_postgres_storage_config(&conn_string);

    let storage = create_postgres_storage(config, None)
        .await
        .expect("Failed to create postgres storage instance");
    WasmStorage { storage }
}

#[wasm_bindgen_test]
async fn test_storage() {
    let storage = create_test_storage("pg_storage").await;
    breez_sdk_spark::storage_tests::test_storage(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_unclaimed_deposits_crud() {
    let storage = create_test_storage("pg_unclaimed_deposits_crud").await;
    breez_sdk_spark::storage_tests::test_unclaimed_deposits_crud(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_deposit_refunds() {
    let storage = create_test_storage("pg_deposit_refunds").await;
    breez_sdk_spark::storage_tests::test_deposit_refunds(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_type_filtering() {
    let storage = create_test_storage("pg_payment_type_filtering").await;
    breez_sdk_spark::storage_tests::test_payment_type_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_status_filtering() {
    let storage = create_test_storage("pg_payment_status_filtering").await;
    breez_sdk_spark::storage_tests::test_payment_status_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_asset_filtering() {
    let storage = create_test_storage("pg_asset_filtering").await;
    breez_sdk_spark::storage_tests::test_asset_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_timestamp_filtering() {
    let storage = create_test_storage("pg_timestamp_filtering").await;
    breez_sdk_spark::storage_tests::test_timestamp_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_combined_filters() {
    let storage = create_test_storage("pg_combined_filters").await;
    breez_sdk_spark::storage_tests::test_combined_filters(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_sort_order() {
    let storage = create_test_storage("pg_sort_order").await;
    breez_sdk_spark::storage_tests::test_sort_order(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_metadata() {
    let storage = create_test_storage("pg_payment_metadata").await;
    breez_sdk_spark::storage_tests::test_payment_metadata(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_metadata_merge() {
    let storage = create_test_storage("pg_payment_metadata_merge").await;
    breez_sdk_spark::storage_tests::test_payment_metadata_merge(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_details_update_persistence() {
    let storage = create_test_storage("pg_payment_details_update").await;
    breez_sdk_spark::storage_tests::test_payment_details_update_persistence(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn test_pending_lnurl_preimages() {
    let storage = create_test_storage("pg_pending_lnurl_preimages").await;
    breez_sdk_spark::storage_tests::test_pending_lnurl_preimages(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_spark_htlc_status_filtering() {
    let storage = create_test_storage("pg_spark_htlc_status_filtering").await;
    breez_sdk_spark::storage_tests::test_spark_htlc_status_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_lightning_htlc_details_and_status_filtering() {
    let storage = create_test_storage("pg_lightning_htlc_details").await;
    breez_sdk_spark::storage_tests::test_lightning_htlc_details_and_status_filtering(Box::new(
        storage,
    ))
    .await;
}

#[wasm_bindgen_test]
async fn test_conversion_refund_needed_filtering() {
    let storage = create_test_storage("pg_conversion_refund_needed_filtering").await;
    breez_sdk_spark::storage_tests::test_conversion_refund_needed_filtering(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn test_token_transaction_type_filtering() {
    let storage = create_test_storage("pg_token_tx_type_filtering").await;
    breez_sdk_spark::storage_tests::test_token_transaction_type_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_contacts_crud() {
    let storage = create_test_storage("pg_contacts_crud").await;
    breez_sdk_spark::storage_tests::test_contacts_crud(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_sync_storage() {
    let storage = create_test_storage("pg_sync_storage").await;
    breez_sdk_spark::storage_tests::test_sync_storage(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_migration_bigint_tagging() {
    // Step 1: Create DB seeded at migration v1 with untagged u128 string values
    let conn_string_js = create_old_v1_database("pg_bigint_migration")
        .await
        .expect("Failed to create old v1 database");
    let conn_string = conn_string_js
        .as_string()
        .expect("Connection string should be a string");

    // Step 2: Open storage (triggers migration v2 - BigInt tagging)
    let config = crate::sdk_builder::default_postgres_storage_config(&conn_string);
    let storage = create_postgres_storage(config, None)
        .await
        .expect("Failed to create postgres storage instance");
    let storage = WasmStorage { storage };

    // Step 3: Verify large values (> u64::MAX) survived migration
    let payment =
        breez_sdk_spark::Storage::get_payment_by_id(&storage, "bigint-token-payment".to_string())
            .await
            .expect("Failed to get migrated payment");

    match &payment.details {
        Some(breez_sdk_spark::PaymentDetails::Token {
            metadata,
            conversion_info,
            ..
        }) => {
            assert_eq!(
                metadata.max_supply,
                u128::MAX,
                "maxSupply > u64::MAX should be migrated correctly"
            );
            assert_eq!(metadata.ticker, "TST");

            let info = conversion_info
                .as_ref()
                .expect("conversion_info should be present");
            assert_eq!(
                info.fee,
                Some(u128::from(u64::MAX) + 1),
                "conversion fee > u64::MAX should be migrated correctly"
            );
        }
        _ => panic!("Expected Token payment details for bigint-token-payment"),
    }

    // Step 4: Verify small values (< u64::MAX) also survived migration
    let payment_small = breez_sdk_spark::Storage::get_payment_by_id(
        &storage,
        "bigint-token-payment-small".to_string(),
    )
    .await
    .expect("Failed to get migrated small payment");

    match &payment_small.details {
        Some(breez_sdk_spark::PaymentDetails::Token {
            metadata,
            conversion_info,
            ..
        }) => {
            assert_eq!(
                metadata.max_supply, 1_000_000u128,
                "small maxSupply should be migrated correctly"
            );
            assert_eq!(metadata.ticker, "SML");

            let info = conversion_info
                .as_ref()
                .expect("conversion_info should be present");
            assert_eq!(
                info.fee,
                Some(500u128),
                "small conversion fee should be migrated correctly"
            );
        }
        _ => panic!("Expected Token payment details for bigint-token-payment-small"),
    }
}
