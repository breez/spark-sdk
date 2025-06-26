use crate::{services::TransferService, tree::TreeNodeStatus};

use super::{TreeNode, error::TreeServiceError, state::TreeState};

pub struct TreeService<S> {
    state: TreeState,
    transfer_service: TransferService<S>,
}

impl<S> TreeService<S> {
    pub fn new(state: TreeState, transfer_service: TransferService<S>) -> Self {
        TreeService {
            state,
            transfer_service,
        }
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

    pub async fn collect_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if nodes.is_empty() {
            return Ok(Vec::new());
        }

        let mut resulting_nodes = Vec::new();
        for node in nodes.into_iter() {
            if node.status != TreeNodeStatus::Available {
                // TODO: Handle other statuses appropriately.
                resulting_nodes.push(node);
                continue;
            }

            let nodes = self.transfer_service.extend_time_lock(&node).await?;

            for n in nodes {
                if n.status != TreeNodeStatus::Available {
                    // TODO: Handle other statuses appropriately.
                    resulting_nodes.push(n);
                    continue;
                }

                let transfer = self
                    .transfer_service
                    .transfer_leaves_to_self(vec![n])
                    .await?;
                resulting_nodes.extend(transfer.into_iter());
            }
        }

        // TODO: add/remove nodes to/from the tree state as needed.
        Ok(resulting_nodes)
    }
}
