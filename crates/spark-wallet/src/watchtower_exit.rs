use std::collections::HashMap;

use bitcoin::{
    Address, Amount, OutPoint, Sequence, Transaction, TxIn, TxOut, Witness, absolute::LockTime,
    transaction::Version,
};
use spark::{
    services::{Fee, MIN_RELAY_FEE_SAT_PER_VBYTE},
    tree::{TreeNode, TreeNodeId, TreeNodeStatus},
};

use crate::SparkWalletError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchtowerExitedOutput {
    pub leaf_id: TreeNodeId,
    /// Before the watchtower's transaction took its fee.
    pub leaf_value: u64,
    pub outpoint: OutPoint,
    pub tx_out: TxOut,
}

pub(crate) fn is_watchtower_exited(status: TreeNodeStatus) -> bool {
    matches!(
        status,
        TreeNodeStatus::WatchtowerExited | TreeNodeStatus::WatchtowerExitRecovered
    )
}

/// The operators mark every node below the confirmed split node as
/// watchtower-exited, so the walk passes over those.
pub(crate) fn direct_split_node_output(
    leaf: &TreeNode,
    nodes: &HashMap<TreeNodeId, TreeNode>,
) -> Option<WatchtowerExitedOutput> {
    let mut parent_id = nodes.get(&leaf.id).unwrap_or(leaf).parent_node_id.clone();
    while let Some(id) = parent_id {
        let node = nodes.get(&id)?;
        if is_watchtower_exited(node.status) {
            parent_id = node.parent_node_id.clone();
            continue;
        }
        if node.status != TreeNodeStatus::OnChain {
            return None;
        }
        let direct_tx = node.direct_tx.as_ref()?;
        return Some(WatchtowerExitedOutput {
            leaf_id: leaf.id.clone(),
            leaf_value: leaf.value,
            outpoint: OutPoint {
                txid: direct_tx.compute_txid(),
                vout: 0,
            },
            tx_out: direct_tx.output.first()?.clone(),
        });
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsignedWatchtowerExitRecovery {
    pub tx: Transaction,
    pub fee_sat: u64,
    /// The size the transaction has once signed.
    pub vsize: u64,
}

/// `None` when the fee leaves the destination less than its dust limit.
pub fn build_watchtower_exit_recovery(
    output: &WatchtowerExitedOutput,
    destination: &Address,
    fee: Fee,
) -> Result<Option<UnsignedWatchtowerExitRecovery>, SparkWalletError> {
    let script_pubkey = destination.script_pubkey();
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
            script_pubkey: script_pubkey.clone(),
        }],
    };
    let vsize = recovery_tx.vsize() as u64;
    recovery_tx.input[0].witness = Witness::new();

    let fee_sat = fee.to_sats(vsize);
    let min_fee_sat = vsize * MIN_RELAY_FEE_SAT_PER_VBYTE;
    if fee_sat < min_fee_sat {
        return Err(SparkWalletError::ValidationError(format!(
            "fee must be at least {min_fee_sat} sats ({MIN_RELAY_FEE_SAT_PER_VBYTE} sat/vB over {vsize} vbytes)"
        )));
    }
    let Some(value) = output.tx_out.value.to_sat().checked_sub(fee_sat) else {
        return Ok(None);
    };
    if Amount::from_sat(value) < script_pubkey.minimal_non_dust() {
        return Ok(None);
    }
    recovery_tx.output[0].value = Amount::from_sat(value);
    Ok(Some(UnsignedWatchtowerExitRecovery {
        tx: recovery_tx,
        fee_sat,
        vsize,
    }))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::{ScriptBuf, Txid, hashes::Hash};
    use spark::tree::tests::create_test_node_with_parent;

    use super::*;

    const LEAF: &str = "00000000-0000-0000-0000-00000000000a";
    const SPLIT_2: &str = "00000000-0000-0000-0000-00000000000b";
    const SPLIT_1: &str = "00000000-0000-0000-0000-00000000000c";
    const BRANCH: &str = "00000000-0000-0000-0000-00000000000d";

    fn direct_tx(value: u64, tag: u8) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: Txid::from_byte_array([tag; 32]),
                    vout: 0,
                },
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::from_bytes(vec![tag]),
            }],
        }
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

    fn output_of(value: u64) -> WatchtowerExitedOutput {
        WatchtowerExitedOutput {
            leaf_id: TreeNodeId::from_str(LEAF).unwrap(),
            leaf_value: value,
            outpoint: OutPoint {
                txid: Txid::from_byte_array([9; 32]),
                vout: 0,
            },
            tx_out: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::new(),
            },
        }
    }

    fn regtest_address() -> Address {
        Address::from_str("bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080")
            .unwrap()
            .assume_checked()
    }

    #[test]
    fn the_output_is_the_confirmed_direct_split_node_tx() {
        let confirmed = direct_tx(9_800, 1);
        let leaf = node(LEAF, Some(SPLIT_2), TreeNodeStatus::WatchtowerExited, None);
        let nodes = by_id(vec![
            leaf.clone(),
            node(
                SPLIT_2,
                Some(SPLIT_1),
                TreeNodeStatus::WatchtowerExited,
                Some(direct_tx(9_900, 2)),
            ),
            node(
                SPLIT_1,
                Some(BRANCH),
                TreeNodeStatus::OnChain,
                Some(confirmed.clone()),
            ),
            node(BRANCH, None, TreeNodeStatus::OnChain, None),
        ]);

        let output = direct_split_node_output(&leaf, &nodes).unwrap();

        assert_eq!(output.outpoint.txid, confirmed.compute_txid());
        assert_eq!(output.outpoint.vout, 0);
        assert_eq!(output.tx_out, confirmed.output[0]);
        assert_eq!(output.leaf_value, leaf.value);
    }

    #[test]
    fn an_ancestor_that_is_neither_exited_nor_on_chain_resolves_nothing() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let nodes = by_id(vec![
            leaf.clone(),
            node(
                SPLIT_1,
                Some(BRANCH),
                TreeNodeStatus::SplitLocked,
                Some(direct_tx(9_800, 1)),
            ),
        ]);

        assert_eq!(direct_split_node_output(&leaf, &nodes), None);
    }

    #[test]
    fn a_missing_ancestor_resolves_nothing() {
        let leaf = node(LEAF, Some(SPLIT_1), TreeNodeStatus::WatchtowerExited, None);
        let nodes = by_id(vec![leaf.clone()]);

        assert_eq!(direct_split_node_output(&leaf, &nodes), None);
    }

    #[test]
    fn the_recovery_pays_the_value_less_the_fee_in_a_version_2_tx() {
        let output = output_of(10_000);
        let destination = regtest_address();

        let recovery =
            build_watchtower_exit_recovery(&output, &destination, Fee::Fixed { amount: 500 })
                .unwrap()
                .unwrap();

        let tx = &recovery.tx;
        assert_eq!(tx.version, Version::TWO);
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.input[0].previous_output, output.outpoint);
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
                .unwrap()
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
    fn a_fee_below_the_relay_minimum_is_rejected() {
        let result = build_watchtower_exit_recovery(
            &output_of(10_000),
            &regtest_address(),
            Fee::Fixed { amount: 1 },
        );

        assert!(matches!(result, Err(SparkWalletError::ValidationError(_))));
    }

    #[test]
    fn a_fee_leaving_dust_builds_nothing() {
        let result = build_watchtower_exit_recovery(
            &output_of(1_000),
            &regtest_address(),
            Fee::Fixed { amount: 900 },
        );

        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn a_fee_above_the_value_builds_nothing() {
        let result = build_watchtower_exit_recovery(
            &output_of(1_000),
            &regtest_address(),
            Fee::Fixed { amount: 2_000 },
        );

        assert_eq!(result.unwrap(), None);
    }
}
