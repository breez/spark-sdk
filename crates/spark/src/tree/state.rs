use std::collections::HashMap;

use uuid::Uuid;

use crate::tree::{LeavesReservationId, TreeNode, TreeNodeId};

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
        for (_, reserved_leaves) in self.leaves_reservations.iter_mut() {
            // remove leaves not existing in the main pool
            reserved_leaves.retain(|l| self.leaves.contains_key(&l.id));

            //Replace every new leaf we got with the corresponding in the reserve pool
            for i in 0..reserved_leaves.len() {
                if let Some(leaf) = self.leaves.remove(&reserved_leaves[i].id) {
                    reserved_leaves[i] = leaf;
                }
            }
        }
    }

    // move leaves from the main pool to the reserved pool
    pub fn reserve_leaves(&mut self, leaves: &[TreeNode]) -> LeavesReservationId {
        let id = Uuid::now_v7().to_string();
        self.leaves_reservations.insert(id.clone(), leaves.to_vec());
        for leaf in leaves {
            self.leaves.remove(&leaf.id);
        }
        id
    }

    // move leaves back from the reserved pool to the main pool
    pub fn cancel_reservation(&mut self, id: LeavesReservationId) {
        if let Some(leaves) = self.leaves_reservations.remove(&id) {
            for leaf in leaves {
                self.leaves.insert(leaf.id.clone(), leaf.clone());
            }
        }
    }

    // remove the leaves from the reserved pool, they are now considered used and
    // not available anymore.
    pub fn finalize_reservation(&mut self, id: LeavesReservationId) {
        self.leaves_reservations.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{Transaction, absolute::LockTime, secp256k1::PublicKey, transaction::Version};
    use frost_secp256k1_tr::Identifier;
    use std::str::FromStr;

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

    #[test]
    fn test_new() {
        let state = TreeState::new();
        assert!(state.leaves.is_empty());
        assert!(state.leaves_reservations.is_empty());
    }

    #[test]
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

    #[test]
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

    #[test]
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

    #[test]
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
        let reservation_id = state.reserve_leaves(&reserved_leaves);

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

    #[test]
    fn test_set_leaves_removes_non_existing_from_reservations() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves);

        // Reserve leaves
        let reservation_id = state.reserve_leaves(&leaves);

        // Set new leaves that don't include the reserved ones
        let new_leaves = vec![create_test_tree_node("node3", 300)];
        state.set_leaves(&new_leaves);

        // Reserved leaves should be removed since they don't exist in main pool
        let reservation = state.leaves_reservations.get(&reservation_id).unwrap();
        assert!(reservation.is_empty());
    }

    #[test]
    fn test_reserve_leaves() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves);

        let to_reserve = vec![leaves[0].clone()];
        let reservation_id = state.reserve_leaves(&to_reserve);

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

    #[test]
    fn test_reserve_leaves_empty() {
        let mut state = TreeState::new();
        let reservation_id = state.reserve_leaves(&[]);

        assert!(state.leaves_reservations.contains_key(&reservation_id));
        let reserved = state.leaves_reservations.get(&reservation_id).unwrap();
        assert!(reserved.is_empty());
    }

    #[test]
    fn test_cancel_reservation() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves);

        let to_reserve = vec![leaves[0].clone()];
        let reservation_id = state.reserve_leaves(&to_reserve);

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

    #[test]
    fn test_cancel_reservation_nonexistent() {
        let mut state = TreeState::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.cancel_reservation(fake_id);

        assert!(state.leaves_reservations.is_empty());
        assert!(state.get_leaves().is_empty());
    }

    #[test]
    fn test_finalize_reservation() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves);

        let to_reserve = vec![leaves[0].clone()];
        let reservation_id = state.reserve_leaves(&to_reserve);

        // Finalize the reservation
        state.finalize_reservation(reservation_id.clone());

        // Check that reservation was removed
        assert!(!state.leaves_reservations.contains_key(&reservation_id));

        // Check that leaf was NOT returned to main pool (it's considered used)
        let main_leaves = state.get_leaves();
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[1].id);
    }

    #[test]
    fn test_finalize_reservation_nonexistent() {
        let mut state = TreeState::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.finalize_reservation(fake_id);

        assert!(state.leaves_reservations.is_empty());
        assert!(state.get_leaves().is_empty());
    }

    #[test]
    fn test_multiple_reservations() {
        let mut state = TreeState::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves);

        // Create multiple reservations
        let reservation1 = state.reserve_leaves(&[leaves[0].clone()]);
        let reservation2 = state.reserve_leaves(&[leaves[1].clone()]);

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

    #[test]
    fn test_reservation_ids_are_unique() {
        let mut state = TreeState::new();
        let leaf = create_test_tree_node("node1", 100);
        state.add_leaves(&[leaf.clone()]);

        let id1 = state.reserve_leaves(&[leaf.clone()]);
        state.cancel_reservation(id1.clone());
        let id2 = state.reserve_leaves(&[leaf.clone()]);

        assert_ne!(id1, id2);
    }
}
