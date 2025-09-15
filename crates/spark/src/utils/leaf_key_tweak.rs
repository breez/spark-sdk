use std::sync::Arc;

use crate::{
    services::LeafKeyTweak,
    signer::{PrivateKeySource, Signer, SignerError},
    tree::TreeNode,
};

pub fn prepare_leaf_key_tweaks_to_send(
    signer: &Arc<dyn Signer>,
    leaves: Vec<TreeNode>,
    signing_key_source: Option<PrivateKeySource>,
) -> Result<Vec<LeafKeyTweak>, SignerError> {
    // Build leaf key tweaks with new signing keys that we will sent to the receiver
    leaves
        .iter()
        .map(|leaf| {
            let our_key = signing_key_source
                .clone()
                .unwrap_or(PrivateKeySource::Derived(leaf.id.clone()));
            let ephemeral_key = signer.generate_random_key()?;

            Ok(LeafKeyTweak {
                node: leaf.clone(),
                signing_key: our_key,
                new_signing_key: ephemeral_key,
            })
        })
        .collect::<Result<Vec<_>, SignerError>>()
}
