use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use crate::tree_store::{TreeStoreJs, WasmTreeStore};

// Import the node-tree-store package (better-sqlite3).
#[wasm_bindgen(module = "js/node-tree-store")]
extern "C" {
    #[wasm_bindgen(js_name = "createNodeTreeStore", catch)]
    async fn create_node_tree_store(
        db_path: &str,
        logger: Option<&crate::logger::Logger>,
        run_migration: bool,
    ) -> Result<TreeStoreJs, JsValue>;
}

/// Each test gets a fresh in-memory SQLite database, so no path or cleanup is
/// needed and tests are fully isolated.
async fn create_test_tree_store() -> WasmTreeStore {
    let tree_store_js = create_node_tree_store(":memory:", None, true)
        .await
        .expect("Failed to create node tree store instance");
    WasmTreeStore::new(tree_store_js)
}

#[wasm_bindgen_test]
async fn test_add_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_add_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_add_leaves_clears_spent_status() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_add_leaves_clears_spent_status(&store).await;
}

#[wasm_bindgen_test]
async fn test_add_leaves_duplicate_ids() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_add_leaves_duplicate_ids(&store).await;
}

#[wasm_bindgen_test]
async fn test_add_leaves_empty_slice() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_add_leaves_empty_slice(&store).await;
}

#[wasm_bindgen_test]
async fn test_add_leaves_clears_missing_from_operators() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_add_leaves_clears_missing_from_operators(&store).await;
}

#[wasm_bindgen_test]
async fn test_missing_from_operators_leaves_are_not_selectable() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_missing_from_operators_leaves_are_not_selectable(
        &store,
    )
    .await;
}

#[wasm_bindgen_test]
async fn test_missing_from_operators_leaf_not_available() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_missing_from_operators_leaf_not_available(&store).await;
}

#[wasm_bindgen_test]
async fn test_add_leaves_not_deleted_by_set_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_add_leaves_not_deleted_by_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_ancestor_not_returned_as_leaf() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_ancestor_not_returned_as_leaf(&store).await;
}

#[wasm_bindgen_test]
async fn test_balance_change_notification() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_balance_change_notification(&store).await;
}

#[wasm_bindgen_test]
async fn test_cancel_reservation() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_cancel_reservation(&store).await;
}

#[wasm_bindgen_test]
async fn test_cancel_reservation_drops_all_when_keep_empty() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_cancel_reservation_drops_all_when_keep_empty(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_cancel_reservation_drops_unkept_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_cancel_reservation_drops_unkept_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_cancel_reservation_nonexistent() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_cancel_reservation_nonexistent(&store).await;
}

#[wasm_bindgen_test]
async fn test_cancel_reservation_nonexistent_keeps_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_cancel_reservation_nonexistent_keeps_leaves(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_change_leaves_from_swap_protected() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_change_leaves_from_swap_protected(&store).await;
}

#[wasm_bindgen_test]
async fn test_finalize_reservation() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_finalize_reservation(&store).await;
}

#[wasm_bindgen_test]
async fn test_finalize_reservation_nonexistent() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_finalize_reservation_nonexistent(&store).await;
}

#[wasm_bindgen_test]
async fn test_finalize_with_new_leaves_protected() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_finalize_with_new_leaves_protected(&store).await;
}

#[wasm_bindgen_test]
async fn test_full_payment_cycle() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_full_payment_cycle(&store).await;
}

#[wasm_bindgen_test]
async fn test_get_exit_chains() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_get_exit_chains(&store).await;
}

#[wasm_bindgen_test]
async fn test_get_exit_chain_missing_ancestor() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_get_exit_chain_missing_ancestor(&store).await;
}

#[wasm_bindgen_test]
async fn test_incomplete_pedigree_still_spendable() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_incomplete_pedigree_still_spendable(&store).await;
}

#[wasm_bindgen_test]
async fn test_exit_chain_after_swap_update() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_exit_chain_after_swap_update(&store).await;
}

#[wasm_bindgen_test]
async fn test_exit_chain_after_cancel_reparent() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_exit_chain_after_cancel_reparent(&store).await;
}

#[wasm_bindgen_test]
async fn test_get_leaves_missing_operators_filters_spent() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_get_leaves_missing_operators_filters_spent(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_get_leaves_not_available() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_get_leaves_not_available(&store).await;
}

#[wasm_bindgen_test]
async fn test_get_verified_leaf_keys() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_get_verified_leaf_keys(&store).await;
}

#[wasm_bindgen_test]
async fn test_leaf_reparented_by_renewal() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_leaf_reparented_by_renewal(&store).await;
}

#[wasm_bindgen_test]
async fn test_missing_operators_replaced_on_set_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_missing_operators_replaced_on_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_multiple_reservations() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_multiple_reservations(&store).await;
}

#[wasm_bindgen_test]
async fn test_new() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_new(&store).await;
}

#[wasm_bindgen_test]
async fn test_node_update_in_place() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_node_update_in_place(&store).await;
}

#[wasm_bindgen_test]
async fn test_non_reservable_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_non_reservable_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_notification_after_swap_with_exact_amount() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_notification_after_swap_with_exact_amount(&store).await;
}

#[wasm_bindgen_test]
async fn test_notification_on_pending_balance_change() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_notification_on_pending_balance_change(&store).await;
}

#[wasm_bindgen_test]
async fn test_old_leaves_deleted_by_set_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_old_leaves_deleted_by_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_payment_reservation_does_not_block_set_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_payment_reservation_does_not_block_set_leaves(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_payment_reservation_excluded_from_balance() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_payment_reservation_excluded_from_balance(&store).await;
}

#[wasm_bindgen_test]
async fn test_pending_cleared_on_cancel() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_pending_cleared_on_cancel(&store).await;
}

#[wasm_bindgen_test]
async fn test_pending_cleared_on_finalize() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_pending_cleared_on_finalize(&store).await;
}

#[wasm_bindgen_test]
async fn test_reservation_ids_are_unique() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_reservation_ids_are_unique(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_reserve_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_leaves_by_ids() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_reserve_leaves_by_ids(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_leaves_by_ids_not_available() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_reserve_leaves_by_ids_not_available(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_leaves_by_ids_preserves_order() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_reserve_leaves_by_ids_preserves_order(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_leaves_empty() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_reserve_leaves_empty(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_skips_non_available_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_reserve_skips_non_available_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_reserve_with_none_target_reserves_all() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_reserve_with_none_target_reserves_all(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_preserves_reservations_for_in_flight_swaps() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_preserves_reservations_for_in_flight_swaps(
        &store,
    )
    .await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_proceeds_after_swap_when_refresh_starts_later() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_proceeds_after_swap_when_refresh_starts_later(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_replaces_fully() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_replaces_fully(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_skipped_after_swap_completes_during_refresh() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_skipped_after_swap_completes_during_refresh(
        &store,
    )
    .await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_skipped_during_active_swap() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_skipped_during_active_swap(&store).await;
}

#[wasm_bindgen_test]
async fn test_set_leaves_with_reservations() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_set_leaves_with_reservations(&store).await;
}

#[wasm_bindgen_test]
async fn test_shared_ancestor_survives_leaf_deletion() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_shared_ancestor_survives_leaf_deletion(&store).await;
}

#[wasm_bindgen_test]
async fn test_spent_ids_cleaned_up_when_no_longer_in_refresh() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_spent_ids_cleaned_up_when_no_longer_in_refresh(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_spent_leaves_not_restored_by_set_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_spent_leaves_not_restored_by_set_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_swap_reservation_included_in_balance() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_swap_reservation_included_in_balance(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_fail_immediately_when_insufficient() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_fail_immediately_when_insufficient(&store)
        .await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_insufficient_funds() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_insufficient_funds(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_min_amount_exact_denominations_above_individual() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_min_amount_exact_denominations_above_individual(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_min_amount_with_leaves_above_individual_target() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_min_amount_with_leaves_above_individual_target(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_success() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_success(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_reserve_wait_for_pending() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_try_reserve_wait_for_pending(&store).await;
}

#[wasm_bindgen_test]
async fn test_try_select_leaves() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_try_select_leaves(&store).await;
}

#[wasm_bindgen_test]
async fn test_unshared_ancestor_deleted_with_leaf() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_unshared_ancestor_deleted_with_leaf(&store).await;
}

#[wasm_bindgen_test]
async fn test_update_reservation_basic() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_update_reservation_basic(&store).await;
}

#[wasm_bindgen_test]
async fn test_update_reservation_clears_pending() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_update_reservation_clears_pending(&store).await;
}

#[wasm_bindgen_test]
async fn test_update_reservation_nonexistent() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_update_reservation_nonexistent(&store).await;
}

#[wasm_bindgen_test]
async fn test_update_reservation_preserves_purpose() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_update_reservation_preserves_purpose(&store).await;
}

#[wasm_bindgen_test]
async fn test_upsert_and_get_leaf() {
    let store = create_test_tree_store().await;
    breez_sdk_spark::tree_store_tests::test_upsert_and_get_leaf(&store).await;
}
