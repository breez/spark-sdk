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
