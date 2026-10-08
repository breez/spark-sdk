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

/// The on-chain output holding a leaf's funds after a watchtower exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchtowerExitOutput {
    pub leaf_id: TreeNodeId,
    pub outpoint: OutPoint,
    pub tx_out: TxOut,
}

/// Where a leaf's funds are after a watchtower exit, read from the results of
/// a scan's queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchtowerExitLookup {
    /// In a direct tx that is in a block.
    Found {
        output: WatchtowerExitOutput,
        /// Unset when the result does not name the block.
        block_height: Option<u32>,
    },
    /// In the direct tx the nearest on-chain ancestor holds. No result has a
    /// direct tx of that ancestor in a block.
    Unconfirmed(WatchtowerExitOutput),
    /// The direct tx has no output to the leaf's key, so the operators co-sign
    /// no recovery of the leaf.
    Unrecoverable,
    /// The leaf's own refunds recover it.
    Unilateral,
    NotFound,
    /// A query the lookup needs has no result.
    Pending,
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
    By(&'a Transaction, Option<u32>),
    /// By a transaction in a block, for which the chain service returned something else.
    Unread,
    /// The query for the output's spend has no result.
    OutspendUnknown,
    /// The query for the transaction that spent the output has no result.
    SpenderUnknown,
}

pub struct WatchtowerExitScan {
    pub lookups: HashMap<TreeNodeId, WatchtowerExitLookup>,
    /// The queries the caller has yet to execute.
    pub pending: Vec<ChainQuery>,
}

/// Looks up the on-chain output holding the funds of each of `leaves`. `nodes`
/// holds their ancestors, which an on-chain leaf does not need. Run it again
/// with the results of `pending` until none is left.
pub fn scan_watchtower_exits(
    leaves: &[TreeNode],
    nodes: &HashMap<TreeNodeId, TreeNode>,
    observed: &[Observation],
) -> WatchtowerExitScan {
    let index = ObservedIndex::new(observed);
    let mut pending = Vec::new();
    let mut lookups = HashMap::new();
    for leaf in leaves {
        let executed = pending.len();
        let lookup = lookup_output(leaf, nodes, &index, &mut pending);
        let lookup = if pending.len() > executed {
            WatchtowerExitLookup::Pending
        } else {
            lookup
        };
        lookups.insert(leaf.id.clone(), lookup);
    }
    let mut seen = HashSet::new();
    pending.retain(|query| seen.insert(query.clone()));
    WatchtowerExitScan { lookups, pending }
}

fn lookup_output(
    leaf: &TreeNode,
    nodes: &HashMap<TreeNodeId, TreeNode>,
    observed: &ObservedIndex<'_>,
    pending: &mut Vec<ChainQuery>,
) -> WatchtowerExitLookup {
    let mut visited = HashSet::new();
    let mut node = nodes.get(&leaf.id).unwrap_or(leaf);
    // Whether the spend of the leaf's own parent output has no result.
    let mut own_spend_unknown = false;
    loop {
        // The operators supply the parent ids, so a cycle is possible.
        if !visited.insert(node.id.clone()) {
            return WatchtowerExitLookup::NotFound;
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
                Spend::By(spender, block_height) => {
                    let held = spender.compute_txid() == direct_tx.compute_txid();
                    return match Spender::of(spender, node, direct_tx) {
                        Spender::DirectTx if !(is_leaf && held) => {
                            output_paying_leaf(leaf, spender).map_or(
                                WatchtowerExitLookup::Unrecoverable,
                                |output| WatchtowerExitLookup::Found {
                                    output,
                                    block_height,
                                },
                            )
                        }
                        // A leaf's refunds have the output of its node tx as
                        // input, and its direct refund that of the direct tx it
                        // holds. A leaf renewed at a zero timelock has no direct
                        // refund.
                        Spender::NodeTx | Spender::DirectTx if is_leaf && on_chain => {
                            WatchtowerExitLookup::Unilateral
                        }
                        _ => WatchtowerExitLookup::NotFound,
                    };
                }
                Spend::Unread => return WatchtowerExitLookup::NotFound,
                Spend::SpenderUnknown => return WatchtowerExitLookup::Pending,
                Spend::OutspendUnknown if is_leaf => own_spend_unknown = true,
                Spend::NotShown | Spend::OutspendUnknown if on_chain && !is_leaf => {
                    // The leaf's own direct tx may be the one in a block.
                    if own_spend_unknown {
                        return WatchtowerExitLookup::Pending;
                    }
                    return output_paying_leaf(leaf, direct_tx).map_or(
                        WatchtowerExitLookup::Unrecoverable,
                        WatchtowerExitLookup::Unconfirmed,
                    );
                }
                Spend::NotShown | Spend::OutspendUnknown => {}
            }
        }
        let not_found = if own_spend_unknown {
            WatchtowerExitLookup::Pending
        } else {
            WatchtowerExitLookup::NotFound
        };
        // The ancestors of an on-chain node confirmed through their node txs, so
        // none of them holds the funds.
        if on_chain {
            return not_found;
        }
        let Some(parent) = node.parent_node_id.as_ref().and_then(|id| nodes.get(id)) else {
            return not_found;
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
    let (txid, block_height) = match observed.get(&query) {
        None => {
            pending.push(query);
            return Spend::OutspendUnknown;
        }
        Some(ChainResult::Unavailable) => return Spend::OutspendUnknown,
        Some(ChainResult::Spend(Some(spend))) if spend.confirmed => {
            (spend.spender_txid, spend.block_height)
        }
        Some(_) => return Spend::NotShown,
    };
    for held in [&node.node_tx, direct_tx] {
        if held.compute_txid() == txid {
            return Spend::By(held, block_height);
        }
    }
    let query = ChainQuery::Transaction(txid);
    match observed.get(&query) {
        None => {
            pending.push(query);
            Spend::SpenderUnknown
        }
        Some(ChainResult::Unavailable) => Spend::SpenderUnknown,
        Some(ChainResult::Transaction(tx)) if tx.compute_txid() == txid => {
            Spend::By(tx, block_height)
        }
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

fn output_paying_leaf(leaf: &TreeNode, direct_tx: &Transaction) -> Option<WatchtowerExitOutput> {
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
    Some(WatchtowerExitOutput {
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
    output: &WatchtowerExitOutput,
    destination: &Address,
    fee: Fee,
) -> Option<UnsignedWatchtowerExitRecovery> {
    let mut recovery_tx = Transaction {
        // The version of the recovery the operators' own SDK builds.
        version: Version::non_standard(3),
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
    ) -> WatchtowerExitLookup {
        let mut scan = scan_watchtower_exits(std::slice::from_ref(leaf), nodes, observed);
        assert!(scan.pending.is_empty(), "not executed: {:?}", scan.pending);
        scan.lookups.remove(&leaf.id).unwrap()
    }

    fn found(output: WatchtowerExitOutput) -> WatchtowerExitLookup {
        WatchtowerExitLookup::Found {
            output,
            block_height: Some(100),
        }
    }

    fn output_of(value: u64) -> WatchtowerExitOutput {
        WatchtowerExitOutput {
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
            found(WatchtowerExitOutput {
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
            found(WatchtowerExitOutput {
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
            WatchtowerExitLookup::NotFound
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

        let lookup = |scan: &WatchtowerExitScan| scan.lookups[&leaves[0].id].clone();

        let first = scan_watchtower_exits(&leaves, &nodes, &[]);
        assert_eq!(first.pending, vec![observed[0].query.clone()]);
        assert_eq!(lookup(&first), WatchtowerExitLookup::Pending);
        let second = scan_watchtower_exits(&leaves, &nodes, &observed[..1]);
        assert_eq!(second.pending, vec![observed[1].query.clone()]);
        assert_eq!(lookup(&second), WatchtowerExitLookup::Pending);
        let done = scan_watchtower_exits(&leaves, &nodes, &observed);
        assert!(done.pending.is_empty());
        assert!(matches!(lookup(&done), WatchtowerExitLookup::Found { .. }));
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
            WatchtowerExitLookup::Unrecoverable
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
            found(WatchtowerExitOutput {
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
                WatchtowerExitLookup::Unilateral
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

        assert!(matches!(
            resolve(&leaf, &HashMap::new(), &observed),
            WatchtowerExitLookup::Found { .. }
        ));
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
                WatchtowerExitLookup::Unconfirmed(WatchtowerExitOutput {
                    leaf_id: leaf.id.clone(),
                    outpoint: OutPoint {
                        txid: held.compute_txid(),
                        vout: 0,
                    },
                    tx_out: held.output[0].clone(),
                })
            );
        }
    }

    #[test]
    fn a_spender_the_chain_does_not_return_leaves_the_lookup_pending() {
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
            WatchtowerExitLookup::Pending
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
            WatchtowerExitLookup::Pending
        );
    }

    #[test]
    fn a_recovered_leaf_without_its_own_result_takes_no_held_direct_tx() {
        let mut leaf = node(
            LEAF,
            Some(SPLIT_1),
            TreeNodeStatus::WatchtowerExitRecovered,
            None,
        );
        leaf.direct_tx = Some(direct_tx(vec![paying(9_800, key_path_script(&leaf))], 3));
        let split_1 = node(
            SPLIT_1,
            None,
            TreeNodeStatus::OnChain,
            Some(direct_tx(vec![paying(9_900, key_path_script(&leaf))], 1)),
        );
        let outspend = |node: &TreeNode, result| Observation {
            query: ChainQuery::Outspend(node.direct_tx.as_ref().unwrap().input[0].previous_output),
            result,
        };
        let nodes = by_id(vec![leaf.clone(), split_1.clone()]);

        // The leaf's own direct tx may be the one in a block.
        let observed = [
            outspend(&leaf, ChainResult::Unavailable),
            outspend(&split_1, ChainResult::Spend(None)),
        ];
        assert_eq!(
            resolve(&leaf, &nodes, &observed),
            WatchtowerExitLookup::Pending
        );

        let observed = [
            outspend(&leaf, ChainResult::Spend(None)),
            outspend(&split_1, ChainResult::Unavailable),
        ];
        assert!(matches!(
            resolve(&leaf, &nodes, &observed),
            WatchtowerExitLookup::Unconfirmed(_)
        ));
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

        assert_eq!(resolve(&leaf, &nodes, &[]), WatchtowerExitLookup::NotFound);
    }

    #[test]
    fn a_missing_ancestor_does_not_find_the_output() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let nodes = by_id(vec![leaf.clone()]);

        assert_eq!(resolve(&leaf, &nodes, &[]), WatchtowerExitLookup::NotFound);
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

        assert_eq!(resolve(&leaf, &nodes, &[]), WatchtowerExitLookup::NotFound);
    }

    #[test]
    fn the_recovery_pays_the_value_less_the_fee_in_a_replaceable_version_3_tx() {
        let output = output_of(10_000);
        let destination = regtest_address();

        let recovery =
            build_watchtower_exit_recovery(&output, &destination, Fee::Fixed { amount: 500 })
                .unwrap();

        let tx = &recovery.tx;
        assert_eq!(tx.version, Version::non_standard(3));
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
