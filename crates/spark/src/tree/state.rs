use tokio::sync::Mutex;

use super::TreeNode;

pub struct TreeState {
    leaves: Mutex<Vec<TreeNode>>,
}

impl TreeState {
    pub fn new(leaves: Vec<TreeNode>) -> Self {
        TreeState {
            leaves: Mutex::new(leaves),
        }
    }

    pub async fn get_leaves(&self) -> Vec<TreeNode> {
        self.leaves.lock().await.clone()
    }
}
