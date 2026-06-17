use crate::{services::LeafKeyTweak, tree::TreeNode};

/// Builds the leaf key tweaks to send for a transfer.
///
/// Records each leaf's node; its current signing key is derived from the node
/// id. The *new* signing key for each leaf is generated inside
/// [`SparkSigner::prepare_transfer`](crate::signer::SparkSigner::prepare_transfer),
/// so it is not produced here.
pub fn prepare_leaf_key_tweaks_to_send(leaves: Vec<TreeNode>) -> Vec<LeafKeyTweak> {
    leaves
        .into_iter()
        .map(|leaf| LeafKeyTweak {
            node: leaf,
            incoming_key: None,
        })
        .collect()
}
