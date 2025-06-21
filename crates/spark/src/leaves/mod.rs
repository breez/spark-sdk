use tokio::sync::Mutex;

use crate::tree::TreeNode;

// TODO: Implement proper leafmanager logic.
pub struct LeafManager {
    leaves: Mutex<Vec<TreeNode>>,
}

impl Default for LeafManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LeafManager {
    pub fn new() -> Self {
        LeafManager {
            leaves: Mutex::new(Vec::new()),
        }
    }

    pub async fn add_leaf(&self, leaf: TreeNode) {
        let mut leaves = self.leaves.lock().await;
        leaves.push(leaf);
    }

    pub async fn add_leaves(&self, leaves: &[TreeNode]) {
        let mut leafmap = self.leaves.lock().await;
        leafmap.extend_from_slice(leaves);
    }

    pub async fn get_leaves(&self) -> Vec<TreeNode> {
        let leaves = self.leaves.lock().await;
        leaves.clone()
    }

    pub async fn clear_leaves(&self) {
        let mut leaves = self.leaves.lock().await;
        leaves.clear();
    }

    pub async fn remove_leaf(&self, leaf: &TreeNode) -> bool {
        let mut leaves = self.leaves.lock().await;
        if let Some(pos) = leaves.iter().position(|x| x.id == leaf.id) {
            leaves.remove(pos);
            true
        } else {
            false
        }
    }

    pub async fn remove_leaves(&self, leaves: &[TreeNode]) {
        for leaf in leaves {
            self.remove_leaf(leaf).await;
        }
    }
}
