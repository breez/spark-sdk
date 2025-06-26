use crate::{services::TransferService, signer::Signer, tree::TreeNodeStatus};

use super::{TreeNode, error::TreeServiceError, state::TreeState};

pub struct TreeService<S: Signer> {
    state: TreeState,
    transfer_service: TransferService<S>,
}

impl<S: Signer> TreeService<S> {
    pub fn new(state: TreeState, transfer_service: TransferService<S>) -> Self {
        TreeService {
            state,
            transfer_service,
        }
    }

    /// Refreshes the tree state by fetching the latest tree from the coordinator/operators?
    pub async fn refresh_leaves(&self) -> Result<(), TreeServiceError> {
        todo!()
    }

    /// Selects leaves from the tree that sum up to the target amount.
    /// If necessary, performs swap to get set of leaves matching target amount.
    pub async fn select_leaves_by_amount(
        &self,
        target_amount_sat: u64,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if target_amount_sat == 0 {
            return Err(TreeServiceError::IllegalAmount);
        }

        let mut amount = 0;
        let mut nodes = vec![];
        let mut leaves = self.state.get_leaves().await;
        leaves.sort_by(|a, b| b.value.cmp(&a.value));

        let mut aggregated_amount: u64 = 0;
        for leaf in leaves {
            aggregated_amount += leaf.value;
            if target_amount_sat.saturating_sub(amount) >= leaf.value {
                amount += leaf.value;
                nodes.push(leaf);
            }
        }
        if amount < target_amount_sat {
            match aggregated_amount > target_amount_sat {
                true => return Err(TreeServiceError::UnselectableAmount),
                false => return Err(TreeServiceError::InsufficientFunds),
            }
        }
        // TODO: if necessary, perform swap to get set of leaves matching target amount

        Ok(nodes)
    }

    pub async fn collect_leaves(
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
