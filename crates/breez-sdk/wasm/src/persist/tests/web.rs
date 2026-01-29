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

// Import test helpers
#[wasm_bindgen(module = "/js/test-helpers.js")]
extern "C" {
    #[wasm_bindgen(js_name = "createOldV2Database", catch)]
    async fn create_old_v2_database(db_name: &str) -> Result<JsValue, JsValue>;
}

// Helper to create a WasmStorage instance for testing using node-storage
async fn create_test_storage(db_name: &str) -> WasmStorage {
    let storage = create_default_storage(db_name, None)
        .await
        .expect("Failed to create node storage instance");
    WasmStorage { storage }
}

#[wasm_bindgen_test]
async fn test_storage() {
    let storage = create_test_storage("sqlite_storage").await;

    breez_sdk_spark::storage_tests::test_storage(Box::new(storage)).await;
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

#[wasm_bindgen_test]
async fn test_payment_metadata() {
    let storage = create_test_storage("test_payment_metadata").await;

    breez_sdk_spark::storage_tests::test_payment_metadata(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_payment_details_update_persistence() {
    let storage = create_test_storage("payment_details_update").await;

    breez_sdk_spark::storage_tests::test_payment_details_update_persistence(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn test_spark_htlc_status_filtering() {
    let storage = create_test_storage("spark_htlc_status_filtering").await;

    breez_sdk_spark::storage_tests::test_spark_htlc_status_filtering(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_conversion_refund_needed_filtering() {
    let storage = create_test_storage("test_conversion_refund_needed_filtering").await;

    breez_sdk_spark::storage_tests::test_conversion_refund_needed_filtering(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn test_sync_storage() {
    let storage = create_test_storage("sync_storage").await;

    breez_sdk_spark::storage_tests::test_sync_storage(Box::new(storage)).await;
}

#[wasm_bindgen_test]
async fn test_migration_from_v2_to_v3() {
    let db_name = "migration_v2_to_v3_test";

    // Step 1: Create old v2 database with Number values
    create_old_v2_database(db_name)
        .await
        .expect("Failed to create old format database");

    // Step 2: Open with new code (triggers migration to v3)
    let storage = create_test_storage(db_name).await;

    // Step 3: Verify old payment was migrated correctly
    let old_payment =
        breez_sdk_spark::Storage::get_payment_by_id(&storage, "migration-test-payment".to_string())
            .await
            .expect("Failed to get migrated payment");

    assert_eq!(old_payment.id, "migration-test-payment");
    assert_eq!(
        old_payment.amount, 1000u128,
        "Amount should be migrated to BigInt"
    );
    assert_eq!(
        old_payment.fees, 50u128,
        "Fees should be migrated to BigInt"
    );
    assert_eq!(
        old_payment.status,
        breez_sdk_spark::PaymentStatus::Completed
    );

    // Step 4: Insert new payment with BigInt values
    let new_payment = breez_sdk_spark::Payment {
        id: "new-payment-after-migration".to_string(),
        payment_type: breez_sdk_spark::PaymentType::Receive,
        status: breez_sdk_spark::PaymentStatus::Completed,
        amount: 2000u128,
        fees: 100u128,
        timestamp: 1234567891,
        method: breez_sdk_spark::PaymentMethod::Lightning,
        details: None,
        conversion_details: None,
    };

    breez_sdk_spark::Storage::insert_payment(&storage, new_payment.clone())
        .await
        .expect("Failed to insert new payment");

    // Step 5: List all payments to verify both work
    let request = breez_sdk_spark::ListPaymentsRequest {
        type_filter: None,
        status_filter: None,
        asset_filter: None,
        payment_details_filter: None,
        from_timestamp: None,
        to_timestamp: None,
        offset: None,
        limit: None,
        sort_ascending: Some(true),
    };

    let payments = breez_sdk_spark::Storage::list_payments(&storage, request)
        .await
        .expect("Failed to list payments");

    assert_eq!(payments.len(), 2, "Should have both old and new payments");

    // Verify both payments have correct BigInt values
    let migrated = payments
        .iter()
        .find(|p| p.id == "migration-test-payment")
        .unwrap();
    assert_eq!(migrated.amount, 1000u128);
    assert_eq!(migrated.fees, 50u128);

    let new = payments
        .iter()
        .find(|p| p.id == "new-payment-after-migration")
        .unwrap();
    assert_eq!(new.amount, 2000u128);
    assert_eq!(new.fees, 100u128);
}
