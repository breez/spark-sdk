use super::{TreeNode, error::TreeServiceError, state::TreeState};

pub struct TreeService {
    state: TreeState,
}

impl TreeService {
    pub fn new(state: TreeState) -> Self {
        TreeService { state }
    }

    pub fn select_leaves(&self, amount: u64) -> Result<Vec<TreeNode>, TreeServiceError> {
        let selected_leaves = self.state.select_leaves(amount);
        match selected_leaves {
            Ok(leaves) => Ok(leaves),
            Err(e) => {
                //TODO We should as the ssp to swap at this point
                Err(e)
            }
        }
    }
}
