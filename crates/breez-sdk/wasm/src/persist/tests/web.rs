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
async fn test_pending_lnurl_preimages() {
    let storage = create_test_storage("pending_lnurl_preimages").await;

    breez_sdk_spark::storage_tests::test_pending_lnurl_preimages(Box::new(storage)).await;
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
async fn test_contacts_crud() {
    let storage = create_test_storage("contacts_crud").await;

    breez_sdk_spark::storage_tests::test_contacts_crud(Box::new(storage)).await;
}

/// Diagnostic test: verify u128::MAX round-trips through serde-wasm-bindgen
/// without going through JS storage. This isolates serde-wasm-bindgen from
/// the JS storage layer.
#[wasm_bindgen_test]
async fn test_u128_serde_wasm_bindgen_roundtrip() {
    use serde::{Deserialize, Serialize};

    // Test 1: Plain struct (no internal tagging)
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct PlainStruct {
        value: u128,
    }

    let original = PlainStruct { value: u128::MAX };
    let js = serde_wasm_bindgen::to_value(&original).expect("serialize plain struct");
    let restored: PlainStruct =
        serde_wasm_bindgen::from_value(js).expect("deserialize plain struct");
    assert_eq!(
        restored.value,
        u128::MAX,
        "Plain struct u128::MAX round-trip failed"
    );

    // Test 2: Internally tagged enum (the problematic case)
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    #[serde(tag = "type")]
    enum Tagged {
        Variant { big_value: u128 },
    }

    let original = Tagged::Variant {
        big_value: u128::MAX,
    };
    let js = serde_wasm_bindgen::to_value(&original).expect("serialize tagged enum");

    // Check the serialized value: is big_value a BigInt on the JS side?
    let big_value_js = js_sys::Reflect::get(&js, &"big_value".into())
        .expect("get big_value from serialized tagged enum");
    assert!(
        big_value_js.is_bigint(),
        "Serialized big_value should be a BigInt, but typeof is: {:?}",
        big_value_js.js_typeof()
    );

    // Check the BigInt value hasn't been truncated during serialization
    let big_value_str = big_value_js.as_string();
    let big_value_via_u128 = u128::try_from(big_value_js.clone());
    assert!(
        big_value_via_u128.is_ok(),
        "BigInt should be convertible to u128"
    );
    assert_eq!(
        big_value_via_u128.unwrap(),
        u128::MAX,
        "Serialized BigInt value should be u128::MAX, string repr: {:?}",
        big_value_str
    );

    let restored: Tagged = serde_wasm_bindgen::from_value(js).expect("deserialize tagged enum");
    assert_eq!(
        restored,
        Tagged::Variant {
            big_value: u128::MAX
        },
        "Tagged enum u128::MAX round-trip failed"
    );

    // Test 3: Nested struct inside tagged enum (mimics TokenMetadata inside PaymentDetails)
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Inner {
        max_supply: u128,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    #[serde(tag = "type")]
    enum Outer {
        Token { metadata: Inner },
    }

    let original = Outer::Token {
        metadata: Inner {
            max_supply: u128::MAX,
        },
    };
    let js = serde_wasm_bindgen::to_value(&original).expect("serialize nested tagged enum");
    let restored: Outer =
        serde_wasm_bindgen::from_value(js).expect("deserialize nested tagged enum");
    assert_eq!(
        restored,
        Outer::Token {
            metadata: Inner {
                max_supply: u128::MAX
            }
        },
        "Nested tagged enum u128::MAX round-trip failed"
    );
}
