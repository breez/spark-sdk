//! Keeps the pool's stored leaves up to date with the coordinator.
//!
//! The operators can change a leaf after it has joined the pool. For example, they
//! mark it as `PARENT_EXITED` once a transaction above it confirms. Leaf selection
//! goes by the status in the store, so the store has to pick up such changes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bitcoin::secp256k1::PublicKey;
use spark::tree::{Leaves, TreeNode, TreeNodeId, TreeNodeStatus, TreeService, TreeStore};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::tree_pooling::MAX_NODE_IDS_PER_QUERY;
use crate::wakeup::Wakeup;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The longest time between two syncs. A new block triggers one right away.
const SYNC_INTERVAL: Duration = Duration::from_secs(60);

pub struct LeafSyncDeps {
    pub tree_service: Arc<dyn TreeService>,
    pub tree_store: Arc<dyn TreeStore>,
    pub identity: PublicKey,
    pub blocks: Wakeup,
}

pub async fn run_leaf_sync_loop(deps: LeafSyncDeps, token: CancellationToken) {
    loop {
        if let Err(e) = sync_leaves(&deps).await {
            error!("could not sync the pool's leaves with the coordinator: {e}");
        }
        tokio::select! {
            () = token.cancelled() => return,
            () = deps.blocks.waited() => {}
            () = tokio::time::sleep(SYNC_INTERVAL) => {}
        }
    }
}

/// Reserved leaves are left alone: the operation that reserved them is still
/// working with them.
async fn sync_leaves(deps: &LeafSyncDeps) -> Result<(), BoxError> {
    // Take the store's time first. `set_leaves` uses it to see what happened while
    // the sync was running: it does not write back a leaf that was spent after this
    // time, and it keeps a leaf that was added after it.
    let started_at = deps.tree_store.now().await?;
    let asked: Vec<TreeNodeId> = deps
        .tree_store
        .get_verified_leaf_keys()
        .await?
        .into_keys()
        .collect();
    let mut reported = Vec::with_capacity(asked.len());
    for chunk in asked.chunks(MAX_NODE_IDS_PER_QUERY) {
        reported.extend(deps.tree_service.fetch_nodes(chunk, false).await?);
    }

    // Read the leaves only now, after the coordinator has answered. A leaf that
    // was sent in the meantime is already gone from the store, so it is not
    // mistaken for one the coordinator stopped reporting.
    let leaves = deps.tree_store.get_leaves().await?;
    let asked = asked.iter().collect();
    let to_store = LeavesToStore::new(&leaves, &asked, reported, &deps.identity);
    if !to_store.changed {
        return Ok(());
    }

    let stored: HashMap<&TreeNodeId, TreeNodeStatus> = unreserved(&leaves)
        .map(|leaf| (&leaf.id, leaf.status))
        .collect();
    for leaf in &to_store.ours {
        if stored.get(&leaf.id) != Some(&leaf.status) {
            info!(leaf_id = %leaf.id, status = %leaf.status, "a pool leaf's status changed");
        }
    }
    for leaf in &to_store.unreported {
        warn!(leaf_id = %leaf.id, "the coordinator does not report a pool leaf as the SSP's, so it is kept out of selection");
    }
    info!(
        leaves = to_store.ours.len(),
        unreported = to_store.unreported.len(),
        "storing the pool's unreserved leaves as the coordinator reports them"
    );
    deps.tree_store
        .set_leaves(&to_store.ours, &to_store.unreported, started_at)
        .await?;
    Ok(())
}

fn unreserved(leaves: &Leaves) -> impl Iterator<Item = &TreeNode> {
    leaves
        .available
        .iter()
        .chain(&leaves.not_available)
        .chain(&leaves.available_missing_from_operators)
}

/// The leaves a sync writes to the store: all unreserved leaves, split into the
/// two lists that `set_leaves` takes.
#[derive(Debug, Default)]
struct LeavesToStore {
    /// Leaves stored as the SSP's. For a leaf the coordinator reported, this is
    /// the coordinator's copy.
    ours: Vec<TreeNode>,
    /// Leaves the coordinator does not report as the SSP's. They stay in the store,
    /// because that alone does not prove a leaf was spent, but the store flags
    /// them so that they are not selected.
    unreported: Vec<TreeNode>,
    /// Whether writing these leaves would change the store.
    changed: bool,
}

impl LeavesToStore {
    /// Compares the stored `leaves` with what the coordinator `reported`. A leaf
    /// that is not in `asked` was added after the coordinator was asked, so it is
    /// kept as it is.
    fn new(
        leaves: &Leaves,
        asked: &HashSet<&TreeNodeId>,
        reported: Vec<TreeNode>,
        identity: &PublicKey,
    ) -> Self {
        let mut reported: HashMap<TreeNodeId, TreeNode> = reported
            .into_iter()
            .filter(|node| node.owner_identity_public_key == Some(*identity))
            .map(|node| (node.id.clone(), node))
            .collect();
        let reported_before = leaves.available.iter().chain(&leaves.not_available);
        let unreported_before = leaves.available_missing_from_operators.iter();
        let mut to_store = Self::default();
        for (stored, was_unreported) in reported_before
            .map(|leaf| (leaf, false))
            .chain(unreported_before.map(|leaf| (leaf, true)))
        {
            if let Some(current) = reported.remove(&stored.id) {
                to_store.changed |= was_unreported || differs(stored, &current);
                to_store.ours.push(current);
            } else if was_unreported || asked.contains(&stored.id) {
                // A write is only needed if the leaf could otherwise still be selected.
                to_store.changed |= !was_unreported && stored.status == TreeNodeStatus::Available;
                to_store.unreported.push(stored.clone());
            } else {
                to_store.ours.push(stored.clone());
            }
        }
        to_store
    }
}

/// Compares the fields that matter instead of the whole leaf, because the
/// operators return a keyshare's owners in no particular order.
fn differs(stored: &TreeNode, current: &TreeNode) -> bool {
    stored.status != current.status
        || stored.signing_keyshare.public_key != current.signing_keyshare.public_key
        || stored.node_tx != current.node_tx
        || stored.refund_tx != current.refund_tx
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use bitcoin::{Transaction, absolute::LockTime, transaction::Version};
    use spark::tree::SigningKeyshare;

    use super::*;

    fn key(byte: u8) -> PublicKey {
        SecretKey::from_slice(&[byte; 32])
            .unwrap()
            .public_key(&Secp256k1::new())
    }

    fn node(id: &str, owner: PublicKey, status: TreeNodeStatus) -> TreeNode {
        TreeNode {
            id: id.parse().unwrap(),
            tree_id: String::new(),
            value: 1_000,
            parent_node_id: None,
            node_tx: Transaction {
                version: Version::non_standard(3),
                lock_time: LockTime::ZERO,
                input: vec![],
                output: vec![],
            },
            refund_tx: None,
            direct_tx: None,
            direct_refund_tx: None,
            direct_from_cpfp_refund_tx: None,
            vout: 0,
            verifying_public_key: owner,
            owner_identity_public_key: Some(owner),
            signing_keyshare: SigningKeyshare {
                owner_identifiers: vec![],
                threshold: 2,
                public_key: owner,
            },
            status,
        }
    }

    fn leaves(
        available: Vec<TreeNode>,
        not_available: Vec<TreeNode>,
        missing: Vec<TreeNode>,
    ) -> Leaves {
        Leaves {
            available,
            not_available,
            available_missing_from_operators: missing,
            reserved_for_payment: vec![],
            reserved_for_swap: vec![],
        }
    }

    fn asked(leaves: &Leaves) -> HashSet<&TreeNodeId> {
        unreserved(leaves).map(|leaf| &leaf.id).collect()
    }

    fn ids(nodes: &[TreeNode]) -> Vec<String> {
        nodes.iter().map(|node| node.id.to_string()).collect()
    }

    #[test]
    fn leaves_get_the_coordinators_copy_and_unreported_leaves_are_kept() {
        let (ssp, user) = (key(1), key(2));
        let stored = leaves(
            vec![
                node("exiting", ssp, TreeNodeStatus::Available),
                node("unchanged", ssp, TreeNodeStatus::Available),
                node("sent", ssp, TreeNodeStatus::Available),
            ],
            vec![node("exited", ssp, TreeNodeStatus::ParentExited)],
            vec![node("back", ssp, TreeNodeStatus::Available)],
        );
        let reported = vec![
            node("exiting", ssp, TreeNodeStatus::ParentExited),
            node("unchanged", ssp, TreeNodeStatus::Available),
            node("sent", user, TreeNodeStatus::TransferLocked),
            node("back", ssp, TreeNodeStatus::Available),
            node("not-held", ssp, TreeNodeStatus::Available),
        ];

        let to_store = LeavesToStore::new(&stored, &asked(&stored), reported, &ssp);

        assert!(to_store.changed);
        assert_eq!(ids(&to_store.ours), ["exiting", "unchanged", "back"]);
        assert_eq!(to_store.ours[0].status, TreeNodeStatus::ParentExited);
        assert_eq!(ids(&to_store.unreported), ["sent", "exited"]);
    }

    #[test]
    fn nothing_changes_when_the_store_already_matches() {
        let ssp = key(1);
        let stored = leaves(
            vec![node("available", ssp, TreeNodeStatus::Available)],
            vec![
                node("exiting", ssp, TreeNodeStatus::ParentExited),
                node("unreported", ssp, TreeNodeStatus::TransferLocked),
            ],
            vec![node("missing", ssp, TreeNodeStatus::Available)],
        );
        let reported = vec![
            node("available", ssp, TreeNodeStatus::Available),
            node("exiting", ssp, TreeNodeStatus::ParentExited),
        ];

        assert!(!LeavesToStore::new(&stored, &asked(&stored), reported, &ssp).changed);
    }

    #[test]
    fn the_order_of_keyshare_owners_is_not_a_change() {
        let ssp = key(1);
        let owners = |ids: [u16; 2]| ids.map(|id| id.try_into().unwrap()).to_vec();
        let mut held = node("leaf", ssp, TreeNodeStatus::Available);
        held.signing_keyshare.owner_identifiers = owners([1, 2]);
        let mut current = held.clone();
        current.signing_keyshare.owner_identifiers = owners([2, 1]);

        let stored = leaves(vec![held], vec![], vec![]);
        assert!(!LeavesToStore::new(&stored, &asked(&stored), vec![current], &ssp).changed);
    }

    #[test]
    fn a_leaf_added_after_the_coordinator_was_asked_is_kept_as_is() {
        let ssp = key(1);
        let stored = leaves(
            vec![node("joined", ssp, TreeNodeStatus::Available)],
            vec![],
            vec![],
        );

        let to_store = LeavesToStore::new(&stored, &HashSet::new(), vec![], &ssp);

        assert!(!to_store.changed);
        assert_eq!(ids(&to_store.ours), ["joined"]);
        assert!(to_store.unreported.is_empty());
    }

    #[test]
    fn a_leaf_that_comes_back_or_goes_missing_is_a_change() {
        let ssp = key(1);
        let back = leaves(
            vec![],
            vec![],
            vec![node("back", ssp, TreeNodeStatus::Available)],
        );
        let reported = vec![node("back", ssp, TreeNodeStatus::Available)];
        assert!(LeavesToStore::new(&back, &asked(&back), reported, &ssp).changed);

        let gone = leaves(
            vec![node("gone", ssp, TreeNodeStatus::Available)],
            vec![],
            vec![],
        );
        assert!(LeavesToStore::new(&gone, &asked(&gone), vec![], &ssp).changed);
    }
}
