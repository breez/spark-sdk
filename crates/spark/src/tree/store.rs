use std::collections::HashMap;

use tokio::sync::Mutex;
use tracing::{error, trace, warn};
use uuid::Uuid;

use crate::tree::{
    LeavesReservation, LeavesReservationId, TargetAmounts, TreeNode, TreeNodeId, TreeNodeStatus,
    TreeServiceError, TreeStore, select_helper,
};

// TODO: Implement proper tree state logic.
pub struct InMemoryTreeStore {
    leaves: Mutex<LeavesState>,
}

#[derive(Default)]
struct LeavesState {
    leaves: HashMap<TreeNodeId, TreeNode>,
    leaves_reservations: HashMap<LeavesReservationId, Vec<TreeNode>>,
}

impl Default for InMemoryTreeStore {
    fn default() -> Self {
        Self::new()
    }
}

#[macros::async_trait]
impl TreeStore for InMemoryTreeStore {
    async fn add_leaves(&self, leaves: &[TreeNode]) {
        self.leaves
            .lock()
            .await
            .leaves
            .extend(leaves.iter().map(|l| (l.id.clone(), l.clone())));
    }

    async fn get_leaves(&self) -> Vec<TreeNode> {
        self.leaves.lock().await.leaves.values().cloned().collect()
    }

    async fn set_leaves(&self, leaves: &[TreeNode]) {
        let mut leaves_state = self.leaves.lock().await;
        leaves_state.leaves = leaves.iter().map(|l| (l.id.clone(), l.clone())).collect();

        for (key, reserved_leaves) in leaves_state.leaves_reservations.clone().iter() {
            // remove leaves not existing in the main pool
            let mut filtered_leaves: Vec<TreeNode> = reserved_leaves
                .iter()
                .filter(|l| leaves_state.leaves.contains_key(&l.id))
                .cloned()
                .collect();

            // update reserved leaves that just got updated in the main pool
            for l in filtered_leaves.iter_mut() {
                if let Some(leaf) = leaves_state.leaves.remove(&l.id) {
                    *l = leaf;
                }
            }
            if filtered_leaves.is_empty() {
                leaves_state.leaves_reservations.remove(key);
            } else {
                leaves_state
                    .leaves_reservations
                    .insert(key.clone(), filtered_leaves);
            }
        }
        trace!("Updated {:?} leaves in the local state", leaves.len());
    }

    async fn reserve_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
    ) -> Result<LeavesReservation, TreeServiceError> {
        trace!("Reserving leaves for amounts: {target_amounts:?}");
        let reservation = {
            // Filter available leaves from the state
            let leaves: Vec<TreeNode> = self
                .get_leaves()
                .await
                .into_iter()
                .filter(|leaf| leaf.status == TreeNodeStatus::Available)
                .collect();
            // Select leaves that match the target amounts
            let target_leaves_res =
                select_helper::select_leaves_by_amounts(&leaves, target_amounts);
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

            let reservation_id = self.reserve_leaves_internal(&selected).await?;
            LeavesReservation::new(selected, reservation_id)
        };

        Ok(reservation)
    }

    // move leaves back from the reserved pool to the main pool
    async fn cancel_reservation(&self, id: &LeavesReservationId) {
        let mut leaves_state = self.leaves.lock().await;
        if let Some(leaves) = leaves_state.leaves_reservations.remove(id) {
            for leaf in leaves {
                leaves_state.leaves.insert(leaf.id.clone(), leaf.clone());
            }
        }
        trace!("Canceled leaves reservation: {}", id);
    }

    // remove the leaves from the reserved pool, they are now considered used and
    // not available anymore.
    async fn finalize_reservation(&self, id: &LeavesReservationId) {
        let mut leaves_state = self.leaves.lock().await;
        if leaves_state.leaves_reservations.remove(id).is_none() {
            warn!("Tried to finalize a non existing reservation");
        }
        trace!("Finalized leaves reservation: {}", id);
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
        leaves_state
            .leaves_reservations
            .insert(id.clone(), leaves.to_vec());
        for leaf in leaves {
            leaves_state.leaves.remove(&leaf.id);
        }
        trace!("New leaves reservation {}: {:?}", id, leaves);
        Ok(id)
    }

    #[cfg(test)]
    async fn get_reservation(&self, id: &LeavesReservationId) -> Option<Vec<TreeNode>> {
        let leaves_state = self.leaves.lock().await;
        leaves_state.leaves_reservations.get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

        state.add_leaves(&leaves).await;

        let stored_leaves = state.get_leaves().await;
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

        state.add_leaves(&[leaf1]).await;
        state.add_leaves(&[leaf2]).await;

        let stored_leaves = state.get_leaves().await;
        assert_eq!(stored_leaves.len(), 1);
        // Should have the second value (200) as it overwrites the first
        assert_eq!(stored_leaves[0].value, 200);
    }

    #[async_test_all]
    async fn test_set_leaves() {
        let state = InMemoryTreeStore::new();
        let initial_leaves = vec![create_test_tree_node("node1", 100)];
        state.add_leaves(&initial_leaves).await;

        let new_leaves = vec![
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.set_leaves(&new_leaves).await;

        let stored_leaves = state.get_leaves().await;
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
        state.add_leaves(&leaves).await;

        // Reserve some leaves
        let reservation = state
            .reserve_leaves(Some(&TargetAmounts::new(600, None)), false)
            .await
            .unwrap();

        // Update leaves with new data (including updated versions of reserved leaves)
        let mut updated_leaf1 = create_test_tree_node("node1", 150); // Updated value
        updated_leaf1.status = crate::tree::TreeNodeStatus::TransferLocked;
        let new_leaves = vec![
            updated_leaf1,
            create_test_tree_node("node2", 250), // Updated value
            create_test_tree_node("node4", 400), // New leaf, node3 removed
        ];
        state.set_leaves(&new_leaves).await;

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
        let main_leaves = state.get_leaves().await;
        assert_eq!(main_leaves.len(), 1); // Only node4 should be in main pool
        assert!(main_leaves.iter().any(|l| l.id.to_string() == "node4"));
    }

    #[async_test_all]
    async fn test_set_leaves_removes_non_existing_from_reservations() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await;

        // Reserve leaves
        let reservation = state
            .reserve_leaves(Some(&TargetAmounts::new(300, None)), false)
            .await
            .unwrap();

        // Set new leaves that don't include the reserved ones
        let new_leaves = vec![create_test_tree_node("node3", 300)];
        state.set_leaves(&new_leaves).await;

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
        state.add_leaves(&leaves).await;

        let reservation = state
            .reserve_leaves(Some(&TargetAmounts::new(100, None)), true)
            .await
            .unwrap();

        // Check that reservation was created
        let reserved = state.get_reservation(&reservation.id).await.unwrap();
        assert_eq!(reserved.len(), 1);
        assert_eq!(reserved[0].id, leaves[0].id);
        // Check that leaf was removed from main pool
        let main_leaves = state.get_leaves().await;
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
        state.add_leaves(&leaves).await;

        let reservation = state
            .reserve_leaves(Some(&TargetAmounts::new(100, None)), true)
            .await
            .unwrap();

        // Cancel the reservation
        state.cancel_reservation(&reservation.id).await;

        // Check that reservation was removed
        assert!(state.get_reservation(&reservation.id).await.is_none());

        // Check that leaf was returned to main pool
        let main_leaves = state.get_leaves().await;
        assert_eq!(main_leaves.len(), 2);
        assert!(main_leaves.iter().any(|l| l.id == leaves[0].id));
        assert!(main_leaves.iter().any(|l| l.id == leaves[1].id));
    }

    #[async_test_all]
    async fn test_cancel_reservation_nonexistent() {
        let state = InMemoryTreeStore::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.cancel_reservation(&fake_id).await;

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
        state.add_leaves(&leaves).await;

        let reservation = state
            .reserve_leaves(Some(&TargetAmounts::new(100, None)), true)
            .await
            .unwrap();

        // Finalize the reservation
        state.finalize_reservation(&reservation.id).await;

        // Check that reservation was removed
        assert!(state.get_reservation(&reservation.id).await.is_none());

        // Check that leaf was NOT returned to main pool (it's considered used)
        let main_leaves = state.get_leaves().await;
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[1].id);
    }

    #[async_test_all]
    async fn test_finalize_reservation_nonexistent() {
        let state = InMemoryTreeStore::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.finalize_reservation(&fake_id).await;

        let leaves_state = state.leaves.lock().await;
        assert!(leaves_state.leaves_reservations.is_empty());
        drop(leaves_state);

        let main_leaves = state.get_leaves().await;
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
        state.add_leaves(&leaves).await;

        // Create multiple reservations
        let reservation1 = state
            .reserve_leaves(Some(&TargetAmounts::new(100, None)), true)
            .await
            .unwrap();
        let reservation2 = state
            .reserve_leaves(Some(&TargetAmounts::new(200, None)), true)
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
        let main_leaves = state.get_leaves().await;
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[2].id);

        // Cancel one reservation
        state.cancel_reservation(&reservation1.id).await;
        assert!(state.get_reservation(&reservation1.id).await.is_none());
        assert_eq!(state.get_leaves().await.len(), 2);

        // Finalize the other
        state.finalize_reservation(&reservation2.id).await;
        assert!(state.get_reservation(&reservation2.id).await.is_none());
        assert_eq!(state.get_leaves().await.len(), 2); // node1 returned, node3 was always there
    }

    #[async_test_all]
    async fn test_reservation_ids_are_unique() {
        let state = InMemoryTreeStore::new();
        let leaf = create_test_tree_node("node1", 100);
        state.add_leaves(std::slice::from_ref(&leaf)).await;

        let r1 = state
            .reserve_leaves(Some(&TargetAmounts::new(100, None)), true)
            .await
            .unwrap();
        state.cancel_reservation(&r1.id).await;
        let r2 = state
            .reserve_leaves(Some(&TargetAmounts::new(100, None)), true)
            .await
            .unwrap();

        assert_ne!(r1.id, r2.id);
    }

    #[async_test_all]
    async fn test_non_reservable_leaves() {
        let state = InMemoryTreeStore::new();
        let leaf = create_test_tree_node("node1", 100);
        state.add_leaves(std::slice::from_ref(&leaf)).await;

        let _ = state
            .reserve_leaves(Some(&TargetAmounts::new(100, None)), true)
            .await
            .unwrap();
        let result = state
            .reserve_leaves(Some(&TargetAmounts::new(100, None)), true)
            .await
            .unwrap_err();
        assert!(matches!(result, TreeServiceError::InsufficientFunds));
    }

    #[async_test_all]
    async fn test_reserve_leaves_empty() {
        let state = InMemoryTreeStore::new();
        let err = state.reserve_leaves(None, false).await.unwrap_err();

        assert!(matches!(err, TreeServiceError::NonReservableLeaves));
    }
}
