use crate::tree::TreeNode;

// TODO: Implement proper tree state logic.
pub struct TreeState {
    leaves: Vec<TreeNode>,
}

impl Default for TreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeState {
    pub fn new() -> Self {
        TreeState { leaves: Vec::new() }
    }

    pub fn add_leaf(&mut self, leaf: TreeNode) {
        self.leaves.push(leaf);
    }

    pub fn add_leaves(&mut self, leaves: &[TreeNode]) {
        self.leaves.extend_from_slice(leaves);
    }

    pub fn get_leaves(&self) -> Vec<TreeNode> {
        self.leaves.clone()
    }

    pub fn clear_leaves(&mut self) {
        self.leaves.clear();
    }

    pub fn remove_leaf(&mut self, leaf: &TreeNode) -> bool {
        if let Some(pos) = self.leaves.iter().position(|x| x.id == leaf.id) {
            self.leaves.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn remove_leaves(&mut self, leaves: &[TreeNode]) {
        for leaf in leaves {
            self.remove_leaf(leaf);
        }
    }
}
