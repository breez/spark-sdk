use std::collections::HashMap;

use tracing::{trace, warn};
use uuid::Uuid;

use crate::tree::{LeavesReservationId, TreeNode, TreeNodeId, TreeServiceError};

// TODO: Implement proper tree state logic.
pub struct TreeState {
    leaves: HashMap<TreeNodeId, TreeNode>,
    leaves_reservations: HashMap<LeavesReservationId, Vec<TreeNode>>,
}

impl Default for TreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeState {
    pub fn new() -> Self {
        TreeState {
            leaves: HashMap::new(),
            leaves_reservations: HashMap::new(),
        }
    }

    pub fn add_leaves(&mut self, leaves: &[TreeNode]) {
        self.leaves
            .extend(leaves.iter().map(|l| (l.id.clone(), l.clone())));
    }

    pub fn get_leaves(&self) -> Vec<TreeNode> {
        self.leaves.values().cloned().collect()
    }

    pub fn set_leaves(&mut self, leaves: &[TreeNode]) {
        self.leaves = leaves.iter().map(|l| (l.id.clone(), l.clone())).collect();

        for (key, reserved_leaves) in self.leaves_reservations.clone().iter() {
            // remove leaves not existing in the main pool
            let mut filtered_leaves: Vec<TreeNode> = reserved_leaves
                .iter()
                .filter(|l| self.leaves.contains_key(&l.id))
                .cloned()
                .collect();

            // update reserved leaves that just got updated in the main pool
            for l in filtered_leaves.iter_mut() {
                if let Some(leaf) = self.leaves.remove(&l.id) {
                    *l = leaf;
                }
            }
            if filtered_leaves.is_empty() {
                self.leaves_reservations.remove(key);
            } else {
                self.leaves_reservations
                    .insert(key.clone(), filtered_leaves);
            }
        }
        trace!("Updated {:?} leaves in the local state", leaves.len());
    }

    pub fn remove_leaves(&mut self, leaf_ids: &[TreeNodeId]) {
        for leaf_id in leaf_ids {
            self.leaves.remove(leaf_id);
        }
        trace!("Removed {} leaves from the local state", leaf_ids.len());
    }

    // Reserves leaves by moving them from the main pool to the reserved pool.
    // If accept_new_leaves is true, allows reserving leaves that are not in the main pool.
    // If false, only allows reserving leaves that are already in the main pool.
    pub fn reserve_leaves(
        &mut self,
        leaves: &[TreeNode],
        accept_new_leaves: bool,
    ) -> Result<LeavesReservationId, TreeServiceError> {
        if leaves.is_empty() {
            return Err(TreeServiceError::NonReservableLeaves);
        }
        if !accept_new_leaves {
            for leaf in leaves {
                if !self.leaves.contains_key(&leaf.id) {
                    return Err(TreeServiceError::NonReservableLeaves);
                }
            }
        }
        let id = Uuid::now_v7().to_string();
        self.leaves_reservations.insert(id.clone(), leaves.to_vec());
        for leaf in leaves {
            self.leaves.remove(&leaf.id);
        }
        trace!("New leaves reservation {}: {:?}", id, leaves);
        Ok(id)
    }

    // move leaves back from the reserved pool to the main pool
    pub fn cancel_reservation(&mut self, id: LeavesReservationId) {
        if let Some(leaves) = self.leaves_reservations.remove(&id) {
            for leaf in leaves {
                self.leaves.insert(leaf.id.clone(), leaf.clone());
            }
        }
        trace!("Canceled leaves reservation: {}", id);
    }

    // remove the leaves from the reserved pool, they are now considered used and
    // not available anymore.
    pub fn finalize_reservation(&mut self, id: LeavesReservationId) {
        if self.leaves_reservations.remove(&id).is_none() {
            warn!("Tried to finalize a non existing reservation");
        }
        trace!("Finalized leaves reservation: {}", id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{Transaction, absolute::LockTime, secp256k1::PublicKey, transaction::Version};
    use frost_secp256k1_tr::Identifier;
    use macros::test_all;
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

    #[test_all]
    fn test_new() {
        let state = TreeState::new();
        assert!(state.leaves.is_empty());
        assert!(state.leaves_reservations.is_empty());
    }

    #[test_all]
    fn test_add_leaves() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];

        state.add_leaves(&leaves);

        let stored_leaves = state.get_leaves();
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

    #[test_all]
    fn test_add_leaves_duplicate_ids() {
        let mut state = TreeState::new();
        let leaf1 = create_test_tree_node("node1", 100);
        let leaf2 = create_test_tree_node("node1", 200); // Same ID, different value

        state.add_leaves(&[leaf1]);
        state.add_leaves(&[leaf2]);

        let stored_leaves = state.get_leaves();
        assert_eq!(stored_leaves.len(), 1);
        // Should have the second value (200) as it overwrites the first
        assert_eq!(stored_leaves[0].value, 200);
    }

    #[test_all]
    fn test_set_leaves() {
        let mut state = TreeState::new();
        let initial_leaves = vec![create_test_tree_node("node1", 100)];
        state.add_leaves(&initial_leaves);

        let new_leaves = vec![
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.set_leaves(&new_leaves);

        let stored_leaves = state.get_leaves();
        assert_eq!(stored_leaves.len(), 2);
        assert!(stored_leaves.iter().any(|l| l.id.to_string() == "node2"));
        assert!(stored_leaves.iter().any(|l| l.id.to_string() == "node3"));
        assert!(!stored_leaves.iter().any(|l| l.id.to_string() == "node1"));
    }

    #[test_all]
    fn test_set_leaves_with_reservations() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves);

        // Reserve some leaves
        let reserved_leaves = vec![leaves[0].clone(), leaves[1].clone()];
        let reservation_id = state.reserve_leaves(&reserved_leaves, false).unwrap();

        // Update leaves with new data (including updated versions of reserved leaves)
        let mut updated_leaf1 = create_test_tree_node("node1", 150); // Updated value
        updated_leaf1.status = crate::tree::TreeNodeStatus::TransferLocked;
        let new_leaves = vec![
            updated_leaf1,
            create_test_tree_node("node2", 250), // Updated value
            create_test_tree_node("node4", 400), // New leaf, node3 removed
        ];
        state.set_leaves(&new_leaves);

        // Check that reserved leaves were updated with new data
        let reservation = state.leaves_reservations.get(&reservation_id).unwrap();
        assert_eq!(reservation.len(), 2);
        assert_eq!(reservation[0].value, 150);
        assert_eq!(
            reservation[0].status,
            crate::tree::TreeNodeStatus::TransferLocked
        );
        assert_eq!(reservation[1].value, 250);

        // Check main pool
        let main_leaves = state.get_leaves();
        assert_eq!(main_leaves.len(), 1); // Only node4 should be in main pool
        assert!(main_leaves.iter().any(|l| l.id.to_string() == "node4"));
    }

    #[test_all]
    fn test_set_leaves_removes_non_existing_from_reservations() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves);

        // Reserve leaves
        let reservation_id = state.reserve_leaves(&leaves, false).unwrap();

        // Set new leaves that don't include the reserved ones
        let new_leaves = vec![create_test_tree_node("node3", 300)];
        state.set_leaves(&new_leaves);

        // Reserved leaves should be removed since they don't exist in main pool
        let reservation = state.leaves_reservations.get(&reservation_id);
        assert!(reservation.is_none());
    }

    #[test_all]
    fn test_reserve_leaves() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves);

        let to_reserve = vec![leaves[0].clone()];
        let reservation_id = state.reserve_leaves(&to_reserve, false).unwrap();

        // Check that reservation was created
        assert!(state.leaves_reservations.contains_key(&reservation_id));
        let reserved = state.leaves_reservations.get(&reservation_id).unwrap();
        assert_eq!(reserved.len(), 1);
        assert_eq!(reserved[0].id, leaves[0].id);

        // Check that leaf was removed from main pool
        let main_leaves = state.get_leaves();
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[1].id);
    }

    #[test_all]
    fn test_cancel_reservation() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves);

        let to_reserve = vec![leaves[0].clone()];
        let reservation_id = state.reserve_leaves(&to_reserve, false).unwrap();

        // Cancel the reservation
        state.cancel_reservation(reservation_id.clone());

        // Check that reservation was removed
        assert!(!state.leaves_reservations.contains_key(&reservation_id));

        // Check that leaf was returned to main pool
        let main_leaves = state.get_leaves();
        assert_eq!(main_leaves.len(), 2);
        assert!(main_leaves.iter().any(|l| l.id == leaves[0].id));
        assert!(main_leaves.iter().any(|l| l.id == leaves[1].id));
    }

    #[test_all]
    fn test_cancel_reservation_nonexistent() {
        let mut state = TreeState::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.cancel_reservation(fake_id);

        assert!(state.leaves_reservations.is_empty());
        assert!(state.get_leaves().is_empty());
    }

    #[test_all]
    fn test_finalize_reservation() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves);

        let to_reserve = vec![leaves[0].clone()];
        let reservation_id = state.reserve_leaves(&to_reserve, false).unwrap();

        // Finalize the reservation
        state.finalize_reservation(reservation_id.clone());

        // Check that reservation was removed
        assert!(!state.leaves_reservations.contains_key(&reservation_id));

        // Check that leaf was NOT returned to main pool (it's considered used)
        let main_leaves = state.get_leaves();
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[1].id);
    }

    #[test_all]
    fn test_finalize_reservation_nonexistent() {
        let mut state = TreeState::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.finalize_reservation(fake_id);

        assert!(state.leaves_reservations.is_empty());
        assert!(state.get_leaves().is_empty());
    }

    #[test_all]
    fn test_multiple_reservations() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves);

        // Create multiple reservations
        let reservation1 = state.reserve_leaves(&[leaves[0].clone()], false).unwrap();
        let reservation2 = state.reserve_leaves(&[leaves[1].clone()], false).unwrap();

        // Check both reservations exist
        assert!(state.leaves_reservations.contains_key(&reservation1));
        assert!(state.leaves_reservations.contains_key(&reservation2));
        assert_eq!(state.leaves_reservations.len(), 2);

        // Check main pool has only one leaf left
        let main_leaves = state.get_leaves();
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[2].id);

        // Cancel one reservation
        state.cancel_reservation(reservation1);
        assert_eq!(state.leaves_reservations.len(), 1);
        assert_eq!(state.get_leaves().len(), 2);

        // Finalize the other
        state.finalize_reservation(reservation2);
        assert_eq!(state.leaves_reservations.len(), 0);
        assert_eq!(state.get_leaves().len(), 2); // node1 returned, node3 was always there
    }

    #[test_all]
    fn test_reservation_ids_are_unique() {
        let mut state = TreeState::new();
        let leaf = create_test_tree_node("node1", 100);
        state.add_leaves(std::slice::from_ref(&leaf));

        let id1 = state
            .reserve_leaves(std::slice::from_ref(&leaf), false)
            .unwrap();
        state.cancel_reservation(id1.clone());
        let id2 = state
            .reserve_leaves(std::slice::from_ref(&leaf), false)
            .unwrap();

        assert_ne!(id1, id2);
    }

    #[test_all]
    fn test_non_reservable_leaves() {
        let mut state = TreeState::new();
        let leaf = create_test_tree_node("node1", 100);
        state.add_leaves(std::slice::from_ref(&leaf));

        let _ = state
            .reserve_leaves(std::slice::from_ref(&leaf), false)
            .unwrap();
        let result = state
            .reserve_leaves(std::slice::from_ref(&leaf), false)
            .unwrap_err();
        assert!(matches!(result, TreeServiceError::NonReservableLeaves));
    }

    #[test_all]
    fn test_reserve_leaves_empty() {
        let mut state = TreeState::new();
        let err = state.reserve_leaves(&[], false).unwrap_err();

        assert!(matches!(err, TreeServiceError::NonReservableLeaves));
    }

    #[test_all]
    fn test_remove_leaves() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves);

        state.remove_leaves(&[
            TreeNodeId::from_str("node1").unwrap(),
            TreeNodeId::from_str("node3").unwrap(),
        ]);

        let remaining_leaves = state.get_leaves();
        assert_eq!(remaining_leaves.len(), 1);
        assert_eq!(remaining_leaves[0].id.to_string(), "node2");
    }

    #[test_all]
    fn test_reserve_leaves_accept_new_leaves() {
        let mut state = TreeState::new();
        let leaf = create_test_tree_node("new_node", 500);

        // Reserve a leaf that doesn't exist in the main pool
        let reservation_id = state
            .reserve_leaves(std::slice::from_ref(&leaf), true)
            .unwrap();

        // Check that reservation was created
        assert!(state.leaves_reservations.contains_key(&reservation_id));
        let reserved = state.leaves_reservations.get(&reservation_id).unwrap();
        assert_eq!(reserved.len(), 1);
        assert_eq!(reserved[0].id.to_string(), "new_node");

        // Check that the main pool is still empty
        assert!(state.get_leaves().is_empty());
    }

    #[test_all]
    fn test_reserve_leaves_mix_existing_and_new() {
        let mut state = TreeState::new();
        let existing_leaf = create_test_tree_node("existing", 100);
        state.add_leaves(std::slice::from_ref(&existing_leaf));

        let new_leaf = create_test_tree_node("new", 200);

        // Try to reserve both an existing and a new leaf
        let leaves_to_reserve = vec![existing_leaf.clone(), new_leaf.clone()];
        let reservation_id = state.reserve_leaves(&leaves_to_reserve, true).unwrap();

        // Check the reservation contains both leaves
        let reserved = state.leaves_reservations.get(&reservation_id).unwrap();
        assert_eq!(reserved.len(), 2);

        // Check the existing leaf is no longer in the main pool
        assert!(state.get_leaves().is_empty());
    }
}
