use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use crate::sdk_builder::PostgresStorageConfig;
use crate::tree_store::{TreeStoreJs, WasmTreeStore};

// Import the postgres-tree-store package
#[wasm_bindgen(module = "js/postgres-tree-store")]
extern "C" {
    #[wasm_bindgen(js_name = "createPostgresTreeStore", catch)]
    async fn create_postgres_tree_store(
        config: PostgresStorageConfig,
        identity: &[u8],
        logger: Option<&crate::logger::Logger>,
    ) -> Result<TreeStoreJs, JsValue>;
}

// Import test helpers
#[wasm_bindgen(module = "js/postgres-test-helpers.cjs")]
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

/// Helper to create a WasmTreeStore instance for testing
async fn create_test_tree_store(test_name: &str) -> WasmTreeStore {
    let conn_string_js = create_test_connection_string(test_name)
        .await
        .expect("Failed to create test connection string");
    let conn_string = conn_string_js
        .as_string()
        .expect("Connection string should be a string");

    let config = crate::sdk_builder::default_postgres_storage_config(&conn_string);

    let tree_store_js = create_postgres_tree_store(config, &TEST_IDENTITY, None)
        .await
        .expect("Failed to create postgres tree store instance");
    WasmTreeStore::new(tree_store_js)
}

#[wasm_bindgen_test]
async fn test_new() {
    let store = create_test_tree_store("pg_tree_new").await;
    breez_sdk_spark::tree_store_tests::test_new(&store).await;
}

#[wasm_bindgen_test]
async fn test_add_leaves() {
    let store = create_test_tree_store("pg_tree_add_leaves").await;
    breez_sdk_spark::tree_store_tests::test_add_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_add_leaves_duplicate_ids() {
    let store = create_test_tree_store("pg_tree_add_leaves_dup").await;
    breez_sdk_spark::tree_store_tests::test_add_leaves_duplicate_ids(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves() {
    let store = create_test_tree_store("pg_tree_set_leaves").await;
    breez_sdk_spark::tree_store_tests::test_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_with_reservations() {
    let store = create_test_tree_store("pg_tree_set_leaves_res").await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_with_reservations(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_preserves_reservations_for_in_flight_swaps() {
    let store = create_test_tree_store("pg_tree_set_leaves_swaps").await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_preserves_reservations_for_in_flight_swaps(
        &store,
    )
    .await;
}

#[wasm_bindgen_test]
async fn test_reserve_leaves() {
    let store = create_test_tree_store("pg_tree_reserve").await;
    breez_sdk_spark::tree_store_tests::test_reserve_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_cancel_reservation() {
    let store = create_test_tree_store("pg_tree_cancel_res").await;
    breez_sdk_spark::tree_store_tests::test_cancel_reservation(&store).await;
}

#[wasm_bindgen_test]
async fn test_cancel_reservation_drops_unkept_leaves() {
    let store = create_test_tree_store("pg_tree_cancel_drop_some").await;
    breez_sdk_spark::tree_store_tests::test_cancel_reservation_drops_unkept_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_cancel_reservation_drops_all_when_keep_empty() {
    let store = create_test_tree_store("pg_tree_cancel_drop_all").await;
    breez_sdk_spark::tree_store_tests::test_cancel_reservation_drops_all_when_keep_empty(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_cancel_reservation_nonexistent() {
    let store = create_test_tree_store("pg_tree_cancel_nonexist").await;
    breez_sdk_spark::tree_store_tests::test_cancel_reservation_nonexistent(&store).await;
}

#[wasm_bindgen_test]
async fn test_finalize_reservation() {
    let store = create_test_tree_store("pg_tree_finalize_res").await;
    breez_sdk_spark::tree_store_tests::test_finalize_reservation(&store).await;
}

#[wasm_bindgen_test]
async fn test_finalize_reservation_nonexistent() {
    let store = create_test_tree_store("pg_tree_finalize_nonexist").await;
    breez_sdk_spark::tree_store_tests::test_finalize_reservation_nonexistent(&store).await;
}

#[wasm_bindgen_test]
async fn test_multiple_reservations() {
    let store = create_test_tree_store("pg_tree_multi_res").await;
    breez_sdk_spark::tree_store_tests::test_multiple_reservations(&store).await;
}

#[wasm_bindgen_test]
async fn test_reservation_ids_are_unique() {
    let store = create_test_tree_store("pg_tree_res_unique").await;
    breez_sdk_spark::tree_store_tests::test_reservation_ids_are_unique(&store).await;
}

#[wasm_bindgen_test]
async fn test_non_reservable_leaves() {
    let store = create_test_tree_store("pg_tree_non_reservable").await;
    breez_sdk_spark::tree_store_tests::test_non_reservable_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_leaves_empty() {
    let store = create_test_tree_store("pg_tree_reserve_empty").await;
    breez_sdk_spark::tree_store_tests::test_reserve_leaves_empty(&store).await;
}

#[wasm_bindgen_test]
async fn test_swap_reservation_included_in_balance() {
    let store = create_test_tree_store("pg_tree_swap_balance").await;
    breez_sdk_spark::tree_store_tests::test_swap_reservation_included_in_balance(&store).await;
}

#[wasm_bindgen_test]
async fn test_payment_reservation_excluded_from_balance() {
    let store = create_test_tree_store("pg_tree_pay_balance").await;
    breez_sdk_spark::tree_store_tests::test_payment_reservation_excluded_from_balance(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_success() {
    let store = create_test_tree_store("pg_tree_try_reserve_ok").await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_success(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_insufficient_funds() {
    let store = create_test_tree_store("pg_tree_try_reserve_insuff").await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_insufficient_funds(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_wait_for_pending() {
    let store = create_test_tree_store("pg_tree_try_reserve_wait").await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_wait_for_pending(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_fail_immediately_when_insufficient() {
    let store = create_test_tree_store("pg_tree_try_reserve_fail").await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_fail_immediately_when_insufficient(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_min_amount_with_leaves_above_individual_target() {
    let store = create_test_tree_store("pg_tree_min_above_target").await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_min_amount_with_leaves_above_individual_target(
        &store,
    )
    .await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_min_amount_exact_denominations_above_individual() {
    let store = create_test_tree_store("pg_tree_min_exact_denoms").await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_min_amount_exact_denominations_above_individual(
        &store,
    )
    .await;
}

#[wasm_bindgen_test]
async fn test_balance_change_notification() {
    let store = create_test_tree_store("pg_tree_balance_notif").await;
    breez_sdk_spark::tree_store_tests::test_balance_change_notification(&store).await;
}

#[wasm_bindgen_test]
async fn test_pending_cleared_on_cancel() {
    let store = create_test_tree_store("pg_tree_pending_cancel").await;
    breez_sdk_spark::tree_store_tests::test_pending_cleared_on_cancel(&store).await;
}

#[wasm_bindgen_test]
async fn test_pending_cleared_on_finalize() {
    let store = create_test_tree_store("pg_tree_pending_finalize").await;
    breez_sdk_spark::tree_store_tests::test_pending_cleared_on_finalize(&store).await;
}

#[wasm_bindgen_test]
async fn test_notification_after_swap_with_exact_amount() {
    let store = create_test_tree_store("pg_tree_notif_swap").await;
    breez_sdk_spark::tree_store_tests::test_notification_after_swap_with_exact_amount(&store).await;
}

#[wasm_bindgen_test]
async fn test_notification_on_pending_balance_change() {
    let store = create_test_tree_store("pg_tree_notif_pending").await;
    breez_sdk_spark::tree_store_tests::test_notification_on_pending_balance_change(&store).await;
}

#[wasm_bindgen_test]
async fn test_spent_leaves_not_restored_by_set_leaves() {
    let store = create_test_tree_store("pg_tree_spent_not_restored").await;
    breez_sdk_spark::tree_store_tests::test_spent_leaves_not_restored_by_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_spent_ids_cleaned_up_when_no_longer_in_refresh() {
    let store = create_test_tree_store("pg_tree_spent_cleanup").await;
    breez_sdk_spark::tree_store_tests::test_spent_ids_cleaned_up_when_no_longer_in_refresh(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_add_leaves_not_deleted_by_set_leaves() {
    let store = create_test_tree_store("pg_tree_add_not_deleted").await;
    breez_sdk_spark::tree_store_tests::test_add_leaves_not_deleted_by_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_old_leaves_deleted_by_set_leaves() {
    let store = create_test_tree_store("pg_tree_old_deleted").await;
    breez_sdk_spark::tree_store_tests::test_old_leaves_deleted_by_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_change_leaves_from_swap_protected() {
    let store = create_test_tree_store("pg_tree_change_protected").await;
    breez_sdk_spark::tree_store_tests::test_change_leaves_from_swap_protected(&store).await;
}

#[wasm_bindgen_test]
async fn test_finalize_with_new_leaves_protected() {
    let store = create_test_tree_store("pg_tree_fin_new_protected").await;
    breez_sdk_spark::tree_store_tests::test_finalize_with_new_leaves_protected(&store).await;
}

#[wasm_bindgen_test]
async fn test_add_leaves_clears_spent_status() {
    let store = create_test_tree_store("pg_tree_add_clears_spent").await;
    breez_sdk_spark::tree_store_tests::test_add_leaves_clears_spent_status(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_skipped_during_active_swap() {
    let store = create_test_tree_store("pg_tree_skip_active_swap").await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_skipped_during_active_swap(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_skipped_after_swap_completes_during_refresh() {
    let store = create_test_tree_store("pg_tree_skip_swap_refresh").await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_skipped_after_swap_completes_during_refresh(
        &store,
    )
    .await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_proceeds_after_swap_when_refresh_starts_later() {
    let store = create_test_tree_store("pg_tree_proceeds_later").await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_proceeds_after_swap_when_refresh_starts_later(
        &store,
    )
    .await;
}

#[wasm_bindgen_test]
async fn test_payment_reservation_does_not_block_set_leaves() {
    let store = create_test_tree_store("pg_tree_pay_no_block").await;
    breez_sdk_spark::tree_store_tests::test_payment_reservation_does_not_block_set_leaves(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_update_reservation_basic() {
    let store = create_test_tree_store("pg_tree_update_basic").await;
    breez_sdk_spark::tree_store_tests::test_update_reservation_basic(&store).await;
}

#[wasm_bindgen_test]
async fn test_update_reservation_nonexistent() {
    let store = create_test_tree_store("pg_tree_update_nonexist").await;
    breez_sdk_spark::tree_store_tests::test_update_reservation_nonexistent(&store).await;
}

#[wasm_bindgen_test]
async fn test_update_reservation_clears_pending() {
    let store = create_test_tree_store("pg_tree_update_clears").await;
    breez_sdk_spark::tree_store_tests::test_update_reservation_clears_pending(&store).await;
}

#[wasm_bindgen_test]
async fn test_update_reservation_preserves_purpose() {
    let store = create_test_tree_store("pg_tree_update_purpose").await;
    breez_sdk_spark::tree_store_tests::test_update_reservation_preserves_purpose(&store).await;
}

#[wasm_bindgen_test]
async fn test_get_leaves_not_available() {
    let store = create_test_tree_store("pg_tree_not_available").await;
    breez_sdk_spark::tree_store_tests::test_get_leaves_not_available(&store).await;
}

#[wasm_bindgen_test]
async fn test_get_leaves_missing_operators_filters_spent() {
    let store = create_test_tree_store("pg_tree_missing_ops_spent").await;
    breez_sdk_spark::tree_store_tests::test_get_leaves_missing_operators_filters_spent(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_missing_operators_replaced_on_set_leaves() {
    let store = create_test_tree_store("pg_tree_missing_ops_replace").await;
    breez_sdk_spark::tree_store_tests::test_missing_operators_replaced_on_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_with_none_target_reserves_all() {
    let store = create_test_tree_store("pg_tree_none_target").await;
    breez_sdk_spark::tree_store_tests::test_reserve_with_none_target_reserves_all(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_skips_non_available_leaves() {
    let store = create_test_tree_store("pg_tree_skip_non_avail").await;
    breez_sdk_spark::tree_store_tests::test_reserve_skips_non_available_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_add_leaves_empty_slice() {
    let store = create_test_tree_store("pg_tree_add_empty").await;
    breez_sdk_spark::tree_store_tests::test_add_leaves_empty_slice(&store).await;
}

#[wasm_bindgen_test]
async fn test_full_payment_cycle() {
    let store = create_test_tree_store("pg_tree_full_cycle").await;
    breez_sdk_spark::tree_store_tests::test_full_payment_cycle(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_replaces_fully() {
    let store = create_test_tree_store("pg_tree_replaces_fully").await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_replaces_fully(&store).await;
}
