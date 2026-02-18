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

    #[wasm_bindgen(js_name = "createOldV8Database", catch)]
    async fn create_old_v8_database(db_name: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = "createOldV10Database", catch)]
    async fn create_old_v10_database(db_name: &str) -> Result<JsValue, JsValue>;
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
async fn test_payment_metadata_merge() {
    let storage = create_test_storage("test_payment_metadata_merge").await;

    breez_sdk_spark::storage_tests::test_payment_metadata_merge(Box::new(storage)).await;
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
async fn test_lightning_htlc_details_and_status_filtering() {
    let storage = create_test_storage("lightning_htlc_details").await;

    breez_sdk_spark::storage_tests::test_lightning_htlc_details_and_status_filtering(Box::new(
        storage,
    ))
    .await;
}

#[wasm_bindgen_test]
async fn test_conversion_refund_needed_filtering() {
    let storage = create_test_storage("test_conversion_refund_needed_filtering").await;

    breez_sdk_spark::storage_tests::test_conversion_refund_needed_filtering(Box::new(storage))
        .await;
}

#[wasm_bindgen_test]
async fn test_token_transaction_type_filtering() {
    let storage = create_test_storage("token_tx_type_filtering").await;

    breez_sdk_spark::storage_tests::test_token_transaction_type_filtering(Box::new(storage)).await;
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

#[wasm_bindgen_test]
async fn test_migration_from_v8_to_v9() {
    let db_name = "migration_v8_to_v9_test";

    // Step 1: Create old v8 database with token payment WITHOUT txType
    create_old_v8_database(db_name)
        .await
        .expect("Failed to create old v8 format database");

    // Step 2: Open with new code (triggers migration to v9)
    let storage = create_test_storage(db_name).await;

    // Step 3: Verify old token payment was migrated correctly
    let migrated_payment = breez_sdk_spark::Storage::get_payment_by_id(
        &storage,
        "token-migration-test-payment".to_string(),
    )
    .await
    .expect("Failed to get migrated token payment");

    assert_eq!(migrated_payment.id, "token-migration-test-payment");
    assert_eq!(migrated_payment.amount, 5000u128);
    assert_eq!(migrated_payment.fees, 10u128);
    assert_eq!(
        migrated_payment.status,
        breez_sdk_spark::PaymentStatus::Completed
    );
    assert_eq!(
        migrated_payment.payment_type,
        breez_sdk_spark::PaymentType::Send
    );
    assert_eq!(
        migrated_payment.method,
        breez_sdk_spark::PaymentMethod::Token
    );

    // Step 4: Verify token payment details can be parsed and have the default txType
    let details = migrated_payment
        .details
        .expect("Token payment should have details");

    match details {
        breez_sdk_spark::PaymentDetails::Token {
            metadata,
            tx_hash,
            tx_type,
            invoice_details,
            conversion_info,
        } => {
            assert_eq!(metadata.identifier, "test-token-id");
            assert_eq!(metadata.name, "Test Token");
            assert_eq!(metadata.ticker, "TST");
            assert_eq!(metadata.decimals, 8);
            assert_eq!(tx_hash, "0xabcdef1234567890");
            // This is the key assertion: the migration should add default txType
            assert_eq!(
                tx_type,
                breez_sdk_spark::TokenTransactionType::Transfer,
                "Migration should add default txType 'Transfer' to token payments"
            );
            assert_eq!(invoice_details, None);
            assert_eq!(conversion_info, None);
        }
        _ => panic!("Expected Token payment details, got {:?}", details),
    }

    // Step 5: Insert a new token payment with explicit txType
    let new_payment = breez_sdk_spark::Payment {
        id: "new-token-payment-after-migration".to_string(),
        payment_type: breez_sdk_spark::PaymentType::Receive,
        status: breez_sdk_spark::PaymentStatus::Completed,
        amount: 8000u128,
        fees: 20u128,
        timestamp: 1234567893,
        method: breez_sdk_spark::PaymentMethod::Token,
        details: Some(breez_sdk_spark::PaymentDetails::Token {
            metadata: breez_sdk_spark::TokenMetadata {
                identifier: "another-token-id".to_string(),
                issuer_public_key: "02".to_string() + &"b".repeat(64),
                name: "Another Token".to_string(),
                ticker: "ATK".to_string(),
                decimals: 6,
                max_supply: 2000000,
                is_freezable: true,
            },
            tx_hash: "0x1111222233334444".to_string(),
            tx_type: breez_sdk_spark::TokenTransactionType::Mint,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    breez_sdk_spark::Storage::insert_payment(&storage, new_payment.clone())
        .await
        .expect("Failed to insert new token payment");

    // Step 6: List all token payments to verify both work
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

    assert_eq!(
        payments.len(),
        2,
        "Should have both migrated and new token payments"
    );

    // Verify the migrated payment has default Transfer type
    let migrated = payments
        .iter()
        .find(|p| p.id == "token-migration-test-payment")
        .unwrap();
    match &migrated.details {
        Some(breez_sdk_spark::PaymentDetails::Token { tx_type, .. }) => {
            assert_eq!(*tx_type, breez_sdk_spark::TokenTransactionType::Transfer);
        }
        _ => panic!("Expected Token payment details"),
    }

    // Verify the new payment has explicit Mint type
    let new = payments
        .iter()
        .find(|p| p.id == "new-token-payment-after-migration")
        .unwrap();
    match &new.details {
        Some(breez_sdk_spark::PaymentDetails::Token { tx_type, .. }) => {
            assert_eq!(*tx_type, breez_sdk_spark::TokenTransactionType::Mint);
        }
        _ => panic!("Expected Token payment details"),
    }

    // Step 7: Test filtering by token transaction type
    let transfer_filter_request = breez_sdk_spark::ListPaymentsRequest {
        type_filter: None,
        status_filter: None,
        asset_filter: None,
        payment_details_filter: Some(vec![breez_sdk_spark::PaymentDetailsFilter::Token {
            conversion_refund_needed: None,
            tx_hash: None,
            tx_type: Some(breez_sdk_spark::TokenTransactionType::Transfer),
        }]),
        from_timestamp: None,
        to_timestamp: None,
        offset: None,
        limit: None,
        sort_ascending: Some(true),
    };

    let transfer_payments =
        breez_sdk_spark::Storage::list_payments(&storage, transfer_filter_request)
            .await
            .expect("Failed to list transfer payments");

    assert_eq!(
        transfer_payments.len(),
        1,
        "Should find only the Transfer payment"
    );
    assert_eq!(transfer_payments[0].id, "token-migration-test-payment");
}

#[wasm_bindgen_test]
async fn test_migration_from_v10_to_v11() {
    let db_name = "migration_v10_to_v11_test";

    // Step 1: Create old v10 database with Lightning payments WITHOUT htlcDetails
    create_old_v10_database(db_name)
        .await
        .expect("Failed to create old v10 format database");

    // Step 2: Open with new code (triggers migration to v11 - htlc_details backfill)
    let storage = create_test_storage(db_name).await;

    // Step 3: Verify Completed → PreimageShared
    let completed =
        breez_sdk_spark::Storage::get_payment_by_id(&storage, "ln-completed".to_string())
            .await
            .expect("Failed to get completed payment");

    match &completed.details {
        Some(breez_sdk_spark::PaymentDetails::Lightning { htlc_details, .. }) => {
            assert_eq!(
                htlc_details.status,
                breez_sdk_spark::SparkHtlcStatus::PreimageShared,
                "Completed payment should have PreimageShared htlc status"
            );
            assert_eq!(htlc_details.expiry_time, 0);
            assert_eq!(htlc_details.payment_hash, "hash_completed_0123456789abcdef");
            assert_eq!(htlc_details.preimage.as_deref(), Some("preimage_completed"));
        }
        _ => panic!("Expected Lightning payment details for ln-completed"),
    }

    // Step 4: Verify Pending → WaitingForPreimage
    let pending = breez_sdk_spark::Storage::get_payment_by_id(&storage, "ln-pending".to_string())
        .await
        .expect("Failed to get pending payment");

    match &pending.details {
        Some(breez_sdk_spark::PaymentDetails::Lightning { htlc_details, .. }) => {
            assert_eq!(
                htlc_details.status,
                breez_sdk_spark::SparkHtlcStatus::WaitingForPreimage,
                "Pending payment should have WaitingForPreimage htlc status"
            );
            assert_eq!(htlc_details.expiry_time, 0);
            assert_eq!(htlc_details.payment_hash, "hash_pending_0123456789abcdef0");
            assert!(htlc_details.preimage.is_none());
        }
        _ => panic!("Expected Lightning payment details for ln-pending"),
    }

    // Step 5: Verify Failed → Returned
    let failed = breez_sdk_spark::Storage::get_payment_by_id(&storage, "ln-failed".to_string())
        .await
        .expect("Failed to get failed payment");

    match &failed.details {
        Some(breez_sdk_spark::PaymentDetails::Lightning { htlc_details, .. }) => {
            assert_eq!(
                htlc_details.status,
                breez_sdk_spark::SparkHtlcStatus::Returned,
                "Failed payment should have Returned htlc status"
            );
            assert_eq!(htlc_details.expiry_time, 0);
        }
        _ => panic!("Expected Lightning payment details for ln-failed"),
    }

    // Step 6: Verify filtering by htlc_status works on migrated data
    let waiting_payments = breez_sdk_spark::Storage::list_payments(
        &storage,
        breez_sdk_spark::ListPaymentsRequest {
            payment_details_filter: Some(vec![breez_sdk_spark::PaymentDetailsFilter::Lightning {
                htlc_status: Some(vec![breez_sdk_spark::SparkHtlcStatus::WaitingForPreimage]),
            }]),
            ..Default::default()
        },
    )
    .await
    .expect("Failed to list waiting payments");
    assert_eq!(waiting_payments.len(), 1);
    assert_eq!(waiting_payments[0].id, "ln-pending");

    let preimage_shared = breez_sdk_spark::Storage::list_payments(
        &storage,
        breez_sdk_spark::ListPaymentsRequest {
            payment_details_filter: Some(vec![breez_sdk_spark::PaymentDetailsFilter::Lightning {
                htlc_status: Some(vec![breez_sdk_spark::SparkHtlcStatus::PreimageShared]),
            }]),
            ..Default::default()
        },
    )
    .await
    .expect("Failed to list preimage shared payments");
    assert_eq!(preimage_shared.len(), 1);
    assert_eq!(preimage_shared[0].id, "ln-completed");

    let returned = breez_sdk_spark::Storage::list_payments(
        &storage,
        breez_sdk_spark::ListPaymentsRequest {
            payment_details_filter: Some(vec![breez_sdk_spark::PaymentDetailsFilter::Lightning {
                htlc_status: Some(vec![breez_sdk_spark::SparkHtlcStatus::Returned]),
            }]),
            ..Default::default()
        },
    )
    .await
    .expect("Failed to list returned payments");
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0].id, "ln-failed");
}
