use std::slice;

use super::*;
use macros::async_test_all;

use crate::token::tests as shared_tests;

#[cfg(feature = "browser-tests")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn create_token_outputs(identifier_no: u8, output_amounts: Vec<u128>) -> TokenOutputs {
    shared_tests::create_token_outputs(identifier_no, output_amounts)
}

fn future_refresh_start() -> web_time::SystemTime {
    shared_tests::future_refresh_start()
}

// ==================== InMemory-specific tests ====================

#[async_test_all]
async fn test_default() {
    let state: InMemoryTokenOutputStore = InMemoryTokenOutputStore::default();
    assert!(
        state
            .token_outputs
            .lock()
            .await
            .available_token_outputs
            .is_empty()
    );
    assert!(state.token_outputs.lock().await.reservations.is_empty());
}

#[async_test_all]
async fn test_reserve_token_outputs_and_set_add_output() {
    let store = InMemoryTokenOutputStore::default();

    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
        .await;
    assert!(result.is_ok());

    let reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MinTotalValue(300),
            ReservationPurpose::Payment,
            None,
            None,
        )
        .await
        .unwrap();

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 2);
    assert_eq!(stored_token1.reserved_for_payment.len(), 1);

    let token1_updated = create_token_outputs(1, vec![100, 200, 300, 400]);
    let result = store
        .set_tokens_outputs(slice::from_ref(&token1_updated), future_refresh_start())
        .await;
    assert!(result.is_ok());

    // InMemory-specific: check internal state directly
    let token_outputs_state = store.token_outputs.lock().await;
    let reserved_token_outputs = token_outputs_state
        .reservations
        .get(&reservation.id)
        .unwrap();
    assert_eq!(reserved_token_outputs.token_outputs.outputs.len(), 1);
    drop(token_outputs_state);

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 3);
    assert_eq!(stored_token1.reserved_for_payment.len(), 1);
}

#[async_test_all]
async fn test_reserve_token_outputs_and_set_remove_reserved_output() {
    let store = InMemoryTokenOutputStore::default();

    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
        .await;
    assert!(result.is_ok());

    let reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MinTotalValue(300),
            ReservationPurpose::Payment,
            None,
            None,
        )
        .await
        .unwrap();

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 2);
    assert_eq!(stored_token1.reserved_for_payment.len(), 1);

    let token1_updated = create_token_outputs(1, vec![100, 200, 400]);
    let result = store
        .set_tokens_outputs(slice::from_ref(&token1_updated), future_refresh_start())
        .await;
    assert!(result.is_ok());

    // InMemory-specific: check internal state
    let token_outputs_state = store.token_outputs.lock().await;
    let reserved_token_outputs = token_outputs_state.reservations.get(&reservation.id);
    assert!(reserved_token_outputs.is_none());
    drop(token_outputs_state);

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 3);
    assert_eq!(stored_token1.reserved_for_payment.len(), 0);

    let reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MinTotalValue(300),
            ReservationPurpose::Payment,
            None,
            None,
        )
        .await
        .unwrap();

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 1);
    assert_eq!(stored_token1.reserved_for_payment.len(), 2);

    let token1_updated = create_token_outputs(1, vec![100, 400]);
    let result = store
        .set_tokens_outputs(slice::from_ref(&token1_updated), future_refresh_start())
        .await;
    assert!(result.is_ok());

    // InMemory-specific: check internal state
    let token_outputs_state = store.token_outputs.lock().await;
    let reserved_token_outputs = token_outputs_state
        .reservations
        .get(&reservation.id)
        .unwrap();
    assert_eq!(reserved_token_outputs.token_outputs.outputs.len(), 1);
    drop(token_outputs_state);

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 1);
    assert_eq!(stored_token1.reserved_for_payment.len(), 1);
}

#[async_test_all]
async fn test_set_reconciles_reservation_with_empty_outputs() {
    let store = InMemoryTokenOutputStore::default();

    let token1 = create_token_outputs(1, vec![100, 200, 300]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

    let _reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MinTotalValue(300),
            ReservationPurpose::Payment,
            None,
            None,
        )
        .await
        .unwrap();

    let result = store
        .set_tokens_outputs(&[], future_refresh_start())
        .await;
    assert!(result.is_ok());

    // InMemory-specific: check internal state
    let token_outputs_state = store.token_outputs.lock().await;
    assert!(token_outputs_state.reservations.is_empty());
}

// ==================== Shared tests ====================

#[async_test_all]
async fn test_set_tokens_outputs() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_set_tokens_outputs(&store).await;
}

#[async_test_all]
async fn test_get_token_outputs() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_get_token_outputs(&store).await;
}

#[async_test_all]
async fn test_set_tokens_outputs_with_update() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_set_tokens_outputs_with_update(&store).await;
}

#[async_test_all]
async fn test_insert_token_outputs() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_insert_token_outputs(&store).await;
}

#[async_test_all]
async fn test_reserve_token_outputs() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_token_outputs(&store).await;
}

#[async_test_all]
async fn test_reserve_token_outputs_and_cancel() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_token_outputs_and_cancel(&store).await;
}

#[async_test_all]
async fn test_reserve_token_outputs_and_finalize() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_token_outputs_and_finalize(&store).await;
}

#[async_test_all]
async fn test_reserve_token_outputs_and_set_add_output_shared() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_token_outputs_and_set_add_output(&store).await;
}

#[async_test_all]
async fn test_reserve_token_outputs_and_set_remove_reserved_output_shared() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_token_outputs_and_set_remove_reserved_output(&store).await;
}

#[async_test_all]
async fn test_multiple_parallel_reservations() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_multiple_parallel_reservations(&store).await;
}

#[async_test_all]
async fn test_reserve_with_preferred_outputs() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_with_preferred_outputs(&store).await;
}

#[async_test_all]
async fn test_reserve_insufficient_outputs() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_insufficient_outputs(&store).await;
}

#[async_test_all]
async fn test_reserve_nonexistent_token() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_nonexistent_token(&store).await;
}

#[async_test_all]
async fn test_reserve_exact_amount_match() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_exact_amount_match(&store).await;
}

#[async_test_all]
async fn test_reserve_multiple_outputs_combination() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_multiple_outputs_combination(&store).await;
}

#[async_test_all]
async fn test_reserve_all_available_outputs() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_all_available_outputs(&store).await;
}

#[async_test_all]
async fn test_reserve_with_preferred_outputs_insufficient() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_with_preferred_outputs_insufficient(&store).await;
}

#[async_test_all]
async fn test_reserve_zero_amount() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_zero_amount(&store).await;
}

#[async_test_all]
async fn test_cancel_nonexistent_reservation() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_cancel_nonexistent_reservation(&store).await;
}

#[async_test_all]
async fn test_finalize_nonexistent_reservation() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_finalize_nonexistent_reservation(&store).await;
}

#[async_test_all]
async fn test_set_removes_all_tokens() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_set_removes_all_tokens(&store).await;
}

#[async_test_all]
async fn test_reserve_single_large_output() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_single_large_output(&store).await;
}

#[async_test_all]
async fn test_get_token_outputs_none_found() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_get_token_outputs_none_found(&store).await;
}

#[async_test_all]
async fn test_reserve_token_outputs_selection_strategy_smallest_first() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_token_outputs_selection_strategy_smallest_first(&store).await;
}

#[async_test_all]
async fn test_reserve_token_outputs_selection_strategy_largest_first() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_token_outputs_selection_strategy_largest_first(&store).await;
}

#[async_test_all]
async fn test_reserve_max_output_count_smallest_first() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_max_output_count_smallest_first(&store).await;
}

#[async_test_all]
async fn test_reserve_max_output_count_largest_first() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_max_output_count_largest_first(&store).await;
}

#[async_test_all]
async fn test_reserve_max_output_count_more_than_available() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_max_output_count_more_than_available(&store).await;
}

#[async_test_all]
async fn test_reserve_max_output_count_zero_rejected() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_max_output_count_zero_rejected(&store).await;
}

#[async_test_all]
async fn test_reserve_for_payment_affects_balance() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_for_payment_affects_balance(&store).await;
}

#[async_test_all]
async fn test_reserve_for_swap_does_not_affect_balance() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_reserve_for_swap_does_not_affect_balance(&store).await;
}

#[async_test_all]
async fn test_mixed_reservation_purposes_balance() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_mixed_reservation_purposes_balance(&store).await;
}

#[async_test_all]
async fn test_set_tokens_outputs_skipped_during_active_swap() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_set_tokens_outputs_skipped_during_active_swap(&store).await;
}

#[async_test_all]
async fn test_set_tokens_outputs_skipped_after_swap_completes_during_refresh() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_set_tokens_outputs_skipped_after_swap_completes_during_refresh(&store).await;
}

#[async_test_all]
async fn test_insert_outputs_preserved_by_set_tokens_outputs() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_insert_outputs_preserved_by_set_tokens_outputs(&store).await;
}

#[async_test_all]
async fn test_spent_outputs_not_restored_by_set_tokens_outputs() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_spent_outputs_not_restored_by_set_tokens_outputs(&store).await;
}

#[async_test_all]
async fn test_finalize_swap_marks_spent_and_tracks_completion() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_finalize_swap_marks_spent_and_tracks_completion(&store).await;
}

#[async_test_all]
async fn test_insert_outputs_clears_spent_status() {
    let store = InMemoryTokenOutputStore::default();
    shared_tests::test_insert_outputs_clears_spent_status(&store).await;
}
