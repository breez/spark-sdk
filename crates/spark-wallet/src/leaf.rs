use serde::{Deserialize, Serialize};
use spark::tree::{TreeNode, TreeNodeId};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletLeaf {
    pub id: TreeNodeId,
    pub value: u64,
}

impl From<TreeNode> for WalletLeaf {
    fn from(node: TreeNode) -> Self {
        Self {
            id: node.id,
            value: node.value,
        }
    }
}
