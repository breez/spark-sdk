use super::{TreeNode, error::TreeServiceError, state::TreeState};

pub struct TreeService {
    state: TreeState,
}

impl TreeService {
    pub fn new(state: TreeState) -> Self {
        TreeService { state }
    }

    pub async fn select_leaves(
        &self,
        target_amount: u64,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if target_amount == 0 {
            return Err(TreeServiceError::IllegalAmount);
        }

        let mut amount = 0;
        let mut nodes = vec![];
        let mut leaves = self.state.get_leaves().await;
        leaves.sort_by(|a, b| b.value.cmp(&a.value));

        let mut aggregated_amount: u64 = 0;
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
