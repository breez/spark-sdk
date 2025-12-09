use std::collections::HashMap;

use tokio::sync::Mutex;
use tracing::{error, trace, warn};
use uuid::Uuid;

use crate::tree::{
    Leaves, LeavesReservation, LeavesReservationId, ReservationPurpose, TargetAmounts, TreeNode,
    TreeNodeId, TreeNodeStatus, TreeServiceError, TreeStore, select_helper,
};

#[derive(Default)]
pub struct InMemoryTreeStore {
    leaves: Mutex<LeavesState>,
}

/// Entry in the reservation map, containing leaves and the purpose of the reservation.
#[derive(Clone)]
struct ReservationEntry {
    leaves: Vec<TreeNode>,
    purpose: ReservationPurpose,
}

#[derive(Default)]
struct LeavesState {
    leaves: HashMap<TreeNodeId, TreeNode>,
    missing_operators_leaves: HashMap<TreeNodeId, TreeNode>,
    leaves_reservations: HashMap<LeavesReservationId, ReservationEntry>,
}

#[macros::async_trait]
impl TreeStore for InMemoryTreeStore {
    async fn add_leaves(&self, leaves: &[TreeNode]) -> Result<(), TreeServiceError> {
        self.leaves
            .lock()
            .await
            .leaves
            .extend(leaves.iter().map(|l| (l.id.clone(), l.clone())));
        Ok(())
    }

    async fn get_leaves(&self) -> Result<Leaves, TreeServiceError> {
        let leaves = self.leaves.lock().await;

        // Separate reserved leaves by purpose
        let mut reserved_for_payment = Vec::new();
        let mut reserved_for_optimization = Vec::new();
        for entry in leaves.leaves_reservations.values() {
            match entry.purpose {
                ReservationPurpose::Payment => {
                    reserved_for_payment.extend(entry.leaves.iter().cloned());
                }
                ReservationPurpose::Optimization => {
                    reserved_for_optimization.extend(entry.leaves.iter().cloned());
                }
            }
        }

        Ok(Leaves {
            available: leaves
                .leaves
                .values()
                .filter(|leaf| leaf.status == TreeNodeStatus::Available)
                .cloned()
                .collect(),
            not_available: leaves
                .leaves
                .values()
                .filter(|leaf| leaf.status != TreeNodeStatus::Available)
                .cloned()
                .collect(),
            available_missing_from_operators: leaves
                .missing_operators_leaves
                .values()
                .filter(|leaf| leaf.status == TreeNodeStatus::Available)
                .cloned()
                .collect(),
            reserved_for_payment,
            reserved_for_optimization,
        })
    }

    async fn set_leaves(
        &self,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let mut leaves_state = self.leaves.lock().await;
        leaves_state.leaves = leaves.iter().map(|l| (l.id.clone(), l.clone())).collect();
        leaves_state.missing_operators_leaves = missing_operators_leaves
            .iter()
            .map(|l| (l.id.clone(), l.clone()))
            .collect();

        for (key, entry) in leaves_state.leaves_reservations.clone().iter() {
            // remove leaves not existing in the main pool
            let mut filtered_leaves: Vec<TreeNode> = entry
                .leaves
                .iter()
                .filter(|l| {
                    leaves_state.leaves.contains_key(&l.id)
                        || leaves_state.missing_operators_leaves.contains_key(&l.id)
                })
                .cloned()
                .collect();

            // update reserved leaves that just got updated in the main pool
            for l in filtered_leaves.iter_mut() {
                if let Some(leaf) = leaves_state.leaves.remove(&l.id) {
                    *l = leaf;
                }
                if let Some(leaf) = leaves_state.missing_operators_leaves.remove(&l.id) {
                    *l = leaf;
                }
            }
            if filtered_leaves.is_empty() {
                leaves_state.leaves_reservations.remove(key);
            } else {
                leaves_state.leaves_reservations.insert(
                    key.clone(),
                    ReservationEntry {
                        leaves: filtered_leaves,
                        purpose: entry.purpose,
                    },
                );
            }
        }
        trace!("Updated {:?} leaves in the local state", leaves.len());
        Ok(())
    }

    async fn reserve_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        trace!("Reserving leaves for amounts: {target_amounts:?}, purpose: {purpose:?}");
        let reservation = {
            // Filter available leaves from the state
            let leaves: Vec<TreeNode> = self.get_leaves().await?.available.into_iter().collect();
            // Select leaves that match the target amounts
            let target_leaves_res =
                select_helper::select_leaves_by_target_amounts(&leaves, target_amounts);
            let selected = match target_leaves_res {
                Ok(target_leaves) => {
                    // Successfully selected target leaves
                    trace!("Successfully selected target leaves");
                    [
                        target_leaves.amount_leaves,
                        target_leaves.fee_leaves.unwrap_or_default(),
                    ]
                    .concat()
                }
                Err(_) if !exact_only => {
                    trace!("No exact match found, selecting leaves by minimum amount");
                    let target_amount_sat = target_amounts.map_or(0, |ta| ta.total_sats());
                    let Some(selected) =
                        select_helper::select_leaves_by_minimum_amount(&leaves, target_amount_sat)?
                    else {
                        return Err(TreeServiceError::UnselectableAmount);
                    };
                    selected
                }
                Err(e) => {
                    error!("Failed to select target leaves: {e:?}");
                    return Err(e);
                }
            };

            let reservation_id = self.reserve_leaves_internal(&selected, purpose).await?;
            LeavesReservation::new(selected, reservation_id)
        };

        Ok(reservation)
    }

    // move leaves back from the reserved pool to the main pool
    async fn cancel_reservation(&self, id: &LeavesReservationId) -> Result<(), TreeServiceError> {
        let mut leaves_state = self.leaves.lock().await;
        if let Some(entry) = leaves_state.leaves_reservations.remove(id) {
            for leaf in entry.leaves {
                leaves_state.leaves.insert(leaf.id.clone(), leaf.clone());
            }
        }
        trace!("Canceled leaves reservation: {}", id);
        Ok(())
    }

    // remove the leaves from the reserved pool, they are now considered used and
    // not available anymore.
    // If resulting_leaves are provided, they are added to the main pool.
    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        let mut leaves_state = self.leaves.lock().await;
        if leaves_state.leaves_reservations.remove(id).is_none() {
            warn!("Tried to finalize a non existing reservation");
        }
        if let Some(resulting_leaves) = new_leaves {
            leaves_state
                .leaves
                .extend(resulting_leaves.iter().map(|l| (l.id.clone(), l.clone())))
        }
        trace!("Finalized leaves reservation: {}", id);
        Ok(())
    }
}

impl InMemoryTreeStore {
    pub fn new() -> Self {
        InMemoryTreeStore {
            leaves: Mutex::new(LeavesState::default()),
        }
    }

    // Reserves leaves by moving them from the main pool to the reserved pool.
    // If accept_new_leaves is true, allows reserving leaves that are not in the main pool.
    // If false, only allows reserving leaves that are already in the main pool.
    async fn reserve_leaves_internal(
        &self,
        leaves: &[TreeNode],
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservationId, TreeServiceError> {
        let mut leaves_state = self.leaves.lock().await;
        if leaves.is_empty() {
            return Err(TreeServiceError::NonReservableLeaves);
        }
        for leaf in leaves {
            if !leaves_state.leaves.contains_key(&leaf.id) {
                return Err(TreeServiceError::NonReservableLeaves);
            }
        }
        let id = Uuid::now_v7().to_string();
        leaves_state.leaves_reservations.insert(
            id.clone(),
            ReservationEntry {
                leaves: leaves.to_vec(),
                purpose,
            },
        );
        for leaf in leaves {
            leaves_state.leaves.remove(&leaf.id);
        }
        trace!("New leaves reservation {}: {:?}", id, leaves);
        Ok(id)
    }

    #[cfg(test)]
    async fn get_reservation(&self, id: &LeavesReservationId) -> Option<Vec<TreeNode>> {
        let leaves_state = self.leaves.lock().await;
        leaves_state
            .leaves_reservations
            .get(id)
            .map(|entry| entry.leaves.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::ReservationPurpose;
    use bitcoin::{Transaction, absolute::LockTime, secp256k1::PublicKey, transaction::Version};
    use frost_secp256k1_tr::Identifier;
    use macros::async_test_all;
    use std::str::FromStr;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn create_test_tree_node(id: &str, value: u64) -> TreeNode {
        TreeNode {
            id: TreeNodeId::from_str(id).unwrap(),
            tree_id: "test_tree".to_string(),
            value,
            parent_node_id: None,
            node_tx: Transaction {
                version: Version::TWO,
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

    #[async_test_all]
    async fn test_new() {
        let state: InMemoryTreeStore = InMemoryTreeStore::new();
        assert!(state.leaves.lock().await.leaves.is_empty());
        assert!(state.leaves.lock().await.leaves_reservations.is_empty());
    }

    #[async_test_all]
    async fn test_add_leaves() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];

        state.add_leaves(&leaves).await.unwrap();

        let stored_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(stored_leaves.len(), 2);
        assert!(
            stored_leaves
                .iter()
                .any(|l| l.id.to_string() == "node1" && l.value == 100)
        );
        assert!(
            stored_leaves
                .iter()
                .any(|l| l.id.to_string() == "node2" && l.value == 200)
        );
    }

    #[async_test_all]
    async fn test_add_leaves_duplicate_ids() {
        let state = InMemoryTreeStore::new();
        let leaf1 = create_test_tree_node("node1", 100);
        let leaf2 = create_test_tree_node("node1", 200); // Same ID, different value

        state.add_leaves(&[leaf1]).await.unwrap();
        state.add_leaves(&[leaf2]).await.unwrap();

        let stored_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(stored_leaves.len(), 1);
        // Should have the second value (200) as it overwrites the first
        assert_eq!(stored_leaves[0].value, 200);
    }

    #[async_test_all]
    async fn test_set_leaves() {
        let state = InMemoryTreeStore::new();
        let initial_leaves = vec![create_test_tree_node("node1", 100)];
        state.add_leaves(&initial_leaves).await.unwrap();

        let new_leaves = vec![
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.set_leaves(&new_leaves, &[]).await.unwrap();

        let stored_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(stored_leaves.len(), 2);
        assert!(stored_leaves.iter().any(|l| l.id.to_string() == "node2"));
        assert!(stored_leaves.iter().any(|l| l.id.to_string() == "node3"));
        assert!(!stored_leaves.iter().any(|l| l.id.to_string() == "node1"));
    }

    #[async_test_all]
    async fn test_set_leaves_with_reservations() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve some leaves
        let reservation = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(600, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Update leaves with new data (including updated versions of reserved leaves)
        let non_existing_operator_leaf = create_test_tree_node("node7", 1000); // Updated value
        let mut updated_leaf1 = create_test_tree_node("node1", 150); // Updated value
        updated_leaf1.status = crate::tree::TreeNodeStatus::TransferLocked;
        let new_leaves = vec![
            updated_leaf1,
            create_test_tree_node("node2", 250), // Updated value
            create_test_tree_node("node4", 400), // New leaf, node3 removed
        ];
        state
            .set_leaves(&new_leaves, &[non_existing_operator_leaf])
            .await
            .unwrap();

        // Check that reserved leaves were updated with new data
        let reservation = state.get_reservation(&reservation.id).await.unwrap();
        assert_eq!(reservation.len(), 2);
        assert_eq!(reservation[0].value, 150);
        assert_eq!(
            reservation[0].status,
            crate::tree::TreeNodeStatus::TransferLocked
        );
        assert_eq!(reservation[1].value, 250);

        // Check main pool
        let all_leaves = state.get_leaves().await.unwrap();
        assert_eq!(all_leaves.payment_reserved_balance(), 400);
        assert_eq!(all_leaves.available_balance(), 400);
        assert_eq!(all_leaves.missing_operators_balance(), 1000);
        // balance() excludes payment-reserved leaves
        assert_eq!(all_leaves.balance(), 400 + 1000);
        assert_eq!(all_leaves.available.len(), 1); // Only node4 should be in main pool
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node4")
        );
    }

    #[async_test_all]
    async fn test_set_leaves_removes_non_existing_from_reservations() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve leaves
        let reservation = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(300, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Set new leaves that don't include the reserved ones
        let new_leaves = vec![create_test_tree_node("node3", 300)];
        state.set_leaves(&new_leaves, &[]).await.unwrap();

        // Reserved leaves should be removed since they don't exist in main pool
        let leaves_state = state.leaves.lock().await;
        let reservation = leaves_state.leaves_reservations.get(&reservation.id);
        assert!(reservation.is_none());
    }

    #[async_test_all]
    async fn test_reserve_leaves() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        let reservation = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Check that reservation was created
        let reserved = state.get_reservation(&reservation.id).await.unwrap();
        assert_eq!(reserved.len(), 1);
        assert_eq!(reserved[0].id, leaves[0].id);
        // Check that leaf was removed from main pool
        let main_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[1].id);
    }

    #[async_test_all]
    async fn test_cancel_reservation() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        let reservation = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Cancel the reservation
        state.cancel_reservation(&reservation.id).await.unwrap();

        // Check that reservation was removed
        assert!(state.get_reservation(&reservation.id).await.is_none());

        // Check that leaf was returned to main pool
        let main_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(main_leaves.len(), 2);
        assert!(main_leaves.iter().any(|l| l.id == leaves[0].id));
        assert!(main_leaves.iter().any(|l| l.id == leaves[1].id));
    }

    #[async_test_all]
    async fn test_cancel_reservation_nonexistent() {
        let state = InMemoryTreeStore::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.cancel_reservation(&fake_id).await.unwrap();

        let leaves_state = state.leaves.lock().await;
        assert!(leaves_state.leaves_reservations.is_empty());
        assert!(leaves_state.leaves.is_empty());
    }

    #[async_test_all]
    async fn test_finalize_reservation() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        let reservation = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Finalize the reservation
        state
            .finalize_reservation(&reservation.id, None)
            .await
            .unwrap();

        // Check that reservation was removed
        assert!(state.get_reservation(&reservation.id).await.is_none());

        // Check that leaf was NOT returned to main pool (it's considered used)
        let main_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[1].id);
    }

    #[async_test_all]
    async fn test_finalize_reservation_nonexistent() {
        let state = InMemoryTreeStore::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.finalize_reservation(&fake_id, None).await.unwrap();

        let leaves_state = state.leaves.lock().await;
        assert!(leaves_state.leaves_reservations.is_empty());
        drop(leaves_state);

        let main_leaves = state.get_leaves().await.unwrap().available;
        assert!(main_leaves.is_empty());
    }

    #[async_test_all]
    async fn test_multiple_reservations() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Create multiple reservations
        let reservation1 = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();
        let reservation2 = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(200, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Check both reservations exist
        assert!(state.get_reservation(&reservation1.id).await.is_some());
        assert!(state.get_reservation(&reservation2.id).await.is_some());
        assert_eq!(
            state.get_reservation(&reservation1.id).await.unwrap().len(),
            1
        );
        assert_eq!(
            state.get_reservation(&reservation2.id).await.unwrap().len(),
            1
        );

        // Check main pool has only one leaf left
        let main_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[2].id);

        // Cancel one reservation
        state.cancel_reservation(&reservation1.id).await.unwrap();
        assert!(state.get_reservation(&reservation1.id).await.is_none());
        assert_eq!(state.get_leaves().await.unwrap().available.len(), 2);

        // Finalize the other
        state
            .finalize_reservation(&reservation2.id, None)
            .await
            .unwrap();
        assert!(state.get_reservation(&reservation2.id).await.is_none());
        assert_eq!(state.get_leaves().await.unwrap().available.len(), 2); // node1 returned, node3 was always there
    }

    #[async_test_all]
    async fn test_reservation_ids_are_unique() {
        let state = InMemoryTreeStore::new();
        let leaf = create_test_tree_node("node1", 100);
        state.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();

        let r1 = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();
        state.cancel_reservation(&r1.id).await.unwrap();
        let r2 = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        assert_ne!(r1.id, r2.id);
    }

    #[async_test_all]
    async fn test_non_reservable_leaves() {
        let state = InMemoryTreeStore::new();
        let leaf = create_test_tree_node("node1", 100);
        state.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();

        state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();
        let result = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap_err();
        assert!(matches!(result, TreeServiceError::InsufficientFunds));
    }

    #[async_test_all]
    async fn test_reserve_leaves_empty() {
        let state = InMemoryTreeStore::new();
        let err = state
            .reserve_leaves(None, false, ReservationPurpose::Payment)
            .await
            .unwrap_err();

        assert!(matches!(err, TreeServiceError::NonReservableLeaves));
    }

    #[async_test_all]
    async fn test_optimization_reservation_included_in_balance() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve some leaves for optimization
        let _reservation = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(300, None)),
                true,
                ReservationPurpose::Optimization,
            )
            .await
            .unwrap();

        // Check that optimization-reserved leaves are included in balance
        let all_leaves = state.get_leaves().await.unwrap();
        assert_eq!(all_leaves.optimization_reserved_balance(), 300);
        assert_eq!(all_leaves.available_balance(), 300); // node1 + node2 remaining
        // balance() should include optimization-reserved leaves
        assert_eq!(all_leaves.balance(), 300 + 300); // available + optimization-reserved
    }

    #[async_test_all]
    async fn test_payment_reservation_excluded_from_balance() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve some leaves for payment
        let _reservation = state
            .reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(300, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Check that payment-reserved leaves are excluded from balance
        let all_leaves = state.get_leaves().await.unwrap();
        assert_eq!(all_leaves.payment_reserved_balance(), 300);
        assert_eq!(all_leaves.available_balance(), 300); // node1 + node2 remaining
        // balance() should NOT include payment-reserved leaves
        assert_eq!(all_leaves.balance(), 300); // only available
    }
}
