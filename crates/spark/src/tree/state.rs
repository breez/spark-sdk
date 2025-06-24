use std::sync::Mutex;

use super::{TreeNode, error::TreeServiceError};

pub struct TreeState {
    pub leaves: Mutex<Vec<TreeNode>>,
}

impl TreeState {
    pub fn new(leaves: Vec<TreeNode>) -> Self {
        TreeState {
            leaves: Mutex::new(leaves),
        }
    }

    pub fn select_leaves(&self, target_amount: u64) -> Result<Vec<TreeNode>, TreeServiceError> {
        if target_amount == 0 {
            return Err(TreeServiceError::IllegalAmount);
        }

        let mut amount = 0;
        let mut nodes = vec![];
        let mut leaves = self.leaves.lock().unwrap().clone();
        leaves.sort_by(|a, b| b.value.cmp(&a.value));

        let mut aggregated_amount: u32 = 0;
        for leaf in leaves {
            aggregated_amount += leaf.value;
            if target_amount.saturating_sub(amount) >= leaf.value {
                amount += leaf.value;
                nodes.push(leaf);
            }
        }
        if amount < target_amount {
            match aggregated_amount > target_amount {
                true => return Err(TreeServiceError::UnselectableAmount),
                false => return Err(TreeServiceError::InsufficientFunds),
            }
        }
        Ok(nodes)
    }
}
