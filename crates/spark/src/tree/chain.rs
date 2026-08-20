//! What a leaf's exit chain is and when one is good enough to keep.
//!
//! A chain is the path of nodes from a leaf up to a root, the pre-signed
//! transactions an exit broadcasts when the operators are unreachable. Every
//! backend stores and reports on chains, so the rules live here rather than in
//! any one of them.

use std::collections::{HashMap, HashSet};

use crate::tree::{LeafPedigree, TreeNode, TreeNodeId};

/// A leaf's ancestors, child first, walking `parent_node_id` through `nodes` and
/// stopping at the root, a gap, or a cycle in the semi-trusted parent ids.
fn walk_ancestors(leaf: &TreeNode, nodes: &HashMap<TreeNodeId, TreeNode>) -> Vec<TreeNode> {
    let mut ancestors = Vec::new();
    let mut visited: HashSet<TreeNodeId> = HashSet::new();
    let mut current = leaf.parent_node_id.clone();
    while let Some(id) = current {
        if !visited.insert(id.clone()) {
            break;
        }
        let Some(node) = nodes.get(&id) else { break };
        current = node.parent_node_id.clone();
        ancestors.push(node.clone());
    }
    ancestors
}

/// Whether `ancestors` is a chain that can back `leaf`'s unilateral exit. A leaf
/// that is itself a root needs no ancestors. The rule in Rust, which a backend
/// answering [`TreeStore::leaves_missing_exit_chains`] restates in its own query
/// language.
///
/// Looks no further than the leaf's current parent, which is enough because
/// [`chain_reaches_root`] gates what gets stored. Renewal reparents a leaf onto a
/// new split node, so the parent it holds now is what a stale chain misses.
///
/// [`TreeStore::leaves_missing_exit_chains`]: crate::tree::TreeStore::leaves_missing_exit_chains
pub fn chain_backs_exit(leaf: &TreeNode, ancestors: &[TreeNode]) -> bool {
    let Some(parent_id) = leaf.parent_node_id.as_ref() else {
        return true;
    };
    ancestors.iter().any(|node| &node.id == parent_id)
}

/// Whether `ancestors` spans `leaf`'s whole path, from its current parent up to a
/// node with no parent of its own. A leaf that is itself a root needs none.
///
/// The test a chain passes before it is stored: one that stops short builds an
/// exit whose transactions cannot confirm.
pub fn chain_reaches_root(leaf: &TreeNode, ancestors: &[TreeNode]) -> bool {
    let Some(parent_id) = leaf.parent_node_id.as_ref() else {
        return true;
    };
    let by_id: HashMap<&TreeNodeId, &TreeNode> =
        ancestors.iter().map(|node| (&node.id, node)).collect();
    let mut visited: HashSet<&TreeNodeId> = HashSet::new();
    let mut current = Some(parent_id);
    while let Some(id) = current {
        let Some(node) = by_id.get(id) else {
            return false;
        };
        if !visited.insert(id) {
            return false;
        }
        current = node.parent_node_id.as_ref();
    }
    true
}

/// Shapes a batch of exit chains from a node lookup. A store loads its leaves and
/// their ancestors into `nodes` with a single query, then calls this to pair each
/// requested leaf with its chain. A leaf id absent from `nodes` is skipped, and a
/// chain that hits a gap comes back with what it reached; [`chain_backs_exit`] is
/// what decides whether that is enough.
pub fn assemble_exit_chains(
    nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
) -> Vec<LeafPedigree> {
    leaf_ids
        .iter()
        .filter_map(|id| {
            let leaf = nodes.get(id)?.clone();
            let ancestors = walk_ancestors(&leaf, nodes);
            Some(LeafPedigree { leaf, ancestors })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use macros::test_all;

    use super::*;
    use crate::tree::TreeNodeStatus;
    use crate::tree::tests::create_test_node_with_parent;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_chain_reaches_root() {
        let node = |id, parent| create_test_node_with_parent(id, parent, TreeNodeStatus::Available);

        let leaf = node("leaf", Some("mid"));
        let mid = node("mid", Some("root"));
        let root = node("root", None);

        assert!(chain_reaches_root(&leaf, &[mid.clone(), root.clone()]));
        // What a zero-timelock renewal rebuilds when it resolved no ancestors:
        // the new parent alone, stopping short of a root.
        assert!(!chain_reaches_root(&leaf, std::slice::from_ref(&mid)));
        // The root is there but the link to it is not.
        assert!(!chain_reaches_root(&leaf, &[root]));
        assert!(!chain_reaches_root(&leaf, &[]));
        // A leaf that is itself a root needs none.
        assert!(chain_reaches_root(&node("solo", None), &[]));
        // A cycle terminates rather than spinning.
        assert!(!chain_reaches_root(
            &leaf,
            &[mid, node("root", Some("mid"))]
        ));
    }
}
