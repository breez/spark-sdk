use crate::{services::LeafKeyTweak, signer::LeafSigningKey, tree::TreeNode};

/// Pairs each leaf with the key derived from its own node id, the key a claim
/// leaves it under.
pub fn with_node_id_keys(leaves: Vec<TreeNode>) -> Vec<LeafKeyTweak> {
    leaves
        .into_iter()
        .map(|leaf| LeafKeyTweak {
            signing_key: LeafSigningKey {
                derived_from: leaf.id.clone(),
            },
            node: leaf,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use macros::test_all;

    use super::with_node_id_keys;
    use crate::tree::tests::create_test_tree_node;

    #[test_all]
    fn each_leaf_is_held_under_the_key_derived_from_its_node_id() {
        let leaves = vec![
            create_test_tree_node("leaf-a", 1_000),
            create_test_tree_node("leaf-b", 2_000),
        ];

        let tweaks = with_node_id_keys(leaves.clone());

        assert_eq!(tweaks.len(), 2);
        for (tweak, leaf) in tweaks.iter().zip(&leaves) {
            assert_eq!(tweak.node, *leaf, "the leaves keep their order");
            assert_eq!(tweak.signing_key.derived_from, leaf.id);
        }
    }
}
