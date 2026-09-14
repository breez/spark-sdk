use std::collections::HashMap;

use spark::tree::Leaves;

use super::config::PoolConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSpec {
    pub denomination: u64,
    pub leaf_count: usize,
}

impl TreeSpec {
    pub fn total_value(&self) -> u64 {
        self.denomination.saturating_mul(self.leaf_count as u64)
    }
}

/// The trees to create, and the requested leaves they do not cover.
#[derive(Debug)]
pub(crate) struct Plan {
    pub trees: Vec<TreeSpec>,
    pub unplanned: HashMap<u64, u32>,
}

/// Plans at most `max_trees` whole trees for the largest deficits first, within
/// `budget_sats` with `output_fee_sats` charged per tree. `pending_creations`
/// counts towards the target, and `requested` is on top of it.
#[allow(dead_code)]
pub(crate) fn compute_trees_needed(
    current_leaves: &Leaves,
    config: &PoolConfig,
    budget_sats: u64,
    output_fee_sats: u64,
    pending_creations: &HashMap<u64, u32>,
    requested: &HashMap<u64, u32>,
    max_trees: usize,
) -> Plan {
    let mut counts: HashMap<u64, u32> = HashMap::new();
    for leaf in current_leaves
        .available
        .iter()
        .chain(&current_leaves.available_missing_from_operators)
    {
        let count = counts.entry(leaf.value).or_default();
        *count = count.saturating_add(1);
    }

    let mut deficits: Vec<(u64, u32, u32)> = config
        .denominations()
        .into_iter()
        .filter_map(|denom| {
            let current = counts.get(&denom).copied().unwrap_or(0);
            let pending = pending_creations.get(&denom).copied().unwrap_or(0);
            let short = config
                .leaves_per_denomination
                .saturating_sub(current.saturating_add(pending));
            let asked = requested.get(&denom).copied().unwrap_or(0);
            (short.saturating_add(asked) > 0).then_some((denom, short, asked))
        })
        .collect();
    deficits.sort_by(|a, b| {
        let deficit = |(_, short, asked): &(u64, u32, u32)| short.saturating_add(*asked);
        deficit(b).cmp(&deficit(a)).then_with(|| a.0.cmp(&b.0))
    });

    let mut remaining = budget_sats;
    let mut plan = Plan {
        trees: Vec::new(),
        unplanned: requested.clone(),
    };
    for (denom, short, asked) in deficits {
        let leaf_count = PoolConfig::leaves_per_tree(denom);
        let tree_leaves = u32::try_from(leaf_count).unwrap_or(u32::MAX);
        let tree_cost = PoolConfig::tree_value(denom).saturating_add(output_fee_sats);
        let mut planned: u32 = 0;
        for _ in 0..short.saturating_add(asked).div_ceil(tree_leaves) {
            if tree_cost > remaining || plan.trees.len() >= max_trees {
                break;
            }
            plan.trees.push(TreeSpec {
                denomination: denom,
                leaf_count,
            });
            remaining = remaining.saturating_sub(tree_cost);
            planned = planned.saturating_add(tree_leaves);
        }
        if let Some(left) = plan.unplanned.get_mut(&denom) {
            *left = left.saturating_sub(planned.saturating_sub(short));
        }
    }
    plan.unplanned.retain(|_, count| *count > 0);
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark::tree::{TreeNode, TreeNodeId, TreeNodeStatus};

    fn make_leaves(values: &[u64]) -> Leaves {
        let available = values.iter().map(|&v| make_node(v)).collect();
        Leaves {
            available,
            not_available: vec![],
            available_missing_from_operators: vec![],
            reserved_for_payment: vec![],
            reserved_for_swap: vec![],
        }
    }

    fn make_node(value: u64) -> TreeNode {
        use bitcoin::Transaction;

        TreeNode {
            id: TreeNodeId::generate(),
            tree_id: String::new(),
            value,
            parent_node_id: None,
            node_tx: Transaction {
                version: bitcoin::transaction::Version::non_standard(3),
                lock_time: bitcoin::absolute::LockTime::ZERO,
                input: vec![],
                output: vec![],
            },
            refund_tx: None,
            direct_tx: None,
            direct_refund_tx: None,
            direct_from_cpfp_refund_tx: None,
            vout: 0,
            verifying_public_key: dummy_pubkey(),
            owner_identity_public_key: None,
            signing_keyshare: dummy_keyshare(),
            status: TreeNodeStatus::Available,
        }
    }

    fn dummy_pubkey() -> bitcoin::secp256k1::PublicKey {
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[1u8; 32]).unwrap();
        PublicKey::from_secret_key(&secp, &sk)
    }

    fn dummy_keyshare() -> spark::tree::SigningKeyshare {
        spark::tree::SigningKeyshare {
            owner_identifiers: vec![],
            threshold: 2,
            public_key: dummy_pubkey(),
        }
    }

    fn config(target: u32, max_power: u32) -> PoolConfig {
        PoolConfig {
            leaves_per_denomination: target,
            max_denomination_power: max_power,
            replenish_interval: std::time::Duration::from_secs(60),
        }
    }

    fn no_requests() -> HashMap<u64, u32> {
        HashMap::new()
    }

    fn no_pending() -> HashMap<u64, u32> {
        HashMap::new()
    }

    #[test]
    fn test_empty_pool_creates_trees_within_budget() {
        let leaves = make_leaves(&[]);
        let cfg = config(1024, 1);

        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            3072,
            0,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert_eq!(trees.len(), 2);
        assert_eq!(trees.iter().map(TreeSpec::total_value).sum::<u64>(), 3072);
    }

    #[test]
    fn test_budget_too_small_for_any_tree() {
        let leaves = make_leaves(&[]);
        let cfg = config(1024, 0);

        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            500,
            0,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert!(trees.is_empty());
    }

    #[test]
    fn test_full_pool_returns_empty() {
        let leaves = make_leaves(&vec![1u64; 1024]);
        let cfg = config(1024, 0);

        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            100_000,
            0,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert!(trees.is_empty());
    }

    #[test]
    fn test_zero_budget_returns_empty() {
        let leaves = make_leaves(&[]);
        let cfg = config(1024, 0);

        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            0,
            0,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert!(trees.is_empty());
    }

    #[test]
    fn test_large_denomination_gets_16_leaves() {
        let leaves = make_leaves(&[]);
        let cfg = config(16, 13);

        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            100_000_000,
            0,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        let tree_8192 = trees.iter().find(|t| t.denomination == 8192);
        assert!(tree_8192.is_some());
        assert_eq!(tree_8192.unwrap().leaf_count, 16);
    }

    #[test]
    fn test_small_denomination_gets_1024_leaves() {
        let leaves = make_leaves(&[]);
        let cfg = config(1024, 12);

        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            10_000_000,
            0,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        let tree_4096 = trees.iter().find(|t| t.denomination == 4096);
        assert!(tree_4096.is_some());
        assert_eq!(tree_4096.unwrap().leaf_count, 1024);
    }

    #[test]
    fn test_partial_pool_only_fills_deficit() {
        let leaves = make_leaves(&vec![1u64; 1024]);
        let cfg = config(1024, 1);

        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            100_000,
            0,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert_eq!(trees.len(), 1);
        assert_eq!(trees[0].denomination, 2);
    }

    #[test]
    fn a_tree_is_charged_its_output_fee() {
        let leaves = make_leaves(&[]);
        let cfg = config(1, 0);
        let tree = PoolConfig::tree_value(1);
        let exact = compute_trees_needed(
            &leaves,
            &cfg,
            tree,
            0,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert_eq!(exact.len(), 1);
        let short = compute_trees_needed(
            &leaves,
            &cfg,
            tree,
            1,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert!(short.is_empty());
    }

    #[test]
    fn test_budget_constrains_which_trees_are_created() {
        let leaves = make_leaves(&[]);
        let cfg = config(1024, 2);

        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            3072,
            0,
            &no_pending(),
            &no_requests(),
            usize::MAX,
        )
        .trees;
        let total: u64 = trees.iter().map(TreeSpec::total_value).sum();
        assert!(total <= 3072);
        assert!(!trees.is_empty());
    }

    #[test]
    fn test_pending_creations_fully_cover_deficit() {
        let leaves = make_leaves(&[]);
        let cfg = config(1024, 1);

        let pending: HashMap<u64, u32> = [(1u64, 1024u32), (2u64, 1024u32)].into_iter().collect();
        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            100_000,
            0,
            &pending,
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert!(
            trees.is_empty(),
            "pending creations should cover the deficit"
        );
    }

    #[test]
    fn test_pending_creations_partially_reduce_deficit() {
        let leaves = make_leaves(&[]);
        let cfg = config(1024, 1);

        let pending: HashMap<u64, u32> = [(1u64, 1024u32)].into_iter().collect();
        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            100_000,
            0,
            &pending,
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert_eq!(trees.len(), 1);
        assert_eq!(trees[0].denomination, 2);
    }

    #[test]
    fn a_request_is_planned_in_full() {
        let leaves = make_leaves(&[8192u64; 16]);
        let cfg = config(16, 13);
        let requested = HashMap::from([(8192u64, 40u32)]);
        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            u64::MAX,
            0,
            &no_pending(),
            &requested,
            usize::MAX,
        )
        .trees;
        let for_request = trees.iter().filter(|t| t.denomination == 8192).count();
        assert_eq!(for_request, 3);
    }

    #[test]
    fn supply_in_flight_does_not_count_against_a_request() {
        let leaves = make_leaves(&[8192u64; 16]);
        let cfg = config(16, 13);
        let pending = HashMap::from([(8192u64, 16u32)]);
        let requested = HashMap::from([(8192u64, 16u32)]);
        let trees =
            compute_trees_needed(&leaves, &cfg, u64::MAX, 0, &pending, &requested, usize::MAX)
                .trees;
        assert_eq!(trees.iter().filter(|t| t.denomination == 8192).count(), 1);
    }

    #[test]
    fn test_pending_plus_existing_leaves_subtract() {
        let leaves = make_leaves(&vec![1u64; 512]);
        let cfg = config(1024, 1);

        let pending: HashMap<u64, u32> = [(1u64, 512u32)].into_iter().collect();
        let trees = compute_trees_needed(
            &leaves,
            &cfg,
            100_000,
            0,
            &pending,
            &no_requests(),
            usize::MAX,
        )
        .trees;
        assert_eq!(trees.len(), 1);
        assert_eq!(trees[0].denomination, 2);
    }

    #[test]
    fn trees_filling_the_target_leave_the_request() {
        let leaves = make_leaves(&[]);
        let cfg = config(16, 13);
        let requested = HashMap::from([(8192u64, 16u32)]);
        let one_tree = PoolConfig::tree_value(8192);

        let plan = compute_trees_needed(
            &leaves,
            &cfg,
            one_tree,
            0,
            &no_pending(),
            &requested,
            usize::MAX,
        );
        assert_eq!(plan.trees.len(), 1);
        assert_eq!(plan.unplanned, requested);

        let plan = compute_trees_needed(
            &leaves,
            &cfg,
            2 * one_tree,
            0,
            &no_pending(),
            &requested,
            usize::MAX,
        );
        assert_eq!(plan.trees.len(), 2);
        assert!(plan.unplanned.is_empty());
    }

    #[test]
    fn a_request_the_budget_cuts_short_keeps_the_rest() {
        let leaves = make_leaves(&[8192u64; 16]);
        let cfg = config(16, 13);
        let requested = HashMap::from([(8192u64, 40u32), (1024u64, 5u32)]);
        let two_trees = 2 * PoolConfig::tree_value(8192);

        let plan = compute_trees_needed(
            &leaves,
            &cfg,
            two_trees,
            0,
            &no_pending(),
            &requested,
            usize::MAX,
        );
        assert_eq!(plan.trees.len(), 2);
        assert_eq!(plan.unplanned, HashMap::from([(8192, 8), (1024, 5)]));
    }

    #[test]
    fn a_cycle_plans_at_most_max_trees() {
        let cfg = config(1, 2);
        let leaves = make_leaves(&[]);
        let trees =
            compute_trees_needed(&leaves, &cfg, u64::MAX, 0, &no_pending(), &no_requests(), 2)
                .trees;
        assert_eq!(trees.len(), 2);
    }
}
