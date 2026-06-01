use crate::{services::LeafKeyTweak, signer::SecretSource, tree::TreeNode};

/// Builds the leaf key tweaks to send for a transfer.
///
/// Records each leaf's node and its current signing key. The *new* signing key
/// for each leaf is generated inside
/// [`SparkSigner::prepare_transfer`](crate::signer::SparkSigner::prepare_transfer),
/// so it is not produced here.
pub fn prepare_leaf_key_tweaks_to_send(
    leaves: Vec<TreeNode>,
    signing_key_source: Option<SecretSource>,
) -> Vec<LeafKeyTweak> {
    leaves
        .into_iter()
        .map(|leaf| {
            let signing_key = signing_key_source
                .clone()
                .unwrap_or_else(|| SecretSource::Derived(leaf.id.clone()));
            LeafKeyTweak {
                node: leaf,
                signing_key,
            }
        })
        .collect()
}
