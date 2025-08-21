use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use crate::{
    models::{DepositInfo, Payment, PaymentMetadata, UpdateDepositPayload},
    persist::{Storage, WasmStorage},
};

// Import the Storage trait to use its methods
use breez_sdk_spark::Storage as CoreStorage;

#[cfg(feature = "browser-tests")]
wasm_bindgen_test_configure!(run_in_browser);

// Mock JavaScript Storage implementation for testing
#[wasm_bindgen(module = "/tests/mock_storage.js")]
extern "C" {
    #[wasm_bindgen(js_name = "MockStorage")]
    type MockStorage;

    #[wasm_bindgen(constructor)]
    fn new() -> MockStorage;

    // Cached Items
    #[wasm_bindgen(structural, method, js_name = getCachedItem, catch)]
    fn get_cached_item(this: &MockStorage, key: String) -> Result<Option<String>, JsValue>;

    #[wasm_bindgen(structural, method, js_name = setCachedItem, catch)]
    fn set_cached_item(this: &MockStorage, key: String, value: String) -> Result<(), JsValue>;

    // Payments
    #[wasm_bindgen(structural, method, js_name = listPayments, catch)]
    fn list_payments(
        this: &MockStorage,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<Vec<Payment>, JsValue>;

    #[wasm_bindgen(structural, method, js_name = insertPayment, catch)]
    fn insert_payment(this: &MockStorage, payment: Payment) -> Result<(), JsValue>;

    #[wasm_bindgen(structural, method, js_name = setPaymentMetadata, catch)]
    fn set_payment_metadata(
        this: &MockStorage,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), JsValue>;

    #[wasm_bindgen(structural, method, js_name = getPaymentById, catch)]
    fn get_payment_by_id(this: &MockStorage, id: String) -> Result<Payment, JsValue>;

    // Unclaimed Deposits
    #[wasm_bindgen(structural, method, js_name = addDeposit, catch)]
    fn add_deposit(
        this: &MockStorage,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), JsValue>;

    #[wasm_bindgen(structural, method, js_name = deleteDeposit, catch)]
    fn delete_deposit(this: &MockStorage, txid: String, vout: u32) -> Result<(), JsValue>;

    #[wasm_bindgen(structural, method, js_name = listDeposits, catch)]
    fn list_deposits(this: &MockStorage) -> Result<Vec<DepositInfo>, JsValue>;

    #[wasm_bindgen(structural, method, js_name = updateDeposit, catch)]
    fn update_deposit(
        this: &MockStorage,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<(), JsValue>;

    // Test utilities
    #[wasm_bindgen(structural, method, js_name = clear)]
    fn clear(this: &MockStorage);

    #[wasm_bindgen(structural, method, js_name = getOperationCount)]
    fn get_operation_count(this: &MockStorage) -> u32;
}

// Helper to create a WasmStorage instance for testing
fn create_test_storage() -> WasmStorage {
    let mock = MockStorage::new();
    // Cast MockStorage to Storage using js_cast
    let storage: Storage = mock.unchecked_into();
    WasmStorage { storage }
}

// Helper to create test data
fn create_test_payment() -> breez_sdk_spark::Payment {
    breez_sdk_spark::Payment {
        id: "test-payment-123".to_string(),
        payment_type: breez_sdk_spark::PaymentType::Receive,
        status: breez_sdk_spark::PaymentStatus::Completed,
        amount: 50000,
        fees: 100,
        timestamp: 1234567890,
        details: Some(breez_sdk_spark::PaymentDetails::Lightning {
            description: Some("Test payment".to_string()),
            preimage: None,
            invoice: "test_invoice".to_string(),
            payment_hash: "test_hash".to_string(),
            destination_pubkey: "test_pubkey".to_string(),
            lnurl_pay_info: None,
        }),
        method: breez_sdk_spark::PaymentMethod::Lightning,
    }
}

fn create_test_deposit() -> breez_sdk_spark::DepositInfo {
    breez_sdk_spark::DepositInfo {
        txid: "abc123def456".to_string(),
        vout: 0,
        amount_sats: 100000,
        refund_tx: None,
        refund_tx_id: None,
        claim_error: None,
    }
}

#[wasm_bindgen_test]
fn test_cached_items_basic_operations() {
    let storage = create_test_storage();

    // Test setting and getting cached items
    storage
        .set_cached_item("test-key".to_string(), "test-value".to_string())
        .expect("Should set cached item");

    let value = storage
        .get_cached_item("test-key".to_string())
        .expect("Should get cached item");

    assert_eq!(value, Some("test-value".to_string()));
}

#[wasm_bindgen_test]
fn test_cached_items_non_existent_key() {
    let storage = create_test_storage();

    let value = storage
        .get_cached_item("non-existent-key".to_string())
        .expect("Should handle non-existent key");

    assert_eq!(value, None);
}

#[wasm_bindgen_test]
fn test_cached_items_overwrite() {
    let storage = create_test_storage();

    // Set initial value
    storage
        .set_cached_item("key".to_string(), "value1".to_string())
        .expect("Should set initial value");

    // Overwrite with new value
    storage
        .set_cached_item("key".to_string(), "value2".to_string())
        .expect("Should overwrite value");

    let value = storage
        .get_cached_item("key".to_string())
        .expect("Should get updated value");

    assert_eq!(value, Some("value2".to_string()));
}

#[wasm_bindgen_test]
fn test_payment_insertion_and_retrieval() {
    let storage = create_test_storage();
    let payment = create_test_payment();

    // Insert payment
    storage
        .insert_payment(payment.clone())
        .expect("Should insert payment");

    // Retrieve payment
    let retrieved = storage
        .get_payment_by_id("test-payment-123".to_string())
        .expect("Should retrieve payment");

    assert_eq!(retrieved.id, payment.id);
    assert_eq!(retrieved.amount, payment.amount);
    assert_eq!(retrieved.payment_type, payment.payment_type);
}

#[wasm_bindgen_test]
fn test_payment_not_found() {
    let storage = create_test_storage();

    let result = storage.get_payment_by_id("non-existent-payment".to_string());
    assert!(
        result.is_err(),
        "Should return error for non-existent payment"
    );
}

#[wasm_bindgen_test]
fn test_payment_metadata() {
    let storage = create_test_storage();
    let payment = create_test_payment();

    // Insert payment first
    storage
        .insert_payment(payment)
        .expect("Should insert payment");

    // Set metadata
    let metadata = breez_sdk_spark::PaymentMetadata {
        lnurl_pay_info: Some(breez_sdk_spark::LnurlPayInfo {
            ln_address: Some("lnurl-address".to_string()),
            comment: None,
            domain: None,
            metadata: None,
            processed_success_action: None,
            raw_success_action: None,
        }),
    };

    storage
        .set_payment_metadata("test-payment-123".to_string(), metadata)
        .expect("Should set payment metadata");

    // Retrieve and verify metadata was set
    let updated = storage
        .get_payment_by_id("test-payment-123".to_string())
        .expect("Should retrieve updated payment");

    assert!(matches!(
        updated.details.unwrap(),
        breez_sdk_spark::PaymentDetails::Lightning {lnurl_pay_info: Some(lnurl_pay_info), ..} if lnurl_pay_info.ln_address == Some("lnurl-address".to_string())
    ));
}

#[wasm_bindgen_test]
fn test_payment_listing() {
    let storage = create_test_storage();

    // Insert multiple payments
    for i in 0..5 {
        let mut payment = create_test_payment();
        payment.id = format!("payment-{}", i);
        payment.amount = 1000 * (i as u64 + 1);
        storage
            .insert_payment(payment)
            .expect("Should insert payment");
    }

    // Test listing all payments
    let all_payments = storage
        .list_payments(None, None)
        .expect("Should list all payments");
    assert_eq!(all_payments.len(), 5);

    // Test pagination
    let first_page = storage
        .list_payments(Some(0), Some(2))
        .expect("Should list first page");
    assert_eq!(first_page.len(), 2);

    let second_page = storage
        .list_payments(Some(2), Some(2))
        .expect("Should list second page");
    assert_eq!(second_page.len(), 2);
}

#[wasm_bindgen_test]
fn test_adding_deposits() {
    let storage = create_test_storage();
    let deposit = create_test_deposit();

    // Add unclaimed deposit
    storage
        .add_deposit(deposit.txid.clone(), deposit.vout, deposit.amount_sats)
        .expect("Should add deposit");

    // List unclaimed deposits
    let deposits = storage.list_deposits().expect("Should list deposits");

    assert_eq!(deposits.len(), 1);
    assert_eq!(deposits[0].txid, deposit.txid);
    assert_eq!(deposits[0].vout, deposit.vout);
    assert_eq!(deposits[0].amount_sats, deposit.amount_sats);
}

#[wasm_bindgen_test]
fn test_deleting_deposit() {
    let storage = create_test_storage();
    let deposit = create_test_deposit();

    // Add deposit
    storage
        .add_deposit(deposit.txid.clone(), deposit.vout, deposit.amount_sats)
        .expect("Should add deposit");

    // Verify it's there
    let deposits = storage.list_deposits().expect("Should list deposits");
    assert_eq!(deposits.len(), 1);

    // Remove deposit
    storage
        .delete_deposit(deposit.txid, deposit.vout)
        .expect("Should delete deposit");

    // Verify it's gone
    let deposits = storage
        .list_deposits()
        .expect("Should list deposits after removal");
    assert_eq!(deposits.len(), 0);
}

#[wasm_bindgen_test]
fn test_updating_deposit() {
    let storage = create_test_storage();
    let deposit = create_test_deposit();

    // Add deposit first
    storage
        .add_deposit(deposit.txid.clone(), deposit.vout, deposit.amount_sats)
        .expect("Should add deposit");

    // Update deposit refund
    storage
        .update_deposit(
            deposit.txid.clone(),
            deposit.vout,
            breez_sdk_spark::UpdateDepositPayload::Refund {
                refund_txid: "refund-tx-123".to_string(),
                refund_tx: "refund-transaction-data".to_string(),
            },
        )
        .expect("Should update deposit refund");

    // Get deposit refund
    let retrieved = storage.list_deposits().expect("Should get deposit refund");

    assert_eq!(retrieved.len(), 1);
    assert_eq!(
        retrieved[0].refund_tx,
        Some("refund-transaction-data".to_string())
    );
    assert_eq!(retrieved[0].refund_tx_id, Some("refund-tx-123".to_string()));
    assert_eq!(retrieved[0].claim_error, None);

    // Update deposit claim error
    storage
        .update_deposit(
            deposit.txid.clone(),
            deposit.vout,
            breez_sdk_spark::UpdateDepositPayload::ClaimError {
                error: breez_sdk_spark::DepositClaimError::Generic {
                    message: "test error".to_string(),
                },
            },
        )
        .expect("Should update deposit claim error");

    // Get deposit claim error
    let retrieved = storage
        .list_deposits()
        .expect("Should get deposit claim error");

    assert_eq!(retrieved.len(), 1);
    assert_eq!(
        retrieved[0].claim_error,
        Some(breez_sdk_spark::DepositClaimError::Generic {
            message: "test error".to_string(),
        })
    );
}

#[wasm_bindgen_test]
fn test_multiple_deposits_same_txid_different_vout() {
    let storage = create_test_storage();

    // Add 2 deposits with same txid but different vout
    storage
        .add_deposit("same-txid".to_string(), 0, 10000)
        .expect("Should add first deposit");
    storage
        .add_deposit("same-txid".to_string(), 1, 20000)
        .expect("Should add second deposit");

    // Verify both exist
    let deposits = storage.list_deposits().expect("Should list deposits");
    assert_eq!(deposits.len(), 2);

    // Remove only one
    storage
        .delete_deposit("same-txid".to_string(), 0)
        .expect("Should remove first deposit");

    // Verify only one remains
    let remaining = storage
        .list_deposits()
        .expect("Should list remaining deposits");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].vout, 1);
}

#[wasm_bindgen_test]
fn test_data_type_isolation() {
    let storage = create_test_storage();

    // Add data to all storage types
    storage
        .set_cached_item("cache-key".to_string(), "cache-value".to_string())
        .expect("Should set cached item");

    let payment = create_test_payment();
    storage
        .insert_payment(payment.clone())
        .expect("Should insert payment");

    let deposit = create_test_deposit();
    storage
        .add_deposit(deposit.txid.clone(), deposit.vout, deposit.amount_sats)
        .expect("Should add deposit");

    storage
        .update_deposit(
            deposit.txid.clone(),
            deposit.vout,
            breez_sdk_spark::UpdateDepositPayload::Refund {
                refund_txid: "refund-tx-123".to_string(),
                refund_tx: "refund-transaction-data".to_string(),
            },
        )
        .expect("Should update refund");

    // Verify all data types are independent and accessible
    assert_eq!(
        storage.get_cached_item("cache-key".to_string()).unwrap(),
        Some("cache-value".to_string())
    );

    assert_eq!(
        storage.get_payment_by_id(payment.id.clone()).unwrap().id,
        payment.id
    );

    assert_eq!(storage.list_deposits().unwrap().len(), 1);
}

// Error handling tests
#[wasm_bindgen_test]
fn test_error_handling_conversion() {
    let storage = create_test_storage();

    // Test that JS errors are properly converted to StorageError
    let result = storage.get_payment_by_id("non-existent".to_string());
    assert!(result.is_err());

    match result.unwrap_err() {
        breez_sdk_spark::StorageError::Implementation(_) => {
            // This is the expected error type
        }
        other => panic!("Expected Implementation error, got: {:?}", other),
    }
}
