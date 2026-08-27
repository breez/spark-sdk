//! Shared test suite for `TreeStore` implementations.
//!
//! Each function tests a specific behavior against any `TreeStore` impl.
//! To use, call these functions from implementation-specific test modules
//! passing a concrete store instance.

use std::str::FromStr;
use std::time::Duration;

use bitcoin::{Transaction, absolute::LockTime, secp256k1::PublicKey, transaction::Version};
use frost_secp256k1_tr::Identifier;
use platform_utils::time::SystemTime;

use crate::tree::{
    LeafPedigree, LeafSelection, Leaves, LeavesReservation, ReservationPurpose, ReserveResult,
    TargetAmounts, TreeNode, TreeNodeId, TreeNodeStatus, TreeServiceError, TreeStore,
};

/// Creates a test `TreeNode` with the given ID and value.
pub fn create_test_tree_node(id: &str, value: u64) -> TreeNode {
    TreeNode {
        id: TreeNodeId::from_str(id).unwrap(),
        tree_id: "test_tree".to_string(),
        value,
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
        verifying_public_key: PublicKey::from_str(
            "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
        )
        .unwrap(),
        owner_identity_public_key: Some(
            PublicKey::from_str(
                "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
            )
            .unwrap(),
        ),
        signing_keyshare: crate::tree::SigningKeyshare {
            public_key: PublicKey::from_str(
                "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
            )
            .unwrap(),
            owner_identifiers: vec![Identifier::try_from(1u16).unwrap()],
            threshold: 2,
        },
        status: crate::tree::TreeNodeStatus::Available,
    }
}

/// Creates a test `TreeNode` with a parent link and status, for exit-chain tests.
pub fn create_test_node_with_parent(
    id: &str,
    parent: Option<&str>,
    status: TreeNodeStatus,
) -> TreeNode {
    let mut node = create_test_tree_node(id, 1_000);
    node.parent_node_id = parent.map(|p| TreeNodeId::from_str(p).unwrap());
    node.status = status;
    node
}

/// Puts each pedigree's leaf in the pool and then writes its chain: the two
/// calls a caller makes now that `store_ancestors` is the only ancestor writer.
async fn add_with_chains(store: &dyn TreeStore, pedigrees: &[LeafPedigree]) {
    let leaves: Vec<TreeNode> = pedigrees.iter().map(|p| p.leaf.clone()).collect();
    store.add_leaves(&leaves).await.unwrap();
    store.store_ancestors(pedigrees).await.unwrap();
}

/// Root-to-leaf ids of a stored leaf's exit chain; empty when the leaf is not
/// stored.
async fn exit_chain_ids(store: &dyn TreeStore, leaf_id: &TreeNodeId) -> Vec<String> {
    let Some(pedigree) = exit_chain(store, leaf_id).await else {
        return Vec::new();
    };
    let mut ids: Vec<String> = pedigree
        .ancestors
        .iter()
        .rev()
        .map(|a| a.id.to_string())
        .collect();
    ids.push(pedigree.leaf.id.to_string());
    ids
}

/// A single leaf's pedigree, or `None` when the leaf is not stored.
async fn exit_chain(store: &dyn TreeStore, leaf_id: &TreeNodeId) -> Option<LeafPedigree> {
    store
        .get_exit_chains(std::slice::from_ref(leaf_id))
        .await
        .unwrap()
        .into_iter()
        .next()
}

/// Storing a leaf round-trips the whole node; an id that is not stored is not
/// returned.
pub async fn test_upsert_and_get_leaf(store: &dyn TreeStore) {
    let node = create_test_tree_node("node-a", 1_000);
    store.add_leaves(std::slice::from_ref(&node)).await.unwrap();

    let missing = TreeNodeId::from_str("no-such-node").unwrap();
    let pedigrees = store
        .get_exit_chains(&[node.id.clone(), missing])
        .await
        .unwrap();
    assert_eq!(pedigrees.len(), 1);
    assert_eq!(pedigrees[0].leaf, node);
    assert!(pedigrees[0].ancestors.is_empty());
}

/// A missing ancestor no longer errors: the store returns as much of the chain
/// as it has (here `mid`, `leaf`), and the caller detects incompleteness because
/// the returned root still references an unstored parent.
pub async fn test_get_exit_chain_missing_ancestor(store: &dyn TreeStore) {
    let mid = create_test_node_with_parent("mid", Some("root"), TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);
    // The root is intentionally left out of the ancestors.
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![mid],
        }],
    )
    .await;

    let pedigree = exit_chain(store, &leaf.id).await.unwrap();
    let ids: Vec<String> = pedigree
        .ancestors
        .iter()
        .map(|a| a.id.to_string())
        .collect();
    assert_eq!(ids, vec!["mid"]);
    assert!(pedigree.ancestors[0].parent_node_id.is_some());
}

/// The batch `get_exit_chains` returns one pedigree per stored leaf in request
/// order (leaf plus nearest-first ancestors), skips a leaf that is not stored, and
/// returns a partial chain when an ancestor is missing.
pub async fn test_get_exit_chains(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);
    let mid_a = create_test_node_with_parent("mid-a", Some("root"), TreeNodeStatus::Splitted);
    let mid_b = create_test_node_with_parent("mid-b", Some("root"), TreeNodeStatus::Splitted);
    let leaf_a = create_test_node_with_parent("leaf-a", Some("mid-a"), TreeNodeStatus::Available);
    let leaf_b = create_test_node_with_parent("leaf-b", Some("mid-b"), TreeNodeStatus::Available);
    // leaf-c's parent is not stored, so its chain comes back empty (partial).
    let leaf_c = create_test_node_with_parent("leaf-c", Some("gone"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[
            LeafPedigree {
                leaf: leaf_a.clone(),
                ancestors: vec![mid_a, root.clone()],
            },
            LeafPedigree {
                leaf: leaf_b.clone(),
                ancestors: vec![mid_b, root],
            },
            LeafPedigree {
                leaf: leaf_c.clone(),
                ancestors: vec![],
            },
        ],
    )
    .await;

    let missing = TreeNodeId::from_str("not-stored").unwrap();
    let pedigrees = store
        .get_exit_chains(&[
            leaf_a.id.clone(),
            leaf_b.id.clone(),
            leaf_c.id.clone(),
            missing,
        ])
        .await
        .unwrap();

    let ancestor_ids = |p: &LeafPedigree| -> Vec<String> {
        p.ancestors.iter().map(|a| a.id.to_string()).collect()
    };
    // One pedigree per stored leaf, in request order; the unstored id is dropped.
    assert_eq!(pedigrees.len(), 3);
    assert_eq!(pedigrees[0].leaf.id.to_string(), "leaf-a");
    assert_eq!(ancestor_ids(&pedigrees[0]), vec!["mid-a", "root"]);
    assert_eq!(pedigrees[1].leaf.id.to_string(), "leaf-b");
    assert_eq!(ancestor_ids(&pedigrees[1]), vec!["mid-b", "root"]);
    assert_eq!(pedigrees[2].leaf.id.to_string(), "leaf-c");
    assert!(pedigrees[2].ancestors.is_empty());
}

/// A leaf whose stored chain does not reach the root is still spendable and
/// counted: the ancestor chain is needed only at exit time, not for balance or
/// selection. Completing the rest of the chain later keeps it spendable and makes
/// its exit chain fully reconstructable.
pub async fn test_incomplete_pedigree_still_spendable(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let mid = create_test_node_with_parent("mid", Some("root"), TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);

    // Stored without the root: the chain cannot reach it, but the leaf still counts.
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![mid.clone()],
        }],
    )
    .await;
    assert_eq!(store.get_available_balance().await.unwrap(), leaf.value);
    assert_eq!(get_available(store).await.len(), 1);

    // Completing the chain keeps it spendable and makes the exit chain whole.
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![mid, root],
        }],
    )
    .await;
    assert_eq!(store.get_available_balance().await.unwrap(), leaf.value);
    assert_eq!(
        exit_chain_ids(store, &leaf.id).await,
        vec!["root", "mid", "leaf"]
    );
}

/// Chains stored for swap outputs after `update_reservation` put them in the
/// pool survive, so both the reserved and the change leaf keep a reconstructable
/// exit chain. The update itself writes no chains.
pub async fn test_exit_chain_after_swap_update(store: &dyn TreeStore) {
    let original = create_test_tree_node("original", 1_000);
    store
        .add_leaves(std::slice::from_ref(&original))
        .await
        .unwrap();
    let reservation = reserve_leaves(store, None, false, ReservationPurpose::Swap)
        .await
        .unwrap();

    let reserved_root = create_test_node_with_parent("res_root", None, TreeNodeStatus::Splitted);
    let reserved_leaf =
        create_test_node_with_parent("res_leaf", Some("res_root"), TreeNodeStatus::Available);
    let change_root = create_test_node_with_parent("chg_root", None, TreeNodeStatus::Splitted);
    let change_leaf =
        create_test_node_with_parent("chg_leaf", Some("chg_root"), TreeNodeStatus::Available);
    store
        .update_reservation(
            &reservation.id,
            std::slice::from_ref(&reserved_leaf),
            std::slice::from_ref(&change_leaf),
        )
        .await
        .unwrap();
    store
        .store_ancestors(&[
            LeafPedigree {
                leaf: reserved_leaf.clone(),
                ancestors: vec![reserved_root],
            },
            LeafPedigree {
                leaf: change_leaf.clone(),
                ancestors: vec![change_root],
            },
        ])
        .await
        .unwrap();

    assert_eq!(
        exit_chain_ids(store, &reserved_leaf.id).await,
        vec!["res_root", "res_leaf"]
    );
    assert_eq!(
        exit_chain_ids(store, &change_leaf.id).await,
        vec!["chg_root", "chg_leaf"]
    );
}

/// A leaf reparented while reserved (a renewal persists the new chain) still has
/// a complete exit chain after `cancel_reservation` returns it, even though cancel
/// is passed only the leaf.
pub async fn test_exit_chain_after_cancel_reparent(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![root],
        }],
    )
    .await;
    let reservation = reserve_leaves(store, None, false, ReservationPurpose::Payment)
        .await
        .unwrap();

    // The renewal reparents the leaf under a new node and persists the new chain
    // while the leaf is still reserved.
    let new_root = create_test_node_with_parent("new_root", None, TreeNodeStatus::Splitted);
    let mut reparented = leaf.clone();
    reparented.parent_node_id = Some(TreeNodeId::from_str("new_root").unwrap());
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: reparented.clone(),
            ancestors: vec![new_root],
        }],
    )
    .await;

    // Cancel returns only the leaf; its new chain is already in the store.
    store
        .cancel_reservation(&reservation.id, std::slice::from_ref(&reparented))
        .await
        .unwrap();

    assert_eq!(
        exit_chain_ids(store, &leaf.id).await,
        vec!["new_root", "leaf"]
    );
    assert_eq!(store.get_available_balance().await.unwrap(), leaf.value);
}

/// Re-storing an existing node updates it in place rather than duplicating it.
pub async fn test_node_update_in_place(store: &dyn TreeStore) {
    let ancestor = create_test_node_with_parent("root", None, TreeNodeStatus::Available);
    let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![ancestor.clone()],
        }],
    )
    .await;

    // Re-store the ancestor with a new status and a new parent, both applied.
    let mut updated = ancestor.clone();
    updated.status = TreeNodeStatus::OnChain;
    updated.parent_node_id = Some(TreeNodeId::from_str("new-parent").unwrap());
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![updated],
        }],
    )
    .await;

    let pedigree = exit_chain(store, &leaf.id).await.unwrap();
    let stored = &pedigree.ancestors[0];
    assert_eq!(stored.id, ancestor.id);
    assert_eq!(stored.value, 1_000, "value unchanged");
    assert_eq!(stored.status, TreeNodeStatus::OnChain, "status updated");
    assert_eq!(
        stored.parent_node_id,
        Some(TreeNodeId::from_str("new-parent").unwrap()),
        "parent updated"
    );
}

/// A timelock renewal re-parents a leaf (same id) onto a fresh split node, so
/// re-adding it must repoint its stored parent and re-route the exit chain
/// through the new ancestor.
pub async fn test_leaf_reparented_by_renewal(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);
    let old_mid = create_test_node_with_parent("old-mid", Some("root"), TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("old-mid"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![old_mid, root.clone()],
        }],
    )
    .await;

    assert_eq!(
        exit_chain_ids(store, &leaf.id).await,
        vec!["root", "old-mid", "leaf"]
    );

    // Renewal introduces a new split node and re-parents the leaf onto it.
    let new_mid = create_test_node_with_parent("new-mid", Some("root"), TreeNodeStatus::Splitted);
    let mut renewed = leaf.clone();
    renewed.parent_node_id = Some(TreeNodeId::from_str("new-mid").unwrap());
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: renewed,
            ancestors: vec![new_mid, root],
        }],
    )
    .await;

    assert_eq!(
        exit_chain_ids(store, &leaf.id).await,
        vec!["root", "new-mid", "leaf"]
    );
}

/// An ancestor is stored separately from the leaf pool: it is part of the leaf's
/// exit chain but never surfaces as a spendable leaf.
pub async fn test_ancestor_not_returned_as_leaf(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![root.clone()],
        }],
    )
    .await;

    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].id.to_string(), "leaf");
    assert_eq!(exit_chain_ids(store, &leaf.id).await, vec!["root", "leaf"]);
}

/// A leaf stored without its chain has it completed later, without the ancestors
/// surfacing in the leaf pool.
pub async fn test_store_ancestors_backfills_chain(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
    store.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();
    assert_eq!(exit_chain_ids(store, &leaf.id).await, vec!["leaf"]);

    store
        .store_ancestors(&[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![root],
        }])
        .await
        .unwrap();

    assert_eq!(exit_chain_ids(store, &leaf.id).await, vec!["root", "leaf"]);
    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].id.to_string(), "leaf");
}

/// A stored chain outlives the refreshes that keep reporting its leaf, and is
/// dropped only when the leaf itself leaves the pool. Refreshes carry leaves
/// alone, so a chain the resolver wrote must not be collateral damage.
pub async fn test_stored_chain_survives_refresh_and_dies_with_its_leaf(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let kept = create_test_node_with_parent("kept", Some("root"), TreeNodeStatus::Available);
    let dropped = create_test_node_with_parent("dropped", Some("root"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[
            LeafPedigree {
                leaf: kept.clone(),
                ancestors: vec![root.clone()],
            },
            LeafPedigree {
                leaf: dropped.clone(),
                ancestors: vec![root],
            },
        ],
    )
    .await;

    // A refresh reporting both leaves keeps both chains: it writes none itself.
    let refresh_start = future_refresh_start(store).await;
    store
        .set_leaves(&[kept.clone(), dropped.clone()], &[], refresh_start)
        .await
        .unwrap();
    assert_eq!(exit_chain_ids(store, &kept.id).await, vec!["root", "kept"]);
    assert_eq!(
        exit_chain_ids(store, &dropped.id).await,
        vec!["root", "dropped"]
    );

    // A refresh that stops reporting `dropped` takes its chain with it, and
    // leaves the surviving leaf's chain intact.
    let refresh_start = future_refresh_start(store).await;
    store
        .set_leaves(std::slice::from_ref(&kept), &[], refresh_start)
        .await
        .unwrap();
    assert_eq!(exit_chain_ids(store, &kept.id).await, vec!["root", "kept"]);
    assert!(exit_chain(store, &dropped.id).await.is_none());
}

/// A leaf spent between its chain being resolved and stored stays spent: storing
/// ancestors must not return it to the pool the way `add_leaves` would.
pub async fn test_store_ancestors_does_not_revive_spent_leaf(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
    let pedigree = LeafPedigree {
        leaf: leaf.clone(),
        ancestors: vec![root],
    };
    add_with_chains(store, std::slice::from_ref(&pedigree)).await;

    let reservation = reserve_leaves(store, None, false, ReservationPurpose::Payment)
        .await
        .unwrap();
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();
    assert!(get_available(store).await.is_empty());

    store
        .store_ancestors(std::slice::from_ref(&pedigree))
        .await
        .unwrap();

    let all = get_all(store).await;
    assert!(
        all.available.is_empty(),
        "a spent leaf must not return to the pool"
    );
    assert_eq!(all.balance(), 0);
}

/// Storing a chain whose leaf the pool no longer holds is accepted: the ancestors
/// are simply unreachable until they are collected.
pub async fn test_store_ancestors_for_absent_leaf(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);

    store.store_ancestors(&[]).await.unwrap();
    store
        .store_ancestors(&[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![root],
        }])
        .await
        .unwrap();

    assert!(get_available(store).await.is_empty());
    assert!(exit_chain(store, &leaf.id).await.is_none());

    // Nothing was written for the missing leaf. A chain is only ever removed with
    // its leaf, so one stored for a leaf that is already gone would never be
    // collected; adding the leaf afterwards would surface it.
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: Vec::new(),
        }],
    )
    .await;
    assert!(
        exit_chain(store, &leaf.id)
            .await
            .expect("leaf was just added")
            .ancestors
            .is_empty(),
        "ancestors stored for an absent leaf outlived it"
    );
}

/// Deleting a leaf also drops its unshared ancestors.
pub async fn test_unshared_ancestor_deleted_with_leaf(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![root.clone()],
        }],
    )
    .await;

    // A refresh in the future drops the pre-existing leaf, taking its ancestors.
    let refresh_start = future_refresh_start(store).await;
    store.set_leaves(&[], &[], refresh_start).await.unwrap();

    assert!(exit_chain(store, &leaf.id).await.is_none());

    // Re-adding the leaf alone brings back no ancestors: the root went with it,
    // so the walk up from the leaf has nothing to link to.
    store.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();
    assert_eq!(exit_chain_ids(store, &leaf.id).await, vec!["leaf"]);
}

/// Each leaf owns its own copy of a shared ancestor, so dropping one leaf leaves
/// the other's chain intact.
pub async fn test_shared_ancestor_survives_leaf_deletion(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let leaf_a = create_test_node_with_parent("leaf-a", Some("root"), TreeNodeStatus::Available);
    let leaf_b = create_test_node_with_parent("leaf-b", Some("root"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[
            LeafPedigree {
                leaf: leaf_a.clone(),
                ancestors: vec![root.clone()],
            },
            LeafPedigree {
                leaf: leaf_b.clone(),
                ancestors: vec![root.clone()],
            },
        ],
    )
    .await;

    // A refresh that keeps only leaf-b drops leaf-a and its copy of the root.
    let refresh_start = future_refresh_start(store).await;
    store
        .set_leaves(std::slice::from_ref(&leaf_b), &[], refresh_start)
        .await
        .unwrap();

    assert!(exit_chain(store, &leaf_a.id).await.is_none());
    assert_eq!(
        exit_chain_ids(store, &leaf_b.id).await,
        vec!["root", "leaf-b"]
    );
}

/// A pedigree with no ancestors means the chain is unknown, not that the leaf has
/// none, so storing one must leave an already-stored chain alone.
pub async fn test_empty_pedigree_keeps_stored_chain(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![root],
        }],
    )
    .await;
    assert_eq!(exit_chain_ids(store, &leaf.id).await, vec!["root", "leaf"]);

    store.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();
    assert_eq!(exit_chain_ids(store, &leaf.id).await, vec!["root", "leaf"]);

    let refresh_start = future_refresh_start(store).await;
    store
        .set_leaves(std::slice::from_ref(&leaf), &[], refresh_start)
        .await
        .unwrap();
    assert_eq!(exit_chain_ids(store, &leaf.id).await, vec!["root", "leaf"]);
}

/// Storing a chain replaces the leaf's previous one rather than merging, so a leaf
/// reparented onto a new branch does not keep the nodes it no longer descends from.
pub async fn test_stored_chain_replaces_previous(store: &dyn TreeStore) {
    let old_root = create_test_node_with_parent("old-root", None, TreeNodeStatus::Splitted);
    let mut leaf =
        create_test_node_with_parent("leaf", Some("old-root"), TreeNodeStatus::Available);
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![old_root],
        }],
    )
    .await;
    assert_eq!(
        exit_chain_ids(store, &leaf.id).await,
        vec!["old-root", "leaf"]
    );

    let new_root = create_test_node_with_parent("new-root", None, TreeNodeStatus::Splitted);
    let split = create_test_node_with_parent("split", Some("new-root"), TreeNodeStatus::Splitted);
    leaf.parent_node_id = Some(TreeNodeId::from_str("split").unwrap());
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![split, new_root],
        }],
    )
    .await;

    assert_eq!(
        exit_chain_ids(store, &leaf.id).await,
        vec!["new-root", "split", "leaf"]
    );
}

/// Only leaves whose stored chain cannot back an exit are reported, so resolving
/// them never has to walk the whole wallet.
pub async fn test_leaves_missing_exit_chains(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let mid = create_test_node_with_parent("mid", Some("root"), TreeNodeStatus::Splitted);
    let complete =
        create_test_node_with_parent("complete", Some("root"), TreeNodeStatus::Available);
    let deep = create_test_node_with_parent("deep", Some("mid"), TreeNodeStatus::Available);
    let bare = create_test_node_with_parent("bare", Some("root"), TreeNodeStatus::Available);
    let root_leaf = create_test_node_with_parent("root-leaf", None, TreeNodeStatus::Available);

    add_with_chains(
        store,
        &[
            LeafPedigree {
                leaf: complete,
                ancestors: vec![root.clone()],
            },
            LeafPedigree {
                leaf: deep.clone(),
                ancestors: vec![mid.clone(), root.clone()],
            },
            LeafPedigree {
                leaf: bare.clone(),
                ancestors: Vec::new(),
            },
            LeafPedigree {
                leaf: root_leaf,
                ancestors: Vec::new(),
            },
        ],
    )
    .await;

    // Only `bare` has no chain. A leaf that is its own root needs none, and a
    // chain is reported once it holds the leaf's parent, at any depth.
    assert_eq!(missing_chain_ids(store).await, vec!["bare"]);

    store
        .store_ancestors(&[LeafPedigree {
            leaf: bare,
            ancestors: vec![root],
        }])
        .await
        .unwrap();
    assert!(missing_chain_ids(store).await.is_empty());
}

/// A refresh reports leaves without their chains, so it must leave the chains
/// already stored alone. A store that treats the empty incoming chain as "none"
/// rather than "unknown" reports every leaf as missing one after every refresh,
/// and the collection then refetches chains it already holds.
pub async fn test_stored_chain_survives_refresh(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let complete =
        create_test_node_with_parent("complete", Some("root"), TreeNodeStatus::Available);
    let bare = create_test_node_with_parent("bare", Some("root"), TreeNodeStatus::Available);

    add_with_chains(
        store,
        &[
            LeafPedigree {
                leaf: complete.clone(),
                ancestors: vec![root],
            },
            LeafPedigree {
                leaf: bare.clone(),
                ancestors: Vec::new(),
            },
        ],
    )
    .await;
    assert_eq!(missing_chain_ids(store).await, vec!["bare"]);

    // A refresh only rebuilds rows older than its start, and the leaves were
    // just added, so the boundary has to sit clearly after them on the store's
    // own clock. Otherwise the rebuild never happens and this passes vacuously.
    let refresh_start = future_refresh_start(store).await;
    store
        .set_leaves(&[complete, bare], &[], refresh_start)
        .await
        .unwrap();

    assert_eq!(
        missing_chain_ids(store).await,
        vec!["bare"],
        "a refresh must not discard the chain a leaf already had"
    );
}

/// A stored chain only backs an exit if it holds the leaf's *current* parent.
/// Timelock renewal reparents a leaf onto a new split node under the same id, and
/// another device learns that from a leaves-only refresh, which carries no chain.
/// The rows left behind describe the parent the leaf had before.
pub async fn test_reparented_leaf_needs_its_chain_again(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    let old_parent =
        create_test_node_with_parent("old-parent", Some("root"), TreeNodeStatus::Splitted);
    let leaf = create_test_node_with_parent("leaf", Some("old-parent"), TreeNodeStatus::Available);

    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![old_parent, root],
        }],
    )
    .await;
    assert!(
        missing_chain_ids(store).await.is_empty(),
        "a rooted chain from the leaf's parent is complete"
    );

    // Renewal moved the leaf under a new split node. The refresh reports the new
    // parent and no chain, exactly as a leaves-only refresh does.
    let reparented =
        create_test_node_with_parent("leaf", Some("new-parent"), TreeNodeStatus::Available);
    let refresh_start = future_refresh_start(store).await;
    store
        .set_leaves(&[reparented], &[], refresh_start)
        .await
        .unwrap();

    assert_eq!(
        missing_chain_ids(store).await,
        vec!["leaf"],
        "the stored chain no longer reaches the leaf, so it needs collecting again"
    );

    // A chain fetched before the reparent still lands afterwards, carrying the
    // operators' older copy of the leaf. Storing it must be judged against the
    // stored leaf, which has moved on, not against the one that came with it.
    let stale = create_test_node_with_parent("leaf", Some("old-parent"), TreeNodeStatus::Available);
    let old_parent_again =
        create_test_node_with_parent("old-parent", Some("root"), TreeNodeStatus::Splitted);
    let root_again = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    store
        .store_ancestors(&[LeafPedigree {
            leaf: stale,
            ancestors: vec![old_parent_again, root_again],
        }])
        .await
        .unwrap();

    assert_eq!(
        missing_chain_ids(store).await,
        vec!["leaf"],
        "a chain for the parent the leaf used to have does not make it exitable"
    );
}

async fn missing_chain_ids(store: &dyn TreeStore) -> Vec<String> {
    store
        .leaves_missing_exit_chains()
        .await
        .unwrap()
        .into_iter()
        .map(|id| id.to_string())
        .collect()
}

/// Helper function to reserve leaves in tests.
/// Wraps `try_reserve_leaves` and expects success.
pub async fn reserve_leaves(
    store: &dyn TreeStore,
    target_amounts: Option<&TargetAmounts>,
    exact_only: bool,
    purpose: ReservationPurpose,
) -> Result<LeavesReservation, TreeServiceError> {
    match store
        .try_reserve_leaves(target_amounts, exact_only, purpose)
        .await?
    {
        ReserveResult::Success(reservation) => Ok(reservation),
        ReserveResult::InsufficientFunds => Err(TreeServiceError::InsufficientFunds),
        ReserveResult::WaitForPending { .. } => Err(TreeServiceError::Generic(
            "Unexpected WaitForPending".into(),
        )),
    }
}

/// Returns a refresh start 10s in the future relative to the store's clock,
/// ensuring that leaves added "now" are treated as old relative to it.
///
/// Sourced from `store.now()` rather than `SystemTime::now()` so the boundary
/// shares a clock with the DB-stamped `added_at` values it is compared against
/// (the application and DB-server clocks can differ, e.g. a Docker VM clock).
async fn future_refresh_start(store: &dyn TreeStore) -> SystemTime {
    store.now().await.unwrap() + Duration::from_secs(10)
}

/// Returns a refresh start 60s in the past relative to the store's clock,
/// ensuring that leaves/spends recorded "now" are treated as new relative to
/// it. See [`future_refresh_start`] for why this uses `store.now()`.
async fn past_refresh_start(store: &dyn TreeStore) -> SystemTime {
    store.now().await.unwrap() - Duration::from_secs(60)
}

/// Asserts that `get_leaves()` returns `expected` available leaves.
async fn assert_available_count(store: &dyn TreeStore, expected: usize) {
    let leaves = store.get_leaves().await.unwrap();
    assert_eq!(leaves.available.len(), expected);
}

/// Returns the available leaves from the store.
async fn get_available(store: &dyn TreeStore) -> Vec<TreeNode> {
    store.get_leaves().await.unwrap().available
}

/// Returns the full `Leaves` snapshot from the store.
async fn get_all(store: &dyn TreeStore) -> Leaves {
    store.get_leaves().await.unwrap()
}

// ==================== Test functions ====================

pub async fn test_new(store: &dyn TreeStore) {
    assert!(store.get_leaves().await.unwrap().available.is_empty());
}

pub async fn test_get_verified_leaf_keys(store: &dyn TreeStore) {
    // `create_test_tree_node` uses one key for both fields; override these so a
    // projection that swaps or drops a field is caught. Both are valid points.
    let verifying = "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443";
    let keyshare = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    let mut available = create_test_tree_node("verified-available", 1000);
    available.verifying_public_key = PublicKey::from_str(verifying).unwrap();
    available.signing_keyshare.public_key = PublicKey::from_str(keyshare).unwrap();

    // Reserved leaves were verified before storage, so they must be included.
    let reserved = create_test_tree_node("verified-reserved", 2000);

    // Non-Available and unreserved: never verified, so it must be excluded.
    let mut not_available = create_test_tree_node("verified-not-available", 3000);
    not_available.status = TreeNodeStatus::TransferLocked;

    store
        .add_leaves(&[available.clone(), reserved.clone(), not_available.clone()])
        .await
        .unwrap();
    store
        .try_reserve_leaves_by_ids(
            std::slice::from_ref(&reserved.id),
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    let keys = store.get_verified_leaf_keys().await.unwrap();

    // The override must agree with the canonical projection of `get_leaves`.
    let expected = crate::tree::verified_leaf_keys_from_leaves(&store.get_leaves().await.unwrap());
    assert_eq!(keys, expected);

    // Available leaf verified with the right (non-swapped) keys.
    let got = keys.get(&available.id).expect("available leaf verified");
    assert_eq!(
        got.verifying_public_key,
        PublicKey::from_str(verifying).unwrap()
    );
    assert_eq!(
        got.signing_keyshare_public_key,
        PublicKey::from_str(keyshare).unwrap()
    );
    // Every stored leaf is a verified leaf, whatever its status: a leaf part-way
    // through an exit is re-verified on every refresh if it is left out, which
    // costs a remote signer a round trip per leaf.
    assert!(keys.contains_key(&reserved.id));
    assert!(keys.contains_key(&not_available.id));
}

pub async fn test_add_leaves(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];

    store.add_leaves(&leaves).await.unwrap();

    let stored = get_available(store).await;
    assert_eq!(stored.len(), 2);
    assert!(
        stored
            .iter()
            .any(|l| l.id.to_string() == "node1" && l.value == 100)
    );
    assert!(
        stored
            .iter()
            .any(|l| l.id.to_string() == "node2" && l.value == 200)
    );
}

pub async fn test_add_leaves_duplicate_ids(store: &dyn TreeStore) {
    let leaf1 = create_test_tree_node("node1", 100);
    let leaf2 = create_test_tree_node("node1", 200); // Same id, different value

    store.add_leaves(&[leaf1]).await.unwrap();
    // The operators are the source of truth: the later report wins, and the id
    // is updated in place rather than stored twice.
    store.add_leaves(&[leaf2]).await.unwrap();

    let stored = get_available(store).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].value, 200);
}

pub async fn test_set_leaves(store: &dyn TreeStore) {
    let initial = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&initial).await.unwrap();

    let refresh_start = future_refresh_start(store).await;
    let new_leaves = vec![
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store
        .set_leaves(&new_leaves, &[], refresh_start)
        .await
        .unwrap();

    let stored = get_available(store).await;
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().any(|l| l.id.to_string() == "node2"));
    assert!(stored.iter().any(|l| l.id.to_string() == "node3"));
    assert!(!stored.iter().any(|l| l.id.to_string() == "node1"));
}

/// Exercises a leaf set large enough to span many upsert chunks, so a dropped,
/// duplicated or mis-ordered chunk shows up as a wrong count.
pub async fn test_set_leaves_across_chunk_boundary(store: &dyn TreeStore) {
    // Must exceed 10922 leaves: the Rust MySQL store prepares its upsert and
    // binds 6 placeholders per leaf against the protocol's 65535 cap, so an
    // unchunked write fails outright there ("Prepared statement contains too
    // many placeholders") rather than merely using too much memory. The JS
    // MySQL store interpolates client-side instead, so it is bounded by
    // max_allowed_packet rather than that cap. Deliberately not a multiple of
    // the chunk size, so the final chunk is partial.
    const LEAF_COUNT: usize = 12_345;

    let leaves: Vec<TreeNode> = (0..LEAF_COUNT)
        .map(|i| create_test_tree_node(&format!("chunked{i}"), 100 + i as u64))
        .collect();

    let refresh_start = future_refresh_start(store).await;
    store.set_leaves(&leaves, &[], refresh_start).await.unwrap();

    let stored = get_available(store).await;
    assert_eq!(stored.len(), LEAF_COUNT);

    let expected_total: u64 = leaves.iter().map(|l| l.value).sum();
    assert_eq!(
        stored.iter().map(|l| l.value).sum::<u64>(),
        expected_total,
        "every leaf must round-trip with its own value"
    );
    assert_eq!(store.get_available_balance().await.unwrap(), expected_total);

    // Re-run with new values to exercise the conflict path in every chunk. The
    // refresh start must be in the past: a future one deletes the rows just
    // written, leaving the second pass to insert into an empty table.
    let updated: Vec<TreeNode> = (0..LEAF_COUNT)
        .map(|i| create_test_tree_node(&format!("chunked{i}"), 1_000 + i as u64))
        .collect();
    let refresh_start = past_refresh_start(store).await;
    store
        .set_leaves(&updated, &[], refresh_start)
        .await
        .unwrap();

    let stored = get_available(store).await;
    assert_eq!(stored.len(), LEAF_COUNT);
    let updated_total: u64 = updated.iter().map(|l| l.value).sum();
    assert_eq!(
        stored.iter().map(|l| l.value).sum::<u64>(),
        updated_total,
        "every chunk must update in place, not skip its conflicting rows"
    );
}

pub async fn test_set_leaves_with_reservations(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve some leaves
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(600, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    let refresh_start = future_refresh_start(store).await;

    // Update leaves with new data (including updated versions of reserved leaves)
    let non_existing_operator_leaf = create_test_tree_node("node7", 1000);
    let mut updated_leaf1 = create_test_tree_node("node1", 100);
    updated_leaf1.status = crate::tree::TreeNodeStatus::TransferLocked;
    let new_leaves = vec![
        updated_leaf1,
        create_test_tree_node("node2", 200),
        create_test_tree_node("node4", 400),
    ];
    store
        .set_leaves(
            &new_leaves,
            std::slice::from_ref(&non_existing_operator_leaf),
            refresh_start,
        )
        .await
        .unwrap();

    // Check main pool via get_leaves
    let all = get_all(store).await;
    assert_eq!(all.payment_reserved_balance(), 600); // 100+200+300
    assert_eq!(all.available_balance(), 400);
    assert_eq!(all.missing_operators_balance(), 1000);
    assert_eq!(all.balance(), 400 + 1000);
    assert_eq!(all.available.len(), 1);
    assert!(all.available.iter().any(|l| l.id.to_string() == "node4"));
}

pub async fn test_set_leaves_preserves_reservations_for_in_flight_swaps(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves (simulating start of a swap)
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    let refresh_start = future_refresh_start(store).await;

    // Set new leaves that don't include the reserved ones
    let new_leaves = vec![create_test_tree_node("node3", 300)];
    store
        .set_leaves(&new_leaves, &[], refresh_start)
        .await
        .unwrap();

    // Reservation should be PRESERVED - verify through get_leaves
    let all = get_all(store).await;
    assert_eq!(all.reserved_for_payment.len(), 2);
    assert!(
        all.reserved_for_payment
            .iter()
            .any(|l| l.id.to_string() == "node1")
    );
    assert!(
        all.reserved_for_payment
            .iter()
            .any(|l| l.id.to_string() == "node2")
    );
}

pub async fn test_reserve_leaves(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Check through get_leaves that reservation was created
    let all = get_all(store).await;
    assert_eq!(all.reserved_for_payment.len(), 1);
    assert_eq!(all.reserved_for_payment[0].id, leaves[0].id);
    // Check that leaf was removed from main pool
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available[0].id, leaves[1].id);
    assert!(!reservation.id.is_empty());
}

pub async fn test_reserve_leaves_by_ids(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let ids = vec![leaves[0].id.clone(), leaves[2].id.clone()];
    let reservation = store
        .try_reserve_leaves_by_ids(&ids, ReservationPurpose::Payment)
        .await
        .unwrap();

    assert_eq!(reservation.leaves.len(), 2);
    assert!(reservation.leaves.iter().any(|l| l.id == leaves[0].id));
    assert!(reservation.leaves.iter().any(|l| l.id == leaves[2].id));
    assert!(!reservation.id.is_empty());

    let all = get_all(store).await;
    assert_eq!(all.reserved_for_payment.len(), 2);
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available[0].id, leaves[1].id);
}

pub async fn test_reserve_leaves_by_ids_not_available(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    let ids = vec![leaves[0].id.clone()];
    store
        .try_reserve_leaves_by_ids(&ids, ReservationPurpose::Payment)
        .await
        .unwrap();

    // Already reserved: no longer available, so reserving it again must fail
    // and reserve nothing.
    assert!(matches!(
        store
            .try_reserve_leaves_by_ids(&ids, ReservationPurpose::Payment)
            .await,
        Err(TreeServiceError::NonReservableLeaves)
    ));

    // A nonexistent id is not reservable.
    let missing = vec![TreeNodeId::from_str("missing_node").unwrap()];
    assert!(matches!(
        store
            .try_reserve_leaves_by_ids(&missing, ReservationPurpose::Payment)
            .await,
        Err(TreeServiceError::NonReservableLeaves)
    ));

    // An empty request reserves nothing.
    assert!(matches!(
        store
            .try_reserve_leaves_by_ids(&[], ReservationPurpose::Payment)
            .await,
        Err(TreeServiceError::NonReservableLeaves)
    ));

    // Duplicate ids must be rejected, not double-counted.
    let available = vec![create_test_tree_node("node2", 200)];
    store.add_leaves(&available).await.unwrap();
    let duplicates = vec![available[0].id.clone(), available[0].id.clone()];
    assert!(matches!(
        store
            .try_reserve_leaves_by_ids(&duplicates, ReservationPurpose::Payment)
            .await,
        Err(TreeServiceError::NonReservableLeaves)
    ));
}

pub async fn test_reserve_leaves_by_ids_preserves_order(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // The per-leaf signing data in a signed package is positional, so the
    // reservation must return leaves in the requested order.
    let ids = vec![
        leaves[2].id.clone(),
        leaves[0].id.clone(),
        leaves[1].id.clone(),
    ];
    let reservation = store
        .try_reserve_leaves_by_ids(&ids, ReservationPurpose::Payment)
        .await
        .unwrap();

    let got: Vec<TreeNodeId> = reservation.leaves.iter().map(|l| l.id.clone()).collect();
    assert_eq!(got, ids);
}

pub async fn test_try_select_leaves(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let selection = store
        .try_select_leaves(Some(&TargetAmounts::new_amount_and_fee(100, None)))
        .await
        .unwrap();
    let LeafSelection::Exact(selected) = selection else {
        panic!("expected Exact selection for an exact-change target");
    };
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, leaves[0].id);

    let selection = store
        .try_select_leaves(Some(&TargetAmounts::new_amount_and_fee(250, None)))
        .await
        .unwrap();
    assert!(matches!(selection, LeafSelection::SwapNeeded(_)));

    assert!(matches!(
        store
            .try_select_leaves(Some(&TargetAmounts::new_amount_and_fee(500, None)))
            .await,
        Err(TreeServiceError::InsufficientFunds)
    ));

    // Selection is read-only: nothing is reserved.
    let all = get_all(store).await;
    assert_eq!(all.available.len(), 2);
    assert_eq!(all.reserved_for_payment.len(), 0);
}

pub async fn test_cancel_reservation(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    store
        .cancel_reservation(&reservation.id, &reservation.leaves)
        .await
        .unwrap();

    let all = get_all(store).await;
    assert!(all.reserved_for_payment.is_empty());
    assert_eq!(all.available.len(), 2);
    assert!(all.available.iter().any(|l| l.id == leaves[0].id));
    assert!(all.available.iter().any(|l| l.id == leaves[1].id));
}

pub async fn test_cancel_reservation_drops_unkept_leaves(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    assert_eq!(reservation.leaves.len(), 2);

    let keep: Vec<_> = reservation
        .leaves
        .iter()
        .filter(|l| l.id == leaves[0].id)
        .cloned()
        .collect();
    store
        .cancel_reservation(&reservation.id, &keep)
        .await
        .unwrap();

    let all = get_all(store).await;
    assert!(all.reserved_for_payment.is_empty());
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available[0].id, leaves[0].id);
}

pub async fn test_cancel_reservation_drops_all_when_keep_empty(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    store
        .cancel_reservation(&reservation.id, &[])
        .await
        .unwrap();

    let all = get_all(store).await;
    assert!(all.reserved_for_payment.is_empty());
    assert!(all.available.is_empty());
}

pub async fn test_cancel_reservation_nonexistent(store: &dyn TreeStore) {
    let fake_id = "fake-reservation-id".to_string();

    store.cancel_reservation(&fake_id, &[]).await.unwrap();
    assert_available_count(store, 0).await;
}

/// Cancelling a reservation that no longer exists (e.g. released by stale
/// cleanup) still returns `leaves_to_keep` to the pool rather than dropping them.
pub async fn test_cancel_reservation_nonexistent_keeps_leaves(store: &dyn TreeStore) {
    let fake_id = "fake-reservation-id".to_string();
    let leaf = create_test_tree_node("kept", 500);

    store
        .cancel_reservation(&fake_id, std::slice::from_ref(&leaf))
        .await
        .unwrap();

    assert_available_count(store, 1).await;
    assert_eq!(store.get_available_balance().await.unwrap(), 500);
}

pub async fn test_finalize_reservation(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Finalize the reservation
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // Check that reservation was removed and leaf was NOT returned to pool
    let all = get_all(store).await;
    assert!(all.reserved_for_payment.is_empty());
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available[0].id, leaves[1].id);
}

pub async fn test_finalize_reservation_nonexistent(store: &dyn TreeStore) {
    let fake_id = "fake-reservation-id".to_string();

    // Should not panic or cause issues
    store.finalize_reservation(&fake_id, None).await.unwrap();
    assert_available_count(store, 0).await;
}

pub async fn test_multiple_reservations(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Create multiple reservations
    let reservation1 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    let reservation2 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(200, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Check both reservations exist
    let all = get_all(store).await;
    assert_eq!(all.reserved_for_payment.len(), 2);
    // Check main pool has only one leaf left
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available[0].id, leaves[2].id);

    store
        .cancel_reservation(&reservation1.id, &reservation1.leaves)
        .await
        .unwrap();
    assert_eq!(get_all(store).await.available.len(), 2);

    // Finalize the other
    store
        .finalize_reservation(&reservation2.id, None)
        .await
        .unwrap();
    assert_eq!(get_all(store).await.available.len(), 2);
}

pub async fn test_reservation_ids_are_unique(store: &dyn TreeStore) {
    let leaf = create_test_tree_node("node1", 100);
    store.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();

    let r1 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    store.cancel_reservation(&r1.id, &r1.leaves).await.unwrap();
    let r2 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    assert_ne!(r1.id, r2.id);
}

pub async fn test_non_reservable_leaves(store: &dyn TreeStore) {
    let leaf = create_test_tree_node("node1", 100);
    store.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();

    reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    let result = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap_err();
    assert!(matches!(result, TreeServiceError::InsufficientFunds));
}

pub async fn test_reserve_leaves_empty(store: &dyn TreeStore) {
    let err = reserve_leaves(store, None, false, ReservationPurpose::Payment)
        .await
        .unwrap_err();

    assert!(matches!(err, TreeServiceError::NonReservableLeaves));
}

pub async fn test_swap_reservation_included_in_balance(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve some leaves for swap
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        true,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Check that swap-reserved leaves are included in balance
    let all = get_all(store).await;
    assert_eq!(all.swap_reserved_balance(), 300);
    assert_eq!(all.available_balance(), 300);
    assert_eq!(all.balance(), 300 + 300);
}

pub async fn test_payment_reservation_excluded_from_balance(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve some leaves for payment
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Check that payment-reserved leaves are excluded from balance
    let all = get_all(store).await;
    assert_eq!(all.payment_reserved_balance(), 300);
    assert_eq!(all.available_balance(), 300);
    assert_eq!(all.balance(), 300);
}

pub async fn test_try_reserve_success(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let result = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    assert!(matches!(result, ReserveResult::Success(_)));
    if let ReserveResult::Success(reservation) = result {
        assert_eq!(reservation.sum(), 100);
    }
}

pub async fn test_try_reserve_insufficient_funds(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    let result = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(500, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    assert!(matches!(result, ReserveResult::InsufficientFunds));
}

pub async fn test_try_reserve_wait_for_pending(store: &dyn TreeStore) {
    // Add a single 1000 sat leaf
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve with target 100 - store will reserve 1000 and auto-track pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(matches!(r1, ReserveResult::Success(_)));

    // Try to reserve 300 more - should get WaitForPending since pending=900 > 300
    let r2 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    match r2 {
        ReserveResult::WaitForPending {
            needed,
            available,
            pending,
        } => {
            assert_eq!(needed, 300);
            assert_eq!(available, 0);
            assert_eq!(pending, 900);
        }
        _ => panic!("Expected WaitForPending, got {:?}", r2),
    }
}

pub async fn test_try_reserve_min_amount_with_leaves_above_individual_target(
    store: &dyn TreeStore,
) {
    // Regression: the Postgres slim prefilter capped on max(amount, fee), so
    // for AmountAndFee {1000, 500} only a single leaf > 1000 was loaded into
    // the slim set. The minimum-amount fallback then saw insufficient slim
    // value (one 1024 < 1500 target) and returned WaitForPending even though
    // the wallet trivially had the funds. Cap must be amount + fee.
    let leaves = vec![
        create_test_tree_node("node1", 1024),
        create_test_tree_node("node2", 1024),
        create_test_tree_node("node3", 1024),
        create_test_tree_node("node4", 1024),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let result = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(1000, Some(500))),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    match result {
        ReserveResult::Success(reservation) => {
            assert!(reservation.sum() >= 1500);
        }
        other => panic!("Expected Success, got {:?}", other),
    }
}

pub async fn test_try_reserve_min_amount_exact_denominations_above_individual(
    store: &dyn TreeStore,
) {
    // Same regression for ExactDenominations: cap was max() instead of sum,
    // so when no leaf matched any individual denomination the min-amount
    // fallback was starved of candidates above the per-denomination max.
    // Here every denomination is unrepresented as an exact match (no 600s),
    // so selection falls through to the min-amount path which needs leaves
    // summing to 1200.
    let leaves = vec![
        create_test_tree_node("node1", 1024),
        create_test_tree_node("node2", 1024),
        create_test_tree_node("node3", 1024),
    ];
    store.add_leaves(&leaves).await.unwrap();

    let result = store
        .try_reserve_leaves(
            Some(&TargetAmounts::ExactDenominations {
                denominations: vec![600, 600],
            }),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    match result {
        ReserveResult::Success(reservation) => {
            assert!(reservation.sum() >= 1200);
        }
        other => panic!("Expected Success, got {:?}", other),
    }
}

pub async fn test_try_reserve_fail_immediately_when_insufficient(store: &dyn TreeStore) {
    // Add 100 sat leaf
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve it for 50 sats - pending will be 50
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(50, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(matches!(r1, ReserveResult::Success(_)));

    // Request 500 - more than available + pending (0 + 50 < 500)
    let result = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(500, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(matches!(result, ReserveResult::InsufficientFunds));
}

pub async fn test_balance_change_notification(store: &dyn TreeStore) {
    let mut rx = store.subscribe_balance_changes();

    // Add leaves
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    // Wait for notification with timeout
    let result =
        platform_utils::tokio::time::timeout(std::time::Duration::from_millis(100), async {
            rx.changed().await.ok();
        })
        .await;

    assert!(result.is_ok());
}

pub async fn test_pending_cleared_on_cancel(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve with target 100 - auto-tracks pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    let (reservation_id, prior_leaves) = match r1 {
        ReserveResult::Success(r) => (r.id, r.leaves),
        _ => panic!("Expected Success"),
    };

    store
        .cancel_reservation(&reservation_id, &prior_leaves)
        .await
        .unwrap();

    // Try to reserve 300 - should succeed since 1000 sat leaf is back
    let r2 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    assert!(matches!(r2, ReserveResult::Success(_)));
}

pub async fn test_pending_cleared_on_finalize(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve with target 100 - auto-tracks pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    let reservation_id = match r1 {
        ReserveResult::Success(r) => r.id,
        _ => panic!("Expected Success"),
    };

    // Finalize with new leaves (the change from swap)
    let change_leaf = create_test_tree_node("node2", 900);
    store
        .finalize_reservation(&reservation_id, Some(&[change_leaf]))
        .await
        .unwrap();

    // Try to reserve 300 - should succeed since change is now available
    let r2 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    assert!(matches!(r2, ReserveResult::Success(_)));
}

pub async fn test_notification_after_swap_with_exact_amount(store: &dyn TreeStore) {
    let mut rx = store.subscribe_balance_changes();

    // Add a single 1000 sat leaf
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Consume the initial notification
    let _ =
        platform_utils::tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    // Reserve it with target 100 - will reserve all 1000, pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    let reservation_id = match r1 {
        ReserveResult::Success(r) => r.id,
        _ => panic!("Expected Success"),
    };

    // Consume the reservation notification
    let _ =
        platform_utils::tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    // Simulate a swap that returns exactly the target amount (100 sats)
    let swap_result_leaf = create_test_tree_node("node2", 100);
    store
        .update_reservation(
            &reservation_id,
            std::slice::from_ref(&swap_result_leaf),
            &[],
        )
        .await
        .unwrap();

    // Verify that we still get a notification even though net balance is 0 -> 0
    let notification_result =
        platform_utils::tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    assert!(
        notification_result.is_ok(),
        "Expected notification after swap update with exact amount"
    );
}

pub async fn test_notification_on_pending_balance_change(store: &dyn TreeStore) {
    let mut rx = store.subscribe_balance_changes();

    // Add a single 1000 sat leaf
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Consume initial notification
    let _ =
        platform_utils::tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    // Reserve with target 100 - pending=900
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

    // Consume reservation notification
    let _ =
        platform_utils::tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    let (reservation_id, prior_leaves) = match r1 {
        ReserveResult::Success(r) => (r.id, r.leaves),
        _ => panic!("Expected Success"),
    };

    store
        .cancel_reservation(&reservation_id, &prior_leaves)
        .await
        .unwrap();

    // Should get notification because pending balance changed
    let notification_result =
        platform_utils::tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed())
            .await;

    assert!(
        notification_result.is_ok(),
        "Expected notification when pending balance changes"
    );
}

pub async fn test_spent_leaves_not_restored_by_set_leaves(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve node1 for payment
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Finalize the reservation (node1 is now spent)
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // Verify node1 is not in the pool
    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "node2"));
    assert!(!available.iter().any(|l| l.id.to_string() == "node1"));

    // Simulate a refresh that started BEFORE the finalize completed.
    let refresh_start = past_refresh_start(store).await;
    let stale_leaves = vec![
        create_test_tree_node("node1", 100), // This was spent!
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store
        .set_leaves(&stale_leaves, &[], refresh_start)
        .await
        .unwrap();

    // Verify node1 was NOT restored
    let available = get_available(store).await;
    assert_eq!(available.len(), 2); // node2 and node3 only
    assert!(available.iter().any(|l| l.id.to_string() == "node2"));
    assert!(available.iter().any(|l| l.id.to_string() == "node3"));
    assert!(
        !available.iter().any(|l| l.id.to_string() == "node1"),
        "Spent leaf node1 should not be restored by set_leaves when refresh started before spend"
    );
}

/// A refresh that reports an already-spent leaf must not store its chain: the
/// leaf itself is skipped, and a chain is only ever removed with its leaf, so
/// the rows would never be collected.
pub async fn test_set_leaves_skips_chains_of_spent_leaves(store: &dyn TreeStore) {
    let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
    // Needs a parent: a parentless leaf's chain is unreachable by the ancestor
    // walk, so a stray row stored under it would not be observable below.
    let mut leaf = create_test_node_with_parent("node1", Some("root"), TreeNodeStatus::Available);
    leaf.value = 100;
    store.add_leaves(&[leaf.clone()]).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // A refresh that started before the spend still reports the leaf, and the
    // resolver may still be holding a chain for it. The leaf is suppressed, so
    // its chain must be too.
    let refresh_start = past_refresh_start(store).await;
    store
        .set_leaves(&[leaf.clone()], &[], refresh_start)
        .await
        .unwrap();
    store
        .store_ancestors(&[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![root],
        }])
        .await
        .unwrap();

    assert!(get_available(store).await.is_empty());
    // Re-adding the leaf would surface any chain stored for it above.
    add_with_chains(
        store,
        &[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: Vec::new(),
        }],
    )
    .await;
    assert!(
        exit_chain(store, &leaf.id)
            .await
            .expect("leaf was just added")
            .ancestors
            .is_empty(),
        "a chain stored for a spent leaf outlived it"
    );
}

pub async fn test_spent_ids_cleaned_up_when_no_longer_in_refresh(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve and finalize node1
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // First refresh with refresh_start BEFORE spent_at
    let refresh_start = past_refresh_start(store).await;
    let stale_leaves = vec![create_test_tree_node("node1", 100)];
    store
        .set_leaves(&stale_leaves, &[], refresh_start)
        .await
        .unwrap();
    // node1 should be filtered out because it's a recent spend
    assert!(get_available(store).await.is_empty());

    // Second refresh with refresh_start AFTER spent_at
    let refresh_start2 = future_refresh_start(store).await;
    let fresh_leaves = vec![create_test_tree_node("node2", 200)];
    store
        .set_leaves(&fresh_leaves, &[], refresh_start2)
        .await
        .unwrap();

    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "node2"));
}

pub async fn test_add_leaves_not_deleted_by_set_leaves(store: &dyn TreeStore) {
    // Add initial leaves
    let initial = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&initial).await.unwrap();

    // Refresh starts at T1
    let refresh_start = store.now().await.unwrap();

    // Small delay to ensure the new leaf is added AFTER refresh_start
    platform_utils::tokio::time::sleep(Duration::from_millis(10)).await;

    // While refresh is in progress, a new leaf arrives
    let new_leaf = create_test_tree_node("node2", 200);
    store.add_leaves(&[new_leaf]).await.unwrap();

    // Refresh completes with stale data (doesn't include node2)
    let stale_refresh_data = vec![create_test_tree_node("node1", 100)];
    store
        .set_leaves(&stale_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // node2 should be PRESERVED because it was added after refresh started
    let available = get_available(store).await;
    assert_eq!(available.len(), 2);
    assert!(available.iter().any(|l| l.id.to_string() == "node1"));
    assert!(
        available.iter().any(|l| l.id.to_string() == "node2"),
        "Leaf added after refresh started should be preserved"
    );
}

pub async fn test_old_leaves_deleted_by_set_leaves(store: &dyn TreeStore) {
    // Add initial leaves
    let initial = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&initial).await.unwrap();

    // Use a future refresh_start so existing leaves are considered "old"
    let refresh_start = future_refresh_start(store).await;

    // Refresh completes without node2
    let refresh_data = vec![create_test_tree_node("node1", 100)];
    store
        .set_leaves(&refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // node2 should be DELETED
    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "node1"));
    assert!(
        !available.iter().any(|l| l.id.to_string() == "node2"),
        "Leaf added before refresh started should be deleted if not in refresh data"
    );
}

pub async fn test_change_leaves_from_swap_protected(store: &dyn TreeStore) {
    // Add initial leaf
    let initial = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&initial).await.unwrap();

    // Reserve the leaf
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Refresh starts
    let refresh_start = store.now().await.unwrap();

    // Small delay
    platform_utils::tokio::time::sleep(Duration::from_millis(10)).await;

    // Swap completes and adds change leaves
    let reserved_leaf = create_test_tree_node("swap_output", 500);
    let change_leaf = create_test_tree_node("change", 500);
    store
        .update_reservation(
            &reservation.id,
            std::slice::from_ref(&reserved_leaf),
            std::slice::from_ref(&change_leaf),
        )
        .await
        .unwrap();

    // Refresh completes with stale data
    let stale_refresh_data = vec![create_test_tree_node("node1", 1000)];
    store
        .set_leaves(&stale_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // change leaf should be PRESERVED
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "change"),
        "Change leaf from swap should be preserved"
    );
}

pub async fn test_finalize_with_new_leaves_protected(store: &dyn TreeStore) {
    // Add initial leaf
    let initial = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&initial).await.unwrap();

    // Reserve the leaf
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Refresh starts
    let refresh_start = store.now().await.unwrap();

    // Small delay
    platform_utils::tokio::time::sleep(Duration::from_millis(10)).await;

    // Payment completes and adds change via finalize_reservation
    let change_leaf = create_test_tree_node("change", 900);
    store
        .finalize_reservation(&reservation.id, Some(&[change_leaf]))
        .await
        .unwrap();

    // Refresh completes with stale data
    let stale_refresh_data = vec![create_test_tree_node("node1", 1000)];
    store
        .set_leaves(&stale_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // change leaf should be PRESERVED, node1 should NOT be restored (spent)
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "change"),
        "Change leaf from finalize should be preserved"
    );
    assert!(
        !available.iter().any(|l| l.id.to_string() == "node1"),
        "Spent leaf should not be restored"
    );
}

pub async fn test_add_leaves_clears_spent_status(store: &dyn TreeStore) {
    // Add initial leaf
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve and finalize node1 (marks it as spent)
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // Verify node1 is not in the pool
    assert!(get_available(store).await.is_empty());

    // Add the leaf back via add_leaves (simulating receiving it back)
    let returning_leaf = create_test_tree_node("node1", 100);
    store.add_leaves(&[returning_leaf]).await.unwrap();

    // Verify node1 IS now in the pool (spent status was cleared)
    let available = get_available(store).await;
    assert_eq!(
        available.len(),
        1,
        "Leaf should be re-added when receiving it back via add_leaves"
    );
    assert!(
        available.iter().any(|l| l.id.to_string() == "node1"),
        "node1 should be in available leaves"
    );

    // New leaf should also be added
    let new_leaf = create_test_tree_node("node2", 200);
    store.add_leaves(&[new_leaf]).await.unwrap();
    assert_eq!(get_available(store).await.len(), 2);
}

pub async fn test_set_leaves_skipped_during_active_swap(store: &dyn TreeStore) {
    // Add initial leaves
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves for a swap (not payment)
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(300, None)),
        false,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Simulate refresh starting while swap is in progress
    let refresh_start = store.now().await.unwrap();

    platform_utils::tokio::time::sleep(Duration::from_millis(10)).await;

    // Try to set new leaves (should be skipped due to active swap)
    let new_leaves = vec![create_test_tree_node("node3", 300)];
    store
        .set_leaves(&new_leaves, &[], refresh_start)
        .await
        .unwrap();

    // set_leaves should have been skipped
    let all = get_all(store).await;
    assert!(
        all.available.is_empty(),
        "set_leaves should be skipped during active swap"
    );
    assert_eq!(all.reserved_for_swap.len(), 2);
}

pub async fn test_set_leaves_skipped_after_swap_completes_during_refresh(store: &dyn TreeStore) {
    // Add initial leaves
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves for a swap
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Refresh starts at T0
    let refresh_start = store.now().await.unwrap();

    // Small delay to ensure swap completes AFTER refresh started
    platform_utils::tokio::time::sleep(Duration::from_millis(10)).await;

    // Swap completes at T1
    let new_leaves_from_swap = vec![create_test_tree_node("swap_result", 500)];
    store
        .finalize_reservation(&reservation.id, Some(&new_leaves_from_swap))
        .await
        .unwrap();

    // Verify swap result leaves are in the pool
    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "swap_result"));

    // At T2, set_leaves with stale data
    let stale_refresh_data = vec![create_test_tree_node("node1", 1000)];
    store
        .set_leaves(&stale_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // set_leaves should have been SKIPPED
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "swap_result"),
        "Swap result leaf should be preserved after skipped set_leaves"
    );
    assert!(
        !available.iter().any(|l| l.id.to_string() == "node1"),
        "Stale leaf should not be restored when set_leaves is skipped"
    );
}

pub async fn test_set_leaves_proceeds_after_swap_when_refresh_starts_later(store: &dyn TreeStore) {
    // Add initial leaves
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves for a swap
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Swap completes first
    let new_leaves_from_swap = vec![create_test_tree_node("swap_result", 500)];
    store
        .finalize_reservation(&reservation.id, Some(&new_leaves_from_swap))
        .await
        .unwrap();

    // Small delay to ensure refresh starts AFTER swap completed
    platform_utils::tokio::time::sleep(Duration::from_millis(10)).await;

    // Refresh starts AFTER swap completed
    let refresh_start = future_refresh_start(store).await;

    // set_leaves with fresh data
    let fresh_refresh_data = vec![
        create_test_tree_node("swap_result", 500),
        create_test_tree_node("new_deposit", 200),
    ];
    store
        .set_leaves(&fresh_refresh_data, &[], refresh_start)
        .await
        .unwrap();

    // set_leaves should have proceeded normally
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "swap_result"),
        "swap_result should be present"
    );
    assert!(
        available.iter().any(|l| l.id.to_string() == "new_deposit"),
        "new_deposit should be added"
    );
}

pub async fn test_payment_reservation_does_not_block_set_leaves(store: &dyn TreeStore) {
    // Add initial leaves
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve leaves for PAYMENT (not swap)
    let _reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    let refresh_start = future_refresh_start(store).await;

    // set_leaves should proceed (payment reservation doesn't block)
    let new_leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node3", 300),
    ];
    store
        .set_leaves(&new_leaves, &[], refresh_start)
        .await
        .unwrap();

    // node3 should be in the pool (set_leaves was not skipped)
    let available = get_available(store).await;
    assert!(
        available.iter().any(|l| l.id.to_string() == "node3"),
        "New leaf should be added when payment reservation is active"
    );
}

pub async fn test_update_reservation_basic(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve node1 for payment
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    // Update the reservation: swap completed, new reserved + change leaves
    let swap_output = create_test_tree_node("swap_output", 500);
    let change = create_test_tree_node("change", 500);
    let updated = store
        .update_reservation(
            &reservation.id,
            std::slice::from_ref(&swap_output),
            std::slice::from_ref(&change),
        )
        .await
        .unwrap();

    // Same reservation ID
    assert_eq!(updated.id, reservation.id);

    let all = get_all(store).await;
    // Reserved leaves should be updated
    assert_eq!(all.reserved_for_payment.len(), 1);
    assert!(
        all.reserved_for_payment
            .iter()
            .any(|l| l.id.to_string() == "swap_output")
    );
    // Change leaf should be available
    assert!(all.available.iter().any(|l| l.id.to_string() == "change"));
    assert_eq!(all.available_balance(), 500);
}

pub async fn test_update_reservation_nonexistent(store: &dyn TreeStore) {
    let leaf = create_test_tree_node("node1", 100);
    let fake_id = "nonexistent".to_string();
    let result = store
        .update_reservation(&fake_id, std::slice::from_ref(&leaf), &[])
        .await;
    assert!(result.is_err());
}

pub async fn test_update_reservation_clears_pending(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve 100 from a 1000-sat leaf (pending=900)
    let r1 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    let reservation_id = match r1 {
        ReserveResult::Success(r) => r.id,
        _ => panic!("Expected Success"),
    };

    // Second reserve should get WaitForPending (pending=900 exists)
    let r2 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(
        matches!(r2, ReserveResult::WaitForPending { .. }),
        "Expected WaitForPending, got {r2:?}"
    );

    // Update reservation: reserved=[100], change=[900]
    let reserved_leaf = create_test_tree_node("reserved", 100);
    let change_leaf = create_test_tree_node("change", 900);
    store
        .update_reservation(
            &reservation_id,
            std::slice::from_ref(&reserved_leaf),
            std::slice::from_ref(&change_leaf),
        )
        .await
        .unwrap();

    // Now pending is cleared and change(900) is available.
    // Reserve 300 should succeed.
    let r3 = store
        .try_reserve_leaves(
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
    assert!(
        matches!(r3, ReserveResult::Success(_)),
        "Expected Success after pending cleared, got {r3:?}"
    );
}

pub async fn test_update_reservation_preserves_purpose(store: &dyn TreeStore) {
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve for Swap purpose
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1000, None)),
        false,
        ReservationPurpose::Swap,
    )
    .await
    .unwrap();

    // Update reservation
    let swap_output = create_test_tree_node("swap_output", 600);
    let change = create_test_tree_node("change", 400);
    store
        .update_reservation(
            &reservation.id,
            std::slice::from_ref(&swap_output),
            std::slice::from_ref(&change),
        )
        .await
        .unwrap();

    let all = get_all(store).await;
    // Should be in reserved_for_swap, not reserved_for_payment
    assert!(
        all.reserved_for_payment.is_empty(),
        "Updated swap reservation should not appear in reserved_for_payment"
    );
    assert_eq!(all.reserved_for_swap.len(), 1);
    assert!(
        all.reserved_for_swap
            .iter()
            .any(|l| l.id.to_string() == "swap_output")
    );
    // balance() includes swap-reserved + available
    assert_eq!(all.swap_reserved_balance(), 600);
    assert_eq!(all.available_balance(), 400);
    assert_eq!(all.balance(), 600 + 400);
}

pub async fn test_get_leaves_not_available(store: &dyn TreeStore) {
    let mut locked_leaf = create_test_tree_node("locked", 100);
    locked_leaf.status = TreeNodeStatus::TransferLocked;
    let available_leaf = create_test_tree_node("avail", 200);

    store
        .add_leaves(&[locked_leaf, available_leaf])
        .await
        .unwrap();

    let all = get_all(store).await;
    assert_eq!(all.not_available.len(), 1);
    assert!(
        all.not_available
            .iter()
            .any(|l| l.id.to_string() == "locked")
    );
    assert_eq!(all.available.len(), 1);
    assert!(all.available.iter().any(|l| l.id.to_string() == "avail"));
    // available_balance excludes locked leaf
    assert_eq!(all.available_balance(), 200);
}

pub async fn test_get_leaves_missing_operators_filters_spent(store: &dyn TreeStore) {
    // Add node1 and reserve+finalize it (mark as spent)
    let leaves = vec![create_test_tree_node("node1", 100)];
    store.add_leaves(&leaves).await.unwrap();

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(100, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    store
        .finalize_reservation(&reservation.id, None)
        .await
        .unwrap();

    // Call set_leaves with refresh_start BEFORE the spend, so spent marker is preserved
    let refresh_start = past_refresh_start(store).await;
    let missing = vec![
        create_test_tree_node("node1", 100), // was spent
        create_test_tree_node("node3", 300), // new
    ];
    store
        .set_leaves(&[], &missing, refresh_start)
        .await
        .unwrap();

    let all = get_all(store).await;
    // node1 should be filtered out (spent), only node3 remains
    assert_eq!(all.available_missing_from_operators.len(), 1);
    assert!(
        all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "node3")
    );
    assert!(
        !all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "node1"),
        "Spent leaf node1 should be filtered from missing_operators"
    );
}

pub async fn test_missing_operators_replaced_on_set_leaves(store: &dyn TreeStore) {
    let refresh_start1 = future_refresh_start(store).await;
    let missing1 = vec![create_test_tree_node("missing1", 100)];
    store
        .set_leaves(&[], &missing1, refresh_start1)
        .await
        .unwrap();

    // Verify missing1 is present
    let all = get_all(store).await;
    assert_eq!(all.available_missing_from_operators.len(), 1);
    assert!(
        all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "missing1")
    );

    // Second set_leaves replaces with missing2
    let refresh_start2 = future_refresh_start(store).await;
    let missing2 = vec![create_test_tree_node("missing2", 200)];
    store
        .set_leaves(&[], &missing2, refresh_start2)
        .await
        .unwrap();

    let all = get_all(store).await;
    assert_eq!(all.available_missing_from_operators.len(), 1);
    assert!(
        all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "missing2")
    );
    assert!(
        !all.available_missing_from_operators
            .iter()
            .any(|l| l.id.to_string() == "missing1"),
        "missing1 should be replaced, not accumulated"
    );
}

/// A leaf must never be both available and missing-from-operators: `balance()`
/// sums the two sets.
pub async fn test_add_leaves_clears_missing_from_operators(store: &dyn TreeStore) {
    let leaf = create_test_tree_node("leaf1", 49_901);

    let refresh_start = future_refresh_start(store).await;
    store
        .set_leaves(&[], std::slice::from_ref(&leaf), refresh_start)
        .await
        .unwrap();
    assert_eq!(get_all(store).await.balance(), 49_901);

    store.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();

    let all = get_all(store).await;
    assert_eq!(
        all.balance(),
        49_901,
        "leaf counted twice: available={}, missing_from_operators={}",
        all.available_balance(),
        all.missing_operators_balance()
    );
    assert_eq!(store.get_available_balance().await.unwrap(), 49_901);
    assert_eq!(all.available.len(), 1);
    assert!(
        all.available_missing_from_operators.is_empty(),
        "add_leaves must clear the missing-from-operators entry"
    );
}

/// A leaf an operator no longer reports still counts towards the balance, but
/// cannot back a payment or a swap: signing needs every operator.
pub async fn test_missing_from_operators_leaves_are_not_selectable(store: &dyn TreeStore) {
    let available = create_test_tree_node("available1", 1_000);
    let missing = create_test_tree_node("missing1", 500);

    let refresh_start = future_refresh_start(store).await;
    store
        .set_leaves(
            std::slice::from_ref(&available),
            std::slice::from_ref(&missing),
            refresh_start,
        )
        .await
        .unwrap();
    assert_eq!(get_all(store).await.balance(), 1_500);

    let by_ids = store
        .try_reserve_leaves_by_ids(
            std::slice::from_ref(&missing.id),
            ReservationPurpose::Payment,
        )
        .await;
    assert!(
        matches!(by_ids, Err(TreeServiceError::NonReservableLeaves)),
        "reserving a missing-from-operators leaf by id must fail"
    );

    // The 1_500 balance includes the missing leaf, but only 1_000 is selectable.
    let too_large = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1_500, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await;
    assert!(
        matches!(too_large, Err(TreeServiceError::InsufficientFunds)),
        "a missing-from-operators leaf must not be selected to reach the target"
    );

    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(1_000, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    assert_eq!(reservation.leaves.len(), 1);
    assert_eq!(reservation.leaves[0].id, available.id);
}

/// A leaf the operators no longer report and whose status is not `Available` is
/// listed as not-available, so the wallet still knows about it (it can be exited
/// from local state) while nothing counts it as spendable.
pub async fn test_missing_from_operators_leaf_not_available(store: &dyn TreeStore) {
    let mut leaf = create_test_tree_node("locked", 700);
    leaf.status = TreeNodeStatus::TransferLocked;

    let refresh_start = future_refresh_start(store).await;
    store
        .set_leaves(&[], std::slice::from_ref(&leaf), refresh_start)
        .await
        .unwrap();

    let all = get_all(store).await;
    assert_eq!(all.not_available.len(), 1);
    assert_eq!(all.not_available[0].id, leaf.id);
    assert!(all.available_missing_from_operators.is_empty());
    assert!(all.available.is_empty());
    assert_eq!(all.balance(), 0);
}

pub async fn test_reserve_with_none_target_reserves_all(store: &dyn TreeStore) {
    let leaves = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
        create_test_tree_node("node3", 300),
    ];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve with None target -> should reserve all leaves
    let reservation = reserve_leaves(store, None, false, ReservationPurpose::Payment)
        .await
        .unwrap();

    assert_eq!(reservation.leaves.len(), 3);
    let all = get_all(store).await;
    assert!(all.available.is_empty(), "All leaves should be reserved");
    assert_eq!(all.reserved_for_payment.len(), 3);
}

pub async fn test_reserve_skips_non_available_leaves(store: &dyn TreeStore) {
    let node1 = create_test_tree_node("node1", 100);
    let mut node2 = create_test_tree_node("node2", 200);
    node2.status = TreeNodeStatus::TransferLocked;
    let node3 = create_test_tree_node("node3", 300);

    store.add_leaves(&[node1, node2, node3]).await.unwrap();

    // Reserve 400 exact -> should pick node1(100) + node3(300)
    let reservation = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(400, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();

    assert_eq!(reservation.sum(), 400);
    let all = get_all(store).await;
    // node2 should remain in not_available
    assert_eq!(all.not_available.len(), 1);
    assert!(
        all.not_available
            .iter()
            .any(|l| l.id.to_string() == "node2")
    );
    // Available pool should be empty (node1 and node3 were reserved)
    assert!(all.available.is_empty());
}

pub async fn test_add_leaves_empty_slice(store: &dyn TreeStore) {
    // Adding empty slice should succeed with no state change
    store.add_leaves(&[]).await.unwrap();
    assert_available_count(store, 0).await;

    // Add a real leaf, then add empty slice again
    let leaf = create_test_tree_node("node1", 100);
    store.add_leaves(&[leaf]).await.unwrap();
    assert_available_count(store, 1).await;

    store.add_leaves(&[]).await.unwrap();
    assert_available_count(store, 1).await;
}

pub async fn test_full_payment_cycle(store: &dyn TreeStore) {
    // Add node1(1000)
    let leaves = vec![create_test_tree_node("node1", 1000)];
    store.add_leaves(&leaves).await.unwrap();

    // Reserve 400 (non-exact, gets 1000)
    let reservation1 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(400, None)),
        false,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    assert_eq!(reservation1.sum(), 1000);

    // Finalize with change=[change(600)]
    let change_leaf = create_test_tree_node("change", 600);
    store
        .finalize_reservation(&reservation1.id, Some(&[change_leaf]))
        .await
        .unwrap();

    // change(600) should now be available
    let all = get_all(store).await;
    assert_eq!(all.available.len(), 1);
    assert_eq!(all.available_balance(), 600);

    // Reserve 600 (exact) from change
    let reservation2 = reserve_leaves(
        store,
        Some(&TargetAmounts::new_amount_and_fee(600, None)),
        true,
        ReservationPurpose::Payment,
    )
    .await
    .unwrap();
    assert_eq!(reservation2.sum(), 600);

    // Finalize with no new leaves
    store
        .finalize_reservation(&reservation2.id, None)
        .await
        .unwrap();

    // Store should be empty
    let all = get_all(store).await;
    assert!(all.available.is_empty());
    assert!(all.reserved_for_payment.is_empty());
    assert!(all.reserved_for_swap.is_empty());
    assert_eq!(all.balance(), 0);
}

pub async fn test_set_leaves_replaces_fully(store: &dyn TreeStore) {
    let refresh_start1 = future_refresh_start(store).await;
    let initial = vec![
        create_test_tree_node("node1", 100),
        create_test_tree_node("node2", 200),
    ];
    store
        .set_leaves(&initial, &[], refresh_start1)
        .await
        .unwrap();
    assert_available_count(store, 2).await;

    // Second set_leaves with only node3
    let refresh_start2 = future_refresh_start(store).await;
    let replacement = vec![create_test_tree_node("node3", 300)];
    store
        .set_leaves(&replacement, &[], refresh_start2)
        .await
        .unwrap();

    let available = get_available(store).await;
    assert_eq!(available.len(), 1);
    assert!(available.iter().any(|l| l.id.to_string() == "node3"));
    assert!(!available.iter().any(|l| l.id.to_string() == "node1"));
    assert!(!available.iter().any(|l| l.id.to_string() == "node2"));
}
