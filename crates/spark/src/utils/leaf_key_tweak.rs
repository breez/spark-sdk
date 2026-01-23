use std::sync::Arc;

use tracing::{debug, trace};
use web_time::Instant;

use crate::{
    services::LeafKeyTweak,
    signer::{SecretSource, Signer, SignerError},
    tree::TreeNode,
};

pub async fn prepare_leaf_key_tweaks_to_send(
    signer: &Arc<dyn Signer>,
    leaves: Vec<TreeNode>,
    signing_key_source: Option<SecretSource>,
) -> Result<Vec<LeafKeyTweak>, SignerError> {
    let start = Instant::now();
    let leaf_count = leaves.len();
    debug!(
        "prepare_leaf_key_tweaks_to_send starting | leaf_count={}",
        leaf_count
    );

    // Build leaf key tweaks with new signing keys that we will sent to the receiver
    let mut tweaks = Vec::with_capacity(leaves.len());

    for (i, leaf) in leaves.into_iter().enumerate() {
        trace!(
            "prepare_leaf_key_tweaks_to_send | generating ephemeral key for leaf {}/{}",
            i + 1,
            leaf_count
        );
        let our_key = signing_key_source
            .clone()
            .unwrap_or(SecretSource::Derived(leaf.id.clone()));
        let ephemeral_key = signer.generate_random_secret().await?;

        tweaks.push(LeafKeyTweak {
            node: leaf.clone(),
            signing_key: our_key,
            new_signing_key: SecretSource::Encrypted(ephemeral_key),
        });
    }

    debug!(
        "prepare_leaf_key_tweaks_to_send completed | leaf_count={} elapsed_ms={}",
        leaf_count,
        start.elapsed().as_millis()
    );
    Ok(tweaks)
}
