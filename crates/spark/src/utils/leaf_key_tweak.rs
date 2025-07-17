use crate::{
    services::LeafKeyTweak,
    signer::{PrivateKeySource, Signer, SignerError},
    tree::TreeNode,
};

pub fn prepare_leaf_key_tweaks_to_send<S: Signer>(
    signer: &S,
    leaves: Vec<TreeNode>,
) -> Result<Vec<LeafKeyTweak>, SignerError> {
    // Build leaf key tweaks with new signing keys that we will sent to the receiver
    leaves
        .iter()
        .map(|leaf| {
            let our_key = PrivateKeySource::Derived(leaf.id.clone());
            let ephemeral_key = signer.generate_random_key()?;

            Ok(LeafKeyTweak {
                node: leaf.clone(),
                signing_key: our_key,
                new_signing_key: ephemeral_key,
            })
        })
        .collect::<Result<Vec<_>, SignerError>>()
}
