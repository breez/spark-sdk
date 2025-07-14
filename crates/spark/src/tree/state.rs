use std::collections::HashMap;

use uuid::Uuid;

use crate::tree::{PendingLeavesId, TreeNode, TreeNodeId};

// TODO: Implement proper tree state logic.
pub struct TreeState {
    leaves: HashMap<TreeNodeId, TreeNode>,
    leaves_in_use: HashMap<PendingLeavesId, Vec<TreeNode>>,
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
            leaves_in_use: HashMap::new(),
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

    pub fn mark_leaves_as_pending(&mut self, leaves: &[TreeNode]) -> PendingLeavesId {
        let pending_leaves_id = Uuid::now_v7().to_string();
        self.leaves_in_use
            .insert(pending_leaves_id.clone(), leaves.to_vec());
        for leaf in leaves {
            self.leaves.remove(&leaf.id);
        }
        pending_leaves_id
    }

    pub fn cancel_pending_leaves(&mut self, pending_leaves_id: PendingLeavesId) {
        if let Some(leaves) = self.leaves_in_use.remove(&pending_leaves_id) {
            for leaf in leaves {
                self.leaves.insert(leaf.id.clone(), leaf.clone());
            }
        }
    }

    pub fn finalize_pending_leaves(&mut self, pending_leaves_id: PendingLeavesId) {
        self.leaves_in_use.remove(&pending_leaves_id);
    }
}
