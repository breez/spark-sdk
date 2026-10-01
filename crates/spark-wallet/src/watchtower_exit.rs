use std::collections::{HashMap, HashSet};

use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    absolute::LockTime, secp256k1::Secp256k1, transaction::Version,
};
use spark::{
    services::Fee,
    tree::{TreeNode, TreeNodeId, TreeNodeStatus},
};

use crate::unilateral_exit::{ChainQuery, ChainResult, Observation, ObservedIndex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchtowerExitedOutput {
    pub leaf_id: TreeNodeId,
    pub outpoint: OutPoint,
    pub tx_out: TxOut,
}

pub const WATCHTOWER_EXITED_STATUSES: [TreeNodeStatus; 2] = [
    TreeNodeStatus::WatchtowerExited,
    TreeNodeStatus::WatchtowerExitRecovered,
];

pub fn is_watchtower_exited(status: TreeNodeStatus) -> bool {
    WATCHTOWER_EXITED_STATUSES.contains(&status)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExitedFunds {
    Found(WatchtowerExitedOutput),
    /// The chain does not show which direct tx confirmed: the output of the
    /// one the node holds.
    Assumed(WatchtowerExitedOutput),
    /// The direct tx pays no output to the leaf's key, so the operators co-sign
    /// no recovery of the leaf.
    Unrecoverable,
    /// The leaf's own refunds recover it.
    Unilateral,
    NotFound,
}

enum Spender {
    NodeTx,
    DirectTx,
    Other,
}

impl Spender {
    fn of(tx: &Transaction, node: &TreeNode, direct_tx: &Transaction) -> Self {
        if tx.compute_txid() == node.node_tx.compute_txid() {
            Self::NodeTx
        } else if is_direct_tx_of(tx, direct_tx) {
            Self::DirectTx
        } else {
            Self::Other
        }
    }
}

/// The confirmed spend of a node's parent output, as far as the chain shows.
enum Spend<'a> {
    NotShown,
    By(&'a Transaction),
    /// By a transaction that could not be read.
    Unread,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedWatchtowerExits {
    pub outputs: Vec<WatchtowerExitedOutput>,
    /// Outputs of the direct tx a node holds, where the chain does not show
    /// which direct tx confirmed.
    pub assumed: Vec<WatchtowerExitedOutput>,
    /// The direct tx has no output paying these leaves' key, so the operators
    /// co-sign no recovery of them.
    pub unrecoverable: Vec<TreeNodeId>,
    /// On-chain leaves that their own refunds recover.
    pub unilateral: Vec<TreeNodeId>,
}

#[derive(Default)]
pub struct WatchtowerExitScan {
    pub resolved: ResolvedWatchtowerExits,
    pub pending: Vec<ChainQuery>,
}

/// Finds the on-chain output holding the funds of each of `leaves`. `nodes`
/// holds their ancestors, which an on-chain leaf does not need. Run it again
/// with the results of `pending` until none is left.
pub fn scan_watchtower_exits(
    leaves: &[TreeNode],
    nodes: &HashMap<TreeNodeId, TreeNode>,
    observed: &[Observation],
) -> WatchtowerExitScan {
    let index = ObservedIndex::new(observed);
    let mut scan = WatchtowerExitScan::default();
    for leaf in leaves {
        match exited_funds(leaf, nodes, &index, &mut scan.pending) {
            ExitedFunds::Found(output) => scan.resolved.outputs.push(output),
            ExitedFunds::Assumed(output) => scan.resolved.assumed.push(output),
            ExitedFunds::Unrecoverable => scan.resolved.unrecoverable.push(leaf.id.clone()),
            ExitedFunds::Unilateral => scan.resolved.unilateral.push(leaf.id.clone()),
            ExitedFunds::NotFound => {}
        }
    }
    let mut seen = HashSet::new();
    scan.pending.retain(|query| seen.insert(query.clone()));
    scan
}

fn exited_funds(
    leaf: &TreeNode,
    nodes: &HashMap<TreeNodeId, TreeNode>,
    observed: &ObservedIndex<'_>,
    pending: &mut Vec<ChainQuery>,
) -> ExitedFunds {
    let mut visited = HashSet::new();
    let mut node = nodes.get(&leaf.id).unwrap_or(leaf);
    loop {
        // The operators supply the parent ids, so a cycle is possible.
        if !visited.insert(node.id.clone()) {
            return ExitedFunds::NotFound;
        }
        let is_leaf = node.id == leaf.id;
        let on_chain = node.status == TreeNodeStatus::OnChain;
        // The operators report an on-chain leaf as recovered once they co-sign
        // its recovery.
        let recovered = is_leaf && node.status == TreeNodeStatus::WatchtowerExitRecovered;
        if (on_chain || recovered)
            && let Some(direct_tx) = node.direct_tx.as_ref()
        {
            match confirmed_spend(node, direct_tx, observed, pending) {
                Spend::By(spender) => {
                    let held = spender.compute_txid() == direct_tx.compute_txid();
                    return match Spender::of(spender, node, direct_tx) {
                        Spender::DirectTx if !(is_leaf && held) => {
                            output_paying_leaf(leaf, spender)
                                .map_or(ExitedFunds::Unrecoverable, ExitedFunds::Found)
                        }
                        // The leaf's refunds spend its node tx and the direct tx it holds.
                        Spender::NodeTx | Spender::DirectTx if is_leaf && on_chain => {
                            ExitedFunds::Unilateral
                        }
                        _ => ExitedFunds::NotFound,
                    };
                }
                Spend::Unread => return ExitedFunds::NotFound,
                Spend::NotShown if on_chain && !is_leaf => {
                    return output_paying_leaf(leaf, direct_tx)
                        .map_or(ExitedFunds::Unrecoverable, ExitedFunds::Assumed);
                }
                Spend::NotShown => {}
            }
        }
        // The ancestors of an on-chain node confirmed through their node txs, so
        // none of them holds the funds.
        if on_chain {
            return ExitedFunds::NotFound;
        }
        let Some(parent) = node.parent_node_id.as_ref().and_then(|id| nodes.get(id)) else {
            return ExitedFunds::NotFound;
        };
        node = parent;
    }
}

/// A lookup missing from `observed` is added to `pending`.
fn confirmed_spend<'a>(
    node: &'a TreeNode,
    direct_tx: &'a Transaction,
    observed: &ObservedIndex<'a>,
    pending: &mut Vec<ChainQuery>,
) -> Spend<'a> {
    let Some(input) = direct_tx.input.first() else {
        return Spend::NotShown;
    };
    let query = ChainQuery::Outspend(input.previous_output);
    let Some(result) = observed.get(&query) else {
        pending.push(query);
        return Spend::NotShown;
    };
    let txid = match result {
        ChainResult::Spend(Some(spend)) if spend.confirmed => spend.spender_txid,
        _ => return Spend::NotShown,
    };
    for held in [&node.node_tx, direct_tx] {
        if held.compute_txid() == txid {
            return Spend::By(held);
        }
    }
    let query = ChainQuery::Transaction(txid);
    match observed.get(&query) {
        None => {
            pending.push(query);
            Spend::Unread
        }
        Some(ChainResult::Transaction(tx)) if tx.compute_txid() == txid => Spend::By(tx),
        Some(_) => Spend::Unread,
    }
}

// The operators hold the direct tx at other fees as well, which differ only in
// the amount paid.
fn is_direct_tx_of(tx: &Transaction, direct_tx: &Transaction) -> bool {
    matches!(
        (tx.input.as_slice(), direct_tx.input.first()),
        ([input], Some(direct)) if input.previous_output == direct.previous_output
            && input.sequence == direct.sequence
    )
}

fn output_paying_leaf(leaf: &TreeNode, direct_tx: &Transaction) -> Option<WatchtowerExitedOutput> {
    let leaf_script = ScriptBuf::new_p2tr(
        &Secp256k1::verification_only(),
        leaf.verifying_public_key.x_only_public_key().0,
        None,
    );
    let (vout, tx_out) = direct_tx
        .output
        .iter()
        .enumerate()
        .find(|(_, output)| output.script_pubkey == leaf_script)?;
    Some(WatchtowerExitedOutput {
        leaf_id: leaf.id.clone(),
        outpoint: OutPoint {
            txid: direct_tx.compute_txid(),
            vout: u32::try_from(vout).ok()?,
        },
        tx_out: tx_out.clone(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsignedWatchtowerExitRecovery {
    pub tx: Transaction,
    pub fee_sat: u64,
    /// The size the transaction has once signed.
    pub vsize: u64,
}

/// `None` when the fee leaves the destination nothing. A fee below the relay
/// minimum, or one that leaves less than the dust limit, is built as asked, for
/// a miner reached directly.
pub fn build_watchtower_exit_recovery(
    output: &WatchtowerExitedOutput,
    destination: &Address,
    fee: Fee,
) -> Option<UnsignedWatchtowerExitRecovery> {
    let mut recovery_tx = Transaction {
        // A version 3 parent only accepts a version 3 child, so version 2 lets
        // the recipient bump the fee with an ordinary one.
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: output.outpoint,
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            // Sized with the key-path signature it will carry.
            witness: Witness::from_slice(&[[0u8; 64]]),
            ..Default::default()
        }],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: destination.script_pubkey(),
        }],
    };
    let vsize = recovery_tx.vsize() as u64;
    recovery_tx.input[0].witness = Witness::new();

    let fee_sat = fee.to_sats(vsize);
    let value = output.tx_out.value.to_sat().checked_sub(fee_sat)?;
    if value == 0 {
        return None;
    }
    recovery_tx.output[0].value = Amount::from_sat(value);
    Some(UnsignedWatchtowerExitRecovery {
        tx: recovery_tx,
        fee_sat,
        vsize,
    })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::{Txid, hashes::Hash};
    use spark::tree::tests::create_test_node_with_parent;

    use crate::unilateral_exit::SpendInfo;

    use super::*;

    const LEAF: &str = "00000000-0000-0000-0000-00000000000a";
    const SPLIT_2: &str = "00000000-0000-0000-0000-00000000000b";
    const SPLIT_1: &str = "00000000-0000-0000-0000-00000000000c";
    const BRANCH: &str = "00000000-0000-0000-0000-00000000000d";

    fn parent_output(tag: u8) -> TxIn {
        TxIn {
            previous_output: OutPoint {
                txid: Txid::from_byte_array([tag; 32]),
                vout: 0,
            },
            sequence: Sequence::from_height(50),
            ..Default::default()
        }
    }

    fn spending(input: TxIn, outputs: Vec<TxOut>) -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![input],
            output: outputs,
        }
    }

    fn direct_tx(outputs: Vec<TxOut>, tag: u8) -> Transaction {
        spending(parent_output(tag), outputs)
    }

    fn paying(value: u64, script_pubkey: ScriptBuf) -> TxOut {
        TxOut {
            value: Amount::from_sat(value),
            script_pubkey,
        }
    }

    fn key_path_script(node: &TreeNode) -> ScriptBuf {
        ScriptBuf::new_p2tr(
            &Secp256k1::verification_only(),
            node.verifying_public_key.x_only_public_key().0,
            None,
        )
    }

    fn node(
        id: &str,
        parent: Option<&str>,
        status: TreeNodeStatus,
        direct: Option<Transaction>,
    ) -> TreeNode {
        let mut node = create_test_node_with_parent(id, parent, status);
        node.direct_tx = direct;
        node
    }

    fn by_id(nodes: Vec<TreeNode>) -> HashMap<TreeNodeId, TreeNode> {
        nodes.into_iter().map(|n| (n.id.clone(), n)).collect()
    }

    fn parent_output_spent_by(node: &TreeNode, spender: &Transaction) -> Vec<Observation> {
        let outpoint = node.direct_tx.as_ref().unwrap().input[0].previous_output;
        vec![
            Observation {
                query: ChainQuery::Outspend(outpoint),
                result: ChainResult::Spend(Some(SpendInfo {
                    spender_txid: spender.compute_txid(),
                    confirmed: true,
                    block_height: Some(100),
                })),
            },
            Observation {
                query: ChainQuery::Transaction(spender.compute_txid()),
                result: ChainResult::Transaction(spender.clone()),
            },
        ]
    }

    fn resolve(
        leaf: &TreeNode,
        nodes: &HashMap<TreeNodeId, TreeNode>,
        observed: &[Observation],
    ) -> ResolvedWatchtowerExits {
        let scan = scan_watchtower_exits(std::slice::from_ref(leaf), nodes, observed);
        assert!(scan.pending.is_empty(), "unanswered: {:?}", scan.pending);
        scan.resolved
    }

    fn found(output: WatchtowerExitedOutput) -> ResolvedWatchtowerExits {
        ResolvedWatchtowerExits {
            outputs: vec![output],
            ..Default::default()
        }
    }

    fn output_of(value: u64) -> WatchtowerExitedOutput {
        WatchtowerExitedOutput {
            leaf_id: TreeNodeId::from_str(LEAF).unwrap(),
            outpoint: OutPoint {
                txid: Txid::from_byte_array([9; 32]),
                vout: 0,
            },
            tx_out: paying(value, ScriptBuf::new()),
        }
    }

    fn regtest_address() -> Address {
        Address::from_str("bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080")
            .unwrap()
            .assume_checked()
    }

    #[test]
    fn the_output_is_the_one_paying_the_leaf_key() {
        let leaf = node(LEAF, Some(SPLIT_2), TreeNodeStatus::WatchtowerExited, None);
        let confirmed = direct_tx(
            vec![
                paying(100, ScriptBuf::from_bytes(vec![1])),
                paying(9_800, key_path_script(&leaf)),
            ],
            1,
        );
        let split_1 = node(
            SPLIT_1,
            Some(BRANCH),
            TreeNodeStatus::OnChain,
            Some(confirmed.clone()),
        );
        let observed = parent_output_spent_by(&split_1, &confirmed);
        let nodes = by_id(vec![
            leaf.clone(),
            node(
                SPLIT_2,
                Some(SPLIT_1),
                TreeNodeStatus::Splitted,
                Some(direct_tx(vec![paying(9_900, key_path_script(&leaf))], 2)),
            ),
            split_1,
            node(BRANCH, None, TreeNodeStatus::OnChain, None),
        ]);

        assert_eq!(
            resolve(&leaf, &nodes, &observed),
            found(WatchtowerExitedOutput {
                leaf_id: leaf.id.clone(),
                outpoint: OutPoint {
                    txid: confirmed.compute_txid(),
                    vout: 1,
                },
                tx_out: confirmed.output[1].clone(),
            })
        );
    }

    #[test]
    fn the_output_is_in_the_direct_tx_the_chain_shows() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let held = direct_tx(vec![paying(9_800, key_path_script(&leaf))], 1);
        let confirmed = spending(
            parent_output(1),
            vec![paying(9_300, key_path_script(&leaf))],
        );
        let split_1 = node(SPLIT_1, Some(BRANCH), TreeNodeStatus::OnChain, Some(held));
        let observed = parent_output_spent_by(&split_1, &confirmed);
        let nodes = by_id(vec![leaf.clone(), split_1]);

        assert_eq!(
            resolve(&leaf, &nodes, &observed),
            found(WatchtowerExitedOutput {
                leaf_id: leaf.id.clone(),
                outpoint: OutPoint {
                    txid: confirmed.compute_txid(),
                    vout: 0,
                },
                tx_out: confirmed.output[0].clone(),
            })
        );
    }

    #[test]
    fn a_spend_of_another_sequence_is_not_the_direct_tx() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let mut other_input = parent_output(1);
        other_input.sequence = Sequence::from_height(150);
        let other = spending(other_input, vec![paying(9_300, key_path_script(&leaf))]);
        let split_1 = node(
            SPLIT_1,
            Some(BRANCH),
            TreeNodeStatus::OnChain,
            Some(direct_tx(vec![paying(9_800, key_path_script(&leaf))], 1)),
        );
        let observed = parent_output_spent_by(&split_1, &other);
        let nodes = by_id(vec![leaf.clone(), split_1]);

        assert_eq!(
            resolve(&leaf, &nodes, &observed),
            ResolvedWatchtowerExits::default()
        );
    }

    #[test]
    fn the_chain_is_asked_one_step_at_a_time() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let confirmed = spending(
            parent_output(1),
            vec![paying(9_300, key_path_script(&leaf))],
        );
        let split_1 = node(
            SPLIT_1,
            Some(BRANCH),
            TreeNodeStatus::OnChain,
            Some(direct_tx(vec![paying(9_800, key_path_script(&leaf))], 1)),
        );
        let observed = parent_output_spent_by(&split_1, &confirmed);
        let nodes = by_id(vec![leaf.clone(), split_1]);
        let leaves = [leaf];

        let first = scan_watchtower_exits(&leaves, &nodes, &[]);
        assert_eq!(first.pending, vec![observed[0].query.clone()]);
        let second = scan_watchtower_exits(&leaves, &nodes, &observed[..1]);
        assert_eq!(second.pending, vec![observed[1].query.clone()]);
        let done = scan_watchtower_exits(&leaves, &nodes, &observed);
        assert!(done.pending.is_empty());
        assert_eq!(done.resolved.outputs.len(), 1);
    }

    #[test]
    fn a_leaf_holding_part_of_the_paid_key_is_unrecoverable() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let mut sibling = leaf.clone();
        sibling.verifying_public_key = bitcoin::secp256k1::SecretKey::from_slice(&[7; 32])
            .unwrap()
            .public_key(&Secp256k1::signing_only());
        let confirmed = direct_tx(vec![paying(9_800, key_path_script(&sibling))], 1);
        let split_1 = node(
            SPLIT_1,
            Some(BRANCH),
            TreeNodeStatus::OnChain,
            Some(confirmed.clone()),
        );
        let observed = parent_output_spent_by(&split_1, &confirmed);
        let nodes = by_id(vec![leaf.clone(), split_1]);

        assert_eq!(
            resolve(&leaf, &nodes, &observed),
            ResolvedWatchtowerExits {
                unrecoverable: vec![leaf.id.clone()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn an_on_chain_leaf_is_recovered_from_its_direct_tx_at_another_fee() {
        let mut leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::OnChain, None);
        leaf.direct_tx = Some(direct_tx(vec![paying(9_800, key_path_script(&leaf))], 3));
        let confirmed = spending(
            parent_output(3),
            vec![paying(9_500, key_path_script(&leaf))],
        );
        let observed = parent_output_spent_by(&leaf, &confirmed);
        let nodes = by_id(vec![
            leaf.clone(),
            node(
                SPLIT_1,
                None,
                TreeNodeStatus::OnChain,
                Some(direct_tx(vec![paying(9_900, key_path_script(&leaf))], 1)),
            ),
        ]);

        assert_eq!(
            resolve(&leaf, &nodes, &observed),
            found(WatchtowerExitedOutput {
                leaf_id: leaf.id.clone(),
                outpoint: OutPoint {
                    txid: confirmed.compute_txid(),
                    vout: 0,
                },
                tx_out: confirmed.output[0].clone(),
            })
        );
    }

    #[test]
    fn an_on_chain_leaf_its_refunds_recover_is_unilateral() {
        let mut leaf = node(LEAF, None, TreeNodeStatus::OnChain, None);
        let own_direct = direct_tx(vec![paying(9_800, key_path_script(&leaf))], 3);
        leaf.direct_tx = Some(own_direct.clone());
        let mut own_node_tx = own_direct.clone();
        own_node_tx.output.push(paying(0, ScriptBuf::new()));
        leaf.node_tx = own_node_tx.clone();

        for spender in [own_direct, own_node_tx] {
            assert_eq!(
                resolve(
                    &leaf,
                    &HashMap::new(),
                    &parent_output_spent_by(&leaf, &spender)
                ),
                ResolvedWatchtowerExits {
                    unilateral: vec![leaf.id.clone()],
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn a_recovered_leaf_finds_its_output_again() {
        let mut leaf = node(LEAF, None, TreeNodeStatus::WatchtowerExitRecovered, None);
        leaf.direct_tx = Some(direct_tx(vec![paying(9_800, key_path_script(&leaf))], 3));
        let confirmed = spending(
            parent_output(3),
            vec![paying(9_500, key_path_script(&leaf))],
        );
        let observed = parent_output_spent_by(&leaf, &confirmed);

        assert_eq!(resolve(&leaf, &HashMap::new(), &observed).outputs.len(), 1);
    }

    #[test]
    fn a_spend_the_chain_does_not_show_takes_the_held_direct_tx() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let held = direct_tx(vec![paying(9_800, key_path_script(&leaf))], 1);
        let split_1 = node(
            SPLIT_1,
            Some(BRANCH),
            TreeNodeStatus::OnChain,
            Some(held.clone()),
        );
        let outpoint = held.input[0].previous_output;
        let nodes = by_id(vec![
            leaf.clone(),
            split_1,
            node(
                BRANCH,
                None,
                TreeNodeStatus::OnChain,
                Some(direct_tx(vec![paying(9_900, key_path_script(&leaf))], 2)),
            ),
        ]);

        for result in [ChainResult::Spend(None), ChainResult::Unavailable] {
            let observed = [Observation {
                query: ChainQuery::Outspend(outpoint),
                result,
            }];
            assert_eq!(
                resolve(&leaf, &nodes, &observed),
                ResolvedWatchtowerExits {
                    assumed: vec![WatchtowerExitedOutput {
                        leaf_id: leaf.id.clone(),
                        outpoint: OutPoint {
                            txid: held.compute_txid(),
                            vout: 0,
                        },
                        tx_out: held.output[0].clone(),
                    }],
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn a_spender_that_cannot_be_read_gives_no_output() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let split_1 = node(
            SPLIT_1,
            None,
            TreeNodeStatus::OnChain,
            Some(direct_tx(vec![paying(9_800, key_path_script(&leaf))], 1)),
        );
        let spender = Txid::from_byte_array([8; 32]);
        let observed = [
            Observation {
                query: ChainQuery::Outspend(
                    split_1.direct_tx.as_ref().unwrap().input[0].previous_output,
                ),
                result: ChainResult::Spend(Some(SpendInfo {
                    spender_txid: spender,
                    confirmed: true,
                    block_height: Some(100),
                })),
            },
            Observation {
                query: ChainQuery::Transaction(spender),
                result: ChainResult::Unavailable,
            },
        ];
        let nodes = by_id(vec![leaf.clone(), split_1]);

        assert_eq!(
            resolve(&leaf, &nodes, &observed),
            ResolvedWatchtowerExits::default()
        );
    }

    #[test]
    fn an_on_chain_leaf_takes_no_held_direct_tx() {
        let mut leaf = node(LEAF, None, TreeNodeStatus::OnChain, None);
        leaf.direct_tx = Some(direct_tx(vec![paying(9_800, key_path_script(&leaf))], 3));
        let observed = [Observation {
            query: ChainQuery::Outspend(leaf.direct_tx.as_ref().unwrap().input[0].previous_output),
            result: ChainResult::Unavailable,
        }];

        assert_eq!(
            resolve(&leaf, &HashMap::new(), &observed),
            ResolvedWatchtowerExits::default()
        );
    }

    #[test]
    fn nodes_without_an_on_chain_ancestor_do_not_find_the_output() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let nodes = by_id(vec![
            leaf.clone(),
            node(
                SPLIT_1,
                None,
                TreeNodeStatus::Splitted,
                Some(direct_tx(vec![paying(9_800, key_path_script(&leaf))], 1)),
            ),
        ]);

        assert_eq!(
            resolve(&leaf, &nodes, &[]),
            ResolvedWatchtowerExits::default()
        );
    }

    #[test]
    fn a_missing_ancestor_does_not_find_the_output() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let nodes = by_id(vec![leaf.clone()]);

        assert_eq!(
            resolve(&leaf, &nodes, &[]),
            ResolvedWatchtowerExits::default()
        );
    }

    #[test]
    fn a_cycle_in_the_parent_ids_does_not_find_the_output() {
        let leaf = node(LEAF, Some(SPLIT_2), TreeNodeStatus::WatchtowerExited, None);
        let nodes = by_id(vec![
            leaf.clone(),
            node(
                SPLIT_2,
                Some(SPLIT_1),
                TreeNodeStatus::WatchtowerExited,
                None,
            ),
            node(
                SPLIT_1,
                Some(SPLIT_2),
                TreeNodeStatus::WatchtowerExited,
                None,
            ),
        ]);

        assert_eq!(
            resolve(&leaf, &nodes, &[]),
            ResolvedWatchtowerExits::default()
        );
    }

    #[test]
    fn the_recovery_pays_the_value_less_the_fee_in_a_replaceable_version_2_tx() {
        let output = output_of(10_000);
        let destination = regtest_address();

        let recovery =
            build_watchtower_exit_recovery(&output, &destination, Fee::Fixed { amount: 500 })
                .unwrap();

        let tx = &recovery.tx;
        assert_eq!(tx.version, Version::TWO);
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.input[0].previous_output, output.outpoint);
        assert_eq!(tx.input[0].sequence, Sequence::ENABLE_RBF_NO_LOCKTIME);
        assert!(tx.input[0].witness.is_empty());
        assert_eq!(tx.output.len(), 1);
        assert_eq!(tx.output[0].value, Amount::from_sat(9_500));
        assert_eq!(tx.output[0].script_pubkey, destination.script_pubkey());
        assert_eq!(recovery.fee_sat, 500);
    }

    #[test]
    fn a_rate_is_charged_on_the_signed_size() {
        let output = output_of(10_000);
        let destination = regtest_address();

        let recovery =
            build_watchtower_exit_recovery(&output, &destination, Fee::Rate { sat_per_vbyte: 2 })
                .unwrap();

        let mut signed = recovery.tx.clone();
        signed.input[0].witness = Witness::from_slice(&[[0u8; 64]]);
        assert_eq!(recovery.vsize, signed.vsize() as u64);
        assert_eq!(recovery.fee_sat, 2 * recovery.vsize);
        assert_eq!(
            recovery.tx.output[0].value.to_sat(),
            10_000 - recovery.fee_sat
        );
    }

    #[test]
    fn a_zero_fee_pays_out_the_whole_output() {
        let recovery = build_watchtower_exit_recovery(
            &output_of(10_000),
            &regtest_address(),
            Fee::Rate { sat_per_vbyte: 0 },
        )
        .unwrap();

        assert_eq!(recovery.fee_sat, 0);
        assert_eq!(recovery.tx.output[0].value, Amount::from_sat(10_000));
    }

    #[test]
    fn a_fee_leaving_dust_is_built_as_asked() {
        let recovery = build_watchtower_exit_recovery(
            &output_of(1_000),
            &regtest_address(),
            Fee::Fixed { amount: 999 },
        )
        .unwrap();

        assert_eq!(recovery.tx.output[0].value, Amount::from_sat(1));
    }

    #[test]
    fn a_fee_taking_the_whole_output_builds_nothing() {
        let recovery = build_watchtower_exit_recovery(
            &output_of(1_000),
            &regtest_address(),
            Fee::Fixed { amount: 1_000 },
        );

        assert_eq!(recovery, None);
    }

    #[test]
    fn a_fee_above_the_value_builds_nothing() {
        let recovery = build_watchtower_exit_recovery(
            &output_of(1_000),
            &regtest_address(),
            Fee::Fixed { amount: 2_000 },
        );

        assert_eq!(recovery, None);
    }
}
