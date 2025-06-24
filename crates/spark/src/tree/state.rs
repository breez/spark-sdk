use std::sync::Mutex;

use super::TreeNode;

pub struct TreeState {
    pub leaves: Mutex<Vec<TreeNode>>,
}

impl TreeState {
    pub fn new(leaves: Vec<TreeNode>) -> Self {
        TreeState {
            leaves: Mutex::new(leaves),
        }
    }

    pub fn select_leaves(&self, target_amount: u64) -> Option<Vec<TreeNode>> {
        let mut amount = 0;
        let mut nodes = vec![];
        let mut leaves = self.leaves.lock().unwrap().clone();
        leaves.sort_by_key(|leaf| leaf.value);

        for leaf in leaves {
            if target_amount - amount >= leaf.value {
                amount += leaf.value;
                nodes.push(leaf);
            }
        }
        if amount < target_amount {
            return None;
        }
        Some(nodes)
    }
}
