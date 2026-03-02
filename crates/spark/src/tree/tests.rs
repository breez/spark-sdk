//! Shared test suite for `TreeStore` implementations.
//!
//! Each function tests a specific behavior against any `TreeStore` impl.
//! To use, call these functions from implementation-specific test modules
//! passing a concrete store instance.

use std::str::FromStr;
use std::time::Duration;

use bitcoin::{Transaction, absolute::LockTime, secp256k1::PublicKey, transaction::Version};
use frost_secp256k1_tr::Identifier;
use web_time::SystemTime;

use crate::tree::{
    Leaves, LeavesReservation, ReservationPurpose, ReserveResult, TargetAmounts, TreeNode,
    TreeNodeId, TreeNodeStatus, TreeServiceError, TreeStore,
};

/// Creates a test `TreeNode` with the given ID and value.
pub fn create_test_tree_node(id: &str, value: u64) -> TreeNode {
    TreeNode {
        id: TreeNodeId::from_str(id).unwrap(),
        tree_id: "test_tree".to_string(),
        value,
        parent_node_id: None,
        node_tx: Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        },
        refund_tx: None,
        direct_tx: None,
        direct_refund_tx: None,
        direct_from_cpfp_refund_tx: None,
        vout: 0,
        verifying_public_key: PublicKey::from_str(
            "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
        )
        .unwrap(),
        owner_identity_public_key: PublicKey::from_str(
            "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
        )
        .unwrap(),
        signing_keyshare: crate::tree::SigningKeyshare {
            public_key: PublicKey::from_str(
                "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
            )
            .unwrap(),
            owner_identifiers: vec![Identifier::try_from(1u16).unwrap()],
            threshold: 2,
        },
        status: crate::tree::TreeNodeStatus::Available,
    }
}

/// Helper function to reserve leaves in tests.
/// Wraps `try_reserve_leaves` and expects success.
pub async fn reserve_leaves(
    store: &dyn TreeStore,
    target_amounts: Option<&TargetAmounts>,
    exact_only: bool,
    purpose: ReservationPurpose,
) -> Result<LeavesReservation, TreeServiceError> {
    match store
        .try_reserve_leaves(target_amounts, exact_only, purpose)
        .await?
    {
        ReserveResult::Success(reservation) => Ok(reservation),
        ReserveResult::InsufficientFunds => Err(TreeServiceError::InsufficientFunds),
        ReserveResult::WaitForPending { .. } => Err(TreeServiceError::Generic(
            "Unexpected WaitForPending".into(),
        )),
    }
}

/// Returns a future `SystemTime`, ensuring that leaves added "now" are
/// treated as old relative to this refresh start.
fn future_refresh_start() -> SystemTime {
    SystemTime::now() + Duration::from_secs(10)
}

/// Asserts that `get_leaves()` returns `expected` available leaves.
async fn assert_available_count(store: &dyn TreeStore, expected: usize) {
    let leaves = store.get_leaves().await.unwrap();
    assert_eq!(leaves.available.len(), expected);
}

/// Returns the available leaves from the store.
async fn get_available(store: &dyn TreeStore) -> Vec<TreeNode> {
    store.get_leaves().await.unwrap().available
}

/// Returns the full `Leaves` snapshot from the store.
async fn get_all(store: &dyn TreeStore) -> Leaves {
    store.get_leaves().await.unwrap()
}

// ==================== Test functions ====================

pub async fn test_new(store: &dyn TreeStore) {
    assert!(store.get_leaves().await.unwrap().available.is_empty());
}

pub async fn test_add_leaves(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];

    store.add_leaves(&leaves).await.unwrap();

    let stored = get_available(store).await;
    assert_eq!(stored.len(), 2);
    assert!(
        stored
            .iter()
            .any(|l| l.id.to_string() == "node1" && l.value == 100)
    );
    assert!(
        stored
            .iter()
            .any(|l| l.id.to_string() == "node2" && l.value == 200)
    );
}

pub async fn test_add_leaves_duplicate_ids(store: &dyn TreeStore) {
    let leaf1 = create_test_tree_node("node1", 100);
    let leaf2 = create_test_tree_node("node1", 200); // Same ID, different value

    store.add_leaves(&[leaf1]).await.unwrap();
    store.add_leaves(&[leaf2]).await.unwrap();

    let stored = get_available(store).await;
    assert_eq!(stored.len(), 1);
    // Should have the second value (200) as it overwrites the first
    assert_eq!(stored[0].value, 200);
}

pub async fn test_set_leaves(store: &dyn TreeStore) {
    let initial = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&initial).await.unwrap();

    let refresh_start = future_refresh_start();
    let new_leaves = vec![
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store
        .set_leaves(&new_leaves, &[], refresh_start)
        .await
        .unwrap();

    let stored = get_available(store).await;
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().any(|l| l.id.to_string() == "node2"));
    assert!(stored.iter().any(|l| l.id.to_string() == "node3"));
    assert!(!stored.iter().any(|l| l.id.to_string() == "node1"));
}

pub async fn test_set_leaves_with_reservations(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve some leaves
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(600, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    let refresh_start = future_refresh_start();

    // Update leaves with new data (including updated versions of reserved leaves)
    let non_existing_operator_leaf = create_test_tree_node("node7", 1000);
    let mut updated_leaf1 = create_test_tree_node("node1", 150);
    updated_leaf1.status = crate::tree::TreeNodeStatus::TransferLocked;
    let new_leaves = vec![
        updated_leaf1,
        create_test_tree_node("node2", 250),
        create_test_tree_node("node4", 400),
    ];
    store
        .set_leaves(&new_leaves, &[non_existing_operator_leaf], refresh_start)
        .await
        .unwrap();

    // Check main pool via get_leaves
    let all = get_all(store).await;
    assert_eq!(all.payment_reserved_balance(), 700); // 150+250+300
    assert_eq!(all.available_balance(), 400);
    assert_eq!(all.missing_operators_balance(), 1000);
    assert_eq!(all.balance(), 400 + 1000);
    assert_eq!(all.available.len(), 1);
    assert!(all.available.iter().any(|l| l.id.to_string() == "node4"));
}

pub async fn test_set_leaves_preserves_reservations_for_in_flight_swaps(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves (simulating start of a swap)
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    let refresh_start = future_refresh_start();

    // Set new leaves that don't include the reserved ones
    let new_leaves = vec![create_test_tree_node("node3", 300)];
    store
        .set_leaves(&new_leaves, &[], refresh_start)
        .await
        .unwrap();

    // Reservation should be PRESERVED - verify through get_leaves
    let all = get_all(store).await;
    assert_eq!(all.reserved_for_payment.len(), 2);
    assert!(
        all.reserved_for_payment
            .iter()
            .any(|l| l.id.to_string() == "node1")
    );
    assert!(
        all.reserved_for_payment
            .iter()
            .any(|l| l.id.to_string() == "node2")
    );
}

pub async fn test_reserve_leaves(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Check through get_leaves that reservation was created
    let all = get_all(store).await;
    assert_eq!(all.reserved_for_payment.len(), 1);
    assert_eq!(all.reserved_for_payment[0].id, leaves[0].id);
    // Check that leaf was removed from main pool
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available[0].id, leaves[1].id);
    assert!(!reservation.id.is_empty());
}

pub async fn test_cancel_reservation(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Cancel the reservation
    store.cancel_reservation(&reservation.id).await.unwrap();

    // Check that leaf was returned to main pool
    let all = get_all(store).await;
    assert!(all.reserved_for_payment.is_empty());
    assert_eq!(all.available.len(), 2);
    assert!(all.available.iter().any(|l| l.id == leaves[0].id));
    assert!(all.available.iter().any(|l| l.id == leaves[1].id));
}

pub async fn test_cancel_reservation_nonexistent(store: &dyn TreeStore) {
    let fake_id = "fake-reservation-id".to_string();

    // Should not panic or cause issues
    store.cancel_reservation(&fake_id).await.unwrap();
    assert_available_count(store, 0).await;
}

pub async fn test_finalize_reservation(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Finalize the reservation
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // Check that reservation was removed and leaf was NOT returned to pool
    let all = get_all(store).await;
    assert!(all.reserved_for_payment.is_empty());
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available[0].id, leaves[1].id);
}

pub async fn test_finalize_reservation_nonexistent(store: &dyn TreeStore) {
    let fake_id = "fake-reservation-id".to_string();

    // Should not panic or cause issues
    store.finalize_reservation(&fake_id, None).await.unwrap();
    assert_available_count(store, 0).await;
}

pub async fn test_multiple_reservations(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Create multiple reservations
    let reservation1 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    let reservation2 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(200, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Check both reservations exist
    let all = get_all(store).await;
    assert_eq!(all.reserved_for_payment.len(), 2);
    // Check main pool has only one leaf left
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available[0].id, leaves[2].id);

    // Cancel one reservation
    store.cancel_reservation(&reservation1.id).await.unwrap();
    assert_eq!(get_all(store).await.available.len(), 2);

    // Finalize the other
    store
        .finalize_reservation(&reservation2.id, None)
        .await
        .unwrap();
    assert_eq!(get_all(store).await.available.len(), 2);
}

pub async fn test_reservation_ids_are_unique(store: &dyn TreeStore) {
    let leaf = create_test_tree_node("node1", 100);
    store.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();

    let r1 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    store.cancel_reservation(&r1.id).await.unwrap();
    let r2 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    assert_ne!(r1.id, r2.id);
}

pub async fn test_non_reservable_leaves(store: &dyn TreeStore) {
    let leaf = create_test_tree_node("node1", 100);
    store.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();

    reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    let result = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap_err();
    assert!(matches!(result, TreeServiceError::InsufficientFunds));
}

pub async fn test_reserve_leaves_empty(store: &dyn TreeStore) {
    let err = reserve_leaves(store, None, false, ReservationPurpose::Payment)
        .await
        .unwrap_err();

    assert!(matches!(err, TreeServiceError::NonReservableLeaves));
}

pub async fn test_swap_reservation_included_in_balance(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve some leaves for swap
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        true,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Check that swap-reserved leaves are included in balance
    let all = get_all(store).await;
    assert_eq!(all.swap_reserved_balance(), 300);
    assert_eq!(all.available_balance(), 300);
    assert_eq!(all.balance(), 300 + 300);
}

pub async fn test_payment_reservation_excluded_from_balance(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve some leaves for payment
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Check that payment-reserved leaves are excluded from balance
    let all = get_all(store).await;
    assert_eq!(all.payment_reserved_balance(), 300);
    assert_eq!(all.available_balance(), 300);
    assert_eq!(all.balance(), 300);
}

pub async fn test_try_reserve_success(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let result = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    assert!(matches!(result, ReserveResult::Success(_)));
    if let ReserveResult::Success(reservation) = result {
        assert_eq!(reservation.sum(), 100);
    }
}

pub async fn test_try_reserve_insufficient_funds(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    let result = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(500, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    assert!(matches!(result, ReserveResult::InsufficientFunds));
}

pub async fn test_try_reserve_wait_for_pending(store: &dyn TreeStore) {
    // Add a single 1000 sat leaf
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve with target 100 - store will reserve 1000 and auto-track pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(matches!(r1, ReserveResult::Success(_)));

    // Try to reserve 300 more - should get WaitForPending since pending=900 > 300
    let r2 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    match r2 {
        ReserveResult::WaitForPending {
            needed,
            available,
            pending,
        } => {
            assert_eq!(needed, 300);
            assert_eq!(available, 0);
            assert_eq!(pending, 900);
        }
        _ => panic!("Expected WaitForPending, got {:?}", r2),
    }
}

pub async fn test_try_reserve_fail_immediately_when_insufficient(store: &dyn TreeStore) {
    // Add 100 sat leaf
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve it for 50 sats - pending will be 50
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(50, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(matches!(r1, ReserveResult::Success(_)));

    // Request 500 - more than available + pending (0 + 50 < 500)
    let result = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(500, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(matches!(result, ReserveResult::InsufficientFunds));
}

pub async fn test_balance_change_notification(store: &dyn TreeStore) {
    let mut rx = store.subscribe_balance_changes();

    // Add leaves
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    // Wait for notification with timeout
    let result =
        tokio_with_wasm::alias::time::timeout(std::time::Duration::from_millis(100), async {
            rx.changed().await.ok();
        })
        .await;

    assert!(result.is_ok());
}

pub async fn test_pending_cleared_on_cancel(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve with target 100 - auto-tracks pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    let reservation_id = match r1 {
        ReserveResult::Success(r) => r.id,
        _ => panic!("Expected Success"),
    };

    // Cancel the reservation - pending should be cleared
    store.cancel_reservation(&reservation_id).await.unwrap();

    // Try to reserve 300 - should succeed since 1000 sat leaf is back
    let r2 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    assert!(matches!(r2, ReserveResult::Success(_)));
}

pub async fn test_pending_cleared_on_finalize(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve with target 100 - auto-tracks pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    let reservation_id = match r1 {
        ReserveResult::Success(r) => r.id,
        _ => panic!("Expected Success"),
    };

    // Finalize with new leaves (the change from swap)
    let change_leaf = create_test_tree_node("node2", 900);
    store
        .finalize_reservation(&reservation_id, Some(&[change_leaf]))
        .await
        .unwrap();

    // Try to reserve 300 - should succeed since change is now available
    let r2 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    assert!(matches!(r2, ReserveResult::Success(_)));
}

pub async fn test_notification_after_swap_with_exact_amount(store: &dyn TreeStore) {
    let mut rx = store.subscribe_balance_changes();

    // Add a single 1000 sat leaf
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Consume the initial notification
    let _ =
        tokio_with_wasm::alias::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    // Reserve it with target 100 - will reserve all 1000, pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    let reservation_id = match r1 {
        ReserveResult::Success(r) => r.id,
        _ => panic!("Expected Success"),
    };

    // Consume the reservation notification
    let _ =
        tokio_with_wasm::alias::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    // Simulate a swap that returns exactly the target amount (100 sats)
    let swap_result_leaf = create_test_tree_node("node2", 100);
    store
        .update_reservation(&reservation_id, &[swap_result_leaf], &[])
        .await
        .unwrap();

    // Verify that we still get a notification even though net balance is 0 -> 0
    let notification_result =
        tokio_with_wasm::alias::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    assert!(
        notification_result.is_ok(),
        "Expected notification after swap update with exact amount"
    );
}

pub async fn test_notification_on_pending_balance_change(store: &dyn TreeStore) {
    let mut rx = store.subscribe_balance_changes();

    // Add a single 1000 sat leaf
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Consume initial notification
    let _ =
        tokio_with_wasm::alias::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    // Reserve with target 100 - pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    // Consume reservation notification
    let _ =
        tokio_with_wasm::alias::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    let reservation_id = match r1 {
        ReserveResult::Success(r) => r.id,
        _ => panic!("Expected Success"),
    };

    // Cancel the reservation - pending changes from 900 to 0
    store.cancel_reservation(&reservation_id).await.unwrap();

    // Should get notification because pending balance changed
    let notification_result =
        tokio_with_wasm::alias::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    assert!(
        notification_result.is_ok(),
        "Expected notification when pending balance changes"
    );
}

pub async fn test_spent_leaves_not_restored_by_set_leaves(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve node1 for payment
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Finalize the reservation (node1 is now spent)
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // Verify node1 is not in the pool
    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "node2"));
    assert!(!available.iter().any(|l| l.id.to_string() == "node1"));

    // Simulate a refresh that started BEFORE the finalize completed.
    let refresh_start = SystemTime::now() - Duration::from_secs(60);
    let stale_leaves = vec![
        create_test_tree_node("node1", 100), // This was spent!
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store
        .set_leaves(&stale_leaves, &[], refresh_start)
        .await
        .unwrap();

    // Verify node1 was NOT restored
    let available = get_available(store).await;
    assert_eq!(available.len(), 2); // node2 and node3 only
    assert!(available.iter().any(|l| l.id.to_string() == "node2"));
    assert!(available.iter().any(|l| l.id.to_string() == "node3"));
    assert!(
        !available.iter().any(|l| l.id.to_string() == "node1"),
        "Spent leaf node1 should not be restored by set_leaves when refresh started before spend"
    );
}

pub async fn test_spent_ids_cleaned_up_when_no_longer_in_refresh(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve and finalize node1
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // First refresh with refresh_start BEFORE spent_at
    let refresh_start = SystemTime::now() - Duration::from_secs(60);
    let stale_leaves = vec![create_test_tree_node("node1", 100)];
    store
        .set_leaves(&stale_leaves, &[], refresh_start)
        .await
        .unwrap();
    // node1 should be filtered out because it's a recent spend
    assert!(get_available(store).await.is_empty());

    // Second refresh with refresh_start AFTER spent_at
    let refresh_start2 = future_refresh_start();
    let fresh_leaves = vec![create_test_tree_node("node2", 200)];
    store
        .set_leaves(&fresh_leaves, &[], refresh_start2)
        .await
        .unwrap();

    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "node2"));
}

pub async fn test_add_leaves_not_deleted_by_set_leaves(store: &dyn TreeStore) {
    // Add initial leaves
    let initial = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&initial).await.unwrap();

    // Refresh starts at T1
    let refresh_start = SystemTime::now();

    // Small delay to ensure the new leaf is added AFTER refresh_start
    tokio_with_wasm::alias::time::sleep(Duration::from_millis(10)).await;

    // While refresh is in progress, a new leaf arrives
    let new_leaf = create_test_tree_node("node2", 200);
    store.add_leaves(&[new_leaf]).await.unwrap();

    // Refresh completes with stale data (doesn't include node2)
    let stale_refresh_data = vec![create_test_tree_node("node1", 100)];
    store
        .set_leaves(&stale_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // node2 should be PRESERVED because it was added after refresh started
    let available = get_available(store).await;
    assert_eq!(available.len(), 2);
    assert!(available.iter().any(|l| l.id.to_string() == "node1"));
    assert!(
        available.iter().any(|l| l.id.to_string() == "node2"),
        "Leaf added after refresh started should be preserved"
    );
}

pub async fn test_old_leaves_deleted_by_set_leaves(store: &dyn TreeStore) {
    // Add initial leaves
    let initial = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&initial).await.unwrap();

    // Use a future refresh_start so existing leaves are considered "old"
    let refresh_start = future_refresh_start();

    // Refresh completes without node2
    let refresh_data = vec![create_test_tree_node("node1", 100)];
    store
        .set_leaves(&refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // node2 should be DELETED
    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "node1"));
    assert!(
        !available.iter().any(|l| l.id.to_string() == "node2"),
        "Leaf added before refresh started should be deleted if not in refresh data"
    );
}

pub async fn test_change_leaves_from_swap_protected(store: &dyn TreeStore) {
    // Add initial leaf
    let initial = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&initial).await.unwrap();

    // Reserve the leaf
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Refresh starts
    let refresh_start = SystemTime::now();

    // Small delay
    tokio_with_wasm::alias::time::sleep(Duration::from_millis(10)).await;

    // Swap completes and adds change leaves
    let reserved_leaf = create_test_tree_node("swap_output", 500);
    let change_leaf = create_test_tree_node("change", 500);
    store
        .update_reservation(&reservation.id, &[reserved_leaf], &[change_leaf])
        .await
        .unwrap();

    // Refresh completes with stale data
    let stale_refresh_data = vec![create_test_tree_node("node1", 1000)];
    store
        .set_leaves(&stale_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // change leaf should be PRESERVED
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "change"),
        "Change leaf from swap should be preserved"
    );
}

pub async fn test_finalize_with_new_leaves_protected(store: &dyn TreeStore) {
    // Add initial leaf
    let initial = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&initial).await.unwrap();

    // Reserve the leaf
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Refresh starts
    let refresh_start = SystemTime::now();

    // Small delay
    tokio_with_wasm::alias::time::sleep(Duration::from_millis(10)).await;

    // Payment completes and adds change via finalize_reservation
    let change_leaf = create_test_tree_node("change", 900);
    store
        .finalize_reservation(&reservation.id, Some(&[change_leaf]))
        .await
        .unwrap();

    // Refresh completes with stale data
    let stale_refresh_data = vec![create_test_tree_node("node1", 1000)];
    store
        .set_leaves(&stale_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // change leaf should be PRESERVED, node1 should NOT be restored (spent)
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "change"),
        "Change leaf from finalize should be preserved"
    );
    assert!(
        !available.iter().any(|l| l.id.to_string() == "node1"),
        "Spent leaf should not be restored"
    );
}

pub async fn test_add_leaves_clears_spent_status(store: &dyn TreeStore) {
    // Add initial leaf
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve and finalize node1 (marks it as spent)
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // Verify node1 is not in the pool
    assert!(get_available(store).await.is_empty());

    // Add the leaf back via add_leaves (simulating receiving it back)
    let returning_leaf = create_test_tree_node("node1", 100);
    store.add_leaves(&[returning_leaf]).await.unwrap();

    // Verify node1 IS now in the pool (spent status was cleared)
    let available = get_available(store).await;
    assert_eq!(
        available.len(),
        1,
        "Leaf should be re-added when receiving it back via add_leaves"
    );
    assert!(
        available.iter().any(|l| l.id.to_string() == "node1"),
        "node1 should be in available leaves"
    );

    // New leaf should also be added
    let new_leaf = create_test_tree_node("node2", 200);
    store.add_leaves(&[new_leaf]).await.unwrap();
    assert_eq!(get_available(store).await.len(), 2);
}

pub async fn test_set_leaves_skipped_during_active_swap(store: &dyn TreeStore) {
    // Add initial leaves
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves for a swap (not payment)
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        false,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Simulate refresh starting while swap is in progress
    let refresh_start = SystemTime::now();

    tokio_with_wasm::alias::time::sleep(Duration::from_millis(10)).await;

    // Try to set new leaves (should be skipped due to active swap)
    let new_leaves = vec![create_test_tree_node("node3", 300)];
    store
        .set_leaves(&new_leaves, &[], refresh_start)
        .await
        .unwrap();

    // set_leaves should have been skipped
    let all = get_all(store).await;
    assert!(
        all.available.is_empty(),
        "set_leaves should be skipped during active swap"
    );
    assert_eq!(all.reserved_for_swap.len(), 2);
}

pub async fn test_set_leaves_skipped_after_swap_completes_during_refresh(store: &dyn TreeStore) {
    // Add initial leaves
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves for a swap
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Refresh starts at T0
    let refresh_start = SystemTime::now();

    // Small delay to ensure swap completes AFTER refresh started
    tokio_with_wasm::alias::time::sleep(Duration::from_millis(10)).await;

    // Swap completes at T1
    let new_leaves_from_swap = vec![create_test_tree_node("swap_result", 500)];
    store
        .finalize_reservation(&reservation.id, Some(&new_leaves_from_swap))
        .await
        .unwrap();

    // Verify swap result leaves are in the pool
    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "swap_result"));

    // At T2, set_leaves with stale data
    let stale_refresh_data = vec![create_test_tree_node("node1", 1000)];
    store
        .set_leaves(&stale_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // set_leaves should have been SKIPPED
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "swap_result"),
        "Swap result leaf should be preserved after skipped set_leaves"
    );
    assert!(
        !available.iter().any(|l| l.id.to_string() == "node1"),
        "Stale leaf should not be restored when set_leaves is skipped"
    );
}

pub async fn test_set_leaves_proceeds_after_swap_when_refresh_starts_later(store: &dyn TreeStore) {
    // Add initial leaves
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves for a swap
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Swap completes first
    let new_leaves_from_swap = vec![create_test_tree_node("swap_result", 500)];
    store
        .finalize_reservation(&reservation.id, Some(&new_leaves_from_swap))
        .await
        .unwrap();

    // Small delay to ensure refresh starts AFTER swap completed
    tokio_with_wasm::alias::time::sleep(Duration::from_millis(10)).await;

    // Refresh starts AFTER swap completed
    let refresh_start = future_refresh_start();

    // set_leaves with fresh data
    let fresh_refresh_data = vec![
        create_test_tree_node("swap_result", 500),
        create_test_tree_node("new_deposit", 200),
    ];
    store
        .set_leaves(&fresh_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // set_leaves should have proceeded normally
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "swap_result"),
        "swap_result should be present"
    );
    assert!(
        available.iter().any(|l| l.id.to_string() == "new_deposit"),
        "new_deposit should be added"
    );
}

pub async fn test_payment_reservation_does_not_block_set_leaves(store: &dyn TreeStore) {
    // Add initial leaves
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves for PAYMENT (not swap)
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    let refresh_start = future_refresh_start();

    // set_leaves should proceed (payment reservation doesn't block)
    let new_leaves = vec![
        create_test_tree_node("node1", 150),
        create_test_tree_node("node3", 300),
    ];
    store
        .set_leaves(&new_leaves, &[], refresh_start)
        .await
        .unwrap();

    // node3 should be in the pool (set_leaves was not skipped)
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "node3"),
        "New leaf should be added when payment reservation is active"
    );
}

pub async fn test_update_reservation_basic(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve node1 for payment
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Update the reservation: swap completed, new reserved + change leaves
    let swap_output = create_test_tree_node("swap_output", 500);
    let change = create_test_tree_node("change", 500);
    let updated = store
        .update_reservation(&reservation.id, &[swap_output], &[change])
        .await
        .unwrap();

    // Same reservation ID
    assert_eq!(updated.id, reservation.id);

    let all = get_all(store).await;
    // Reserved leaves should be updated
    assert_eq!(all.reserved_for_payment.len(), 1);
    assert!(
        all.reserved_for_payment
            .iter()
            .any(|l| l.id.to_string() == "swap_output")
    );
    // Change leaf should be available
    assert!(
        all.available
            .iter()
            .any(|l| l.id.to_string() == "change")
    );
    assert_eq!(all.available_balance(), 500);
}

pub async fn test_update_reservation_nonexistent(store: &dyn TreeStore) {
    let leaf = create_test_tree_node("node1", 100);
    let fake_id = "nonexistent".to_string();
    let result = store
        .update_reservation(&fake_id, std::slice::from_ref(&leaf), &[])
        .await;
    assert!(result.is_err());
}

pub async fn test_update_reservation_clears_pending(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve 100 from a 1000-sat leaf (pending=900)
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    let reservation_id = match r1 {
        ReserveResult::Success(r) => r.id,
        _ => panic!("Expected Success"),
    };

    // Second reserve should get WaitForPending (pending=900 exists)
    let r2 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(
        matches!(r2, ReserveResult::WaitForPending { .. }),
        "Expected WaitForPending, got {r2:?}"
    );

    // Update reservation: reserved=[100], change=[900]
    let reserved_leaf = create_test_tree_node("reserved", 100);
    let change_leaf = create_test_tree_node("change", 900);
    store
        .update_reservation(&reservation_id, &[reserved_leaf], &[change_leaf])
        .await
        .unwrap();

    // Now pending is cleared and change(900) is available.
    // Reserve 300 should succeed.
    let r3 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(
        matches!(r3, ReserveResult::Success(_)),
        "Expected Success after pending cleared, got {r3:?}"
    );
}

pub async fn test_update_reservation_preserves_purpose(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve for Swap purpose
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Update reservation
    let swap_output = create_test_tree_node("swap_output", 600);
    let change = create_test_tree_node("change", 400);
    store
        .update_reservation(&reservation.id, &[swap_output], &[change])
        .await
        .unwrap();

    let all = get_all(store).await;
    // Should be in reserved_for_swap, not reserved_for_payment
    assert!(
        all.reserved_for_payment.is_empty(),
        "Updated swap reservation should not appear in reserved_for_payment"
    );
    assert_eq!(all.reserved_for_swap.len(), 1);
    assert!(
        all.reserved_for_swap
            .iter()
            .any(|l| l.id.to_string() == "swap_output")
    );
    // balance() includes swap-reserved + available
    assert_eq!(all.swap_reserved_balance(), 600);
    assert_eq!(all.available_balance(), 400);
    assert_eq!(all.balance(), 600 + 400);
}

pub async fn test_get_leaves_not_available(store: &dyn TreeStore) {
    let mut locked_leaf = create_test_tree_node("locked", 100);
    locked_leaf.status = TreeNodeStatus::TransferLocked;
    let available_leaf = create_test_tree_node("avail", 200);

    store
        .add_leaves(&[locked_leaf, available_leaf])
        .await
        .unwrap();

    let all = get_all(store).await;
    assert_eq!(all.not_available.len(), 1);
    assert!(
        all.not_available
            .iter()
            .any(|l| l.id.to_string() == "locked")
    );
    assert_eq!(all.available.len(), 1);
    assert!(
        all.available
            .iter()
            .any(|l| l.id.to_string() == "avail")
    );
    // available_balance excludes locked leaf
    assert_eq!(all.available_balance(), 200);
}

pub async fn test_get_leaves_missing_operators_filters_spent(store: &dyn TreeStore) {
    // Add node1 and reserve+finalize it (mark as spent)
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // Call set_leaves with refresh_start BEFORE the spend, so spent marker is preserved
    let refresh_start = SystemTime::now() - Duration::from_secs(60);
    let missing = vec![
        create_test_tree_node("node1", 100), // was spent
        create_test_tree_node("node3", 300), // new
    ];
    store
        .set_leaves(&[], &missing, refresh_start)
        .await
        .unwrap();

    let all = get_all(store).await;
    // node1 should be filtered out (spent), only node3 remains
    assert_eq!(all.available_missing_from_operators.len(), 1);
    assert!(
        all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "node3")
    );
    assert!(
        !all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "node1"),
        "Spent leaf node1 should be filtered from missing_operators"
    );
}

pub async fn test_missing_operators_replaced_on_set_leaves(store: &dyn TreeStore) {
    let refresh_start1 = future_refresh_start();
    let missing1 = vec![create_test_tree_node("missing1", 100)];
    store
        .set_leaves(&[], &missing1, refresh_start1)
        .await
        .unwrap();

    // Verify missing1 is present
    let all = get_all(store).await;
    assert_eq!(all.available_missing_from_operators.len(), 1);
    assert!(
        all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "missing1")
    );

    // Second set_leaves replaces with missing2
    let refresh_start2 = future_refresh_start();
    let missing2 = vec![create_test_tree_node("missing2", 200)];
    store
        .set_leaves(&[], &missing2, refresh_start2)
        .await
        .unwrap();

    let all = get_all(store).await;
    assert_eq!(all.available_missing_from_operators.len(), 1);
    assert!(
        all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "missing2")
    );
    assert!(
        !all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "missing1"),
        "missing1 should be replaced, not accumulated"
    );
}

pub async fn test_reserve_with_none_target_reserves_all(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve with None target -> should reserve all leaves
    let reservation = reserve_leaves(store, None, false, ReservationPurpose::Payment)
        .await
        .unwrap();

    assert_eq!(reservation.leaves.len(), 3);
    let all = get_all(store).await;
    assert!(all.available.is_empty(), "All leaves should be reserved");
    assert_eq!(all.reserved_for_payment.len(), 3);
}

pub async fn test_reserve_skips_non_available_leaves(store: &dyn TreeStore) {
    let node1 = create_test_tree_node("node1", 100);
    let mut node2 = create_test_tree_node("node2", 200);
    node2.status = TreeNodeStatus::TransferLocked;
    let node3 = create_test_tree_node("node3", 300);

    store.add_leaves(&[node1, node2, node3]).await.unwrap();

    // Reserve 400 exact -> should pick node1(100) + node3(300)
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(400, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    assert_eq!(reservation.sum(), 400);
    let all = get_all(store).await;
    // node2 should remain in not_available
    assert_eq!(all.not_available.len(), 1);
    assert!(
        all.not_available
            .iter()
            .any(|l| l.id.to_string() == "node2")
    );
    // Available pool should be empty (node1 and node3 were reserved)
    assert!(all.available.is_empty());
}

pub async fn test_add_leaves_empty_slice(store: &dyn TreeStore) {
    // Adding empty slice should succeed with no state change
    store.add_leaves(&[]).await.unwrap();
    assert_available_count(store, 0).await;

    // Add a real leaf, then add empty slice again
    let leaf = create_test_tree_node("node1", 100);
    store.add_leaves(&[leaf]).await.unwrap();
    assert_available_count(store, 1).await;

    store.add_leaves(&[]).await.unwrap();
    assert_available_count(store, 1).await;
}

pub async fn test_full_payment_cycle(store: &dyn TreeStore) {
    // Add node1(1000)
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve 400 (non-exact, gets 1000)
    let reservation1 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(400, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    assert_eq!(reservation1.sum(), 1000);

    // Finalize with change=[change(600)]
    let change_leaf = create_test_tree_node("change", 600);
    store
        .finalize_reservation(&reservation1.id, Some(&[change_leaf]))
        .await
        .unwrap();

    // change(600) should now be available
    let all = get_all(store).await;
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available_balance(), 600);

    // Reserve 600 (exact) from change
    let reservation2 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(600, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    assert_eq!(reservation2.sum(), 600);

    // Finalize with no new leaves
    store
        .finalize_reservation(&reservation2.id, None)
        .await
        .unwrap();

    // Store should be empty
    let all = get_all(store).await;
    assert!(all.available.is_empty());
    assert!(all.reserved_for_payment.is_empty());
    assert!(all.reserved_for_swap.is_empty());
    assert_eq!(all.balance(), 0);
}

pub async fn test_set_leaves_replaces_fully(store: &dyn TreeStore) {
    let refresh_start1 = future_refresh_start();
    let initial = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store
        .set_leaves(&initial, &[], refresh_start1)
        .await
        .unwrap();
    assert_available_count(store, 2).await;

    // Second set_leaves with only node3
    let refresh_start2 = future_refresh_start();
    let replacement = vec![create_test_tree_node("node3", 300)];
    store
        .set_leaves(&replacement, &[], refresh_start2)
        .await
        .unwrap();

    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "node3"));
    assert!(!available.iter().any(|l| l.id.to_string() == "node1"));
    assert!(!available.iter().any(|l| l.id.to_string() == "node2"));
}
