use bitcoin::Transaction;
use serde::{Deserialize, Serialize};
use spark::tree::{TreeNode, TreeNodeId, TreeNodeStatus};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletLeaf {
    pub id: TreeNodeId,
    pub value: u64,
    pub status: TreeNodeStatus,
    pub refund_tx: Option<Transaction>,
}

impl From<TreeNode> for WalletLeaf {
    fn from(node: TreeNode) -> Self {
        Self {
            id: node.id,
            value: node.value,
            status: node.status,
            refund_tx: node.refund_tx,
        }
    }
}
