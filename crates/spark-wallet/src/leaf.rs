use serde::{Deserialize, Serialize};
use spark::tree::TreeNode;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletLeaf {}

impl From<TreeNode> for WalletLeaf {
    fn from(node: TreeNode) -> Self {
        todo!()
    }
}
