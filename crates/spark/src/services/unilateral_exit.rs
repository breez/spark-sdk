use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bitcoin::{
    Amount, OutPoint, Psbt, Sequence, Transaction, TxIn, TxOut, absolute::LockTime, psbt,
    transaction::Version,
};
use tracing::trace;

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            SparkRpcClient,
            spark::{QueryNodesRequest, TreeNodeIds, query_nodes_request::Source},
        },
    },
    services::ServiceError,
    tree::{TreeNode, TreeNodeId},
    utils::{
        paging::{PagingFilter, PagingResult, pager},
        transactions::is_ephemeral_anchor_output,
    },
};

/// A UTXO input for CPFP fee-bumping.
///
/// The caller provides the full `witness_utxo` (value + scriptPubKey) and the expected
/// signed input weight. Change outputs reuse `witness_utxo.script_pubkey`.
#[derive(Clone)]
pub struct CpfpInput {
    pub outpoint: OutPoint,
    pub witness_utxo: TxOut,
    pub signed_input_weight: u64,
}

pub struct TxCpfpPsbt {
    pub node_id: TreeNodeId,
    pub parent_tx: Transaction,
    /// `Some(psbt)` when the caller must broadcast the CPFP package to progress
    /// this step; `None` when the corresponding on-chain output has already been
    /// spent by some spending transaction (CPFP variant, direct variant, or a
    /// future protocol addition).
    pub child_psbt: Option<Psbt>,
    /// Outpoints the chain-level walk tracks for this step. If any of them is
    /// already spent on-chain, the step is considered done and `child_psbt` is
    /// `None`. Node steps carry a single outpoint (the parent's consumed vout);
    /// the leaf refund step may carry a second entry for `direct_tx.output[0]`
    /// when `direct_tx` is present.
    pub spent_outpoints: Vec<OutPoint>,
    /// Protocol-known alternative spending transactions for the outpoints in
    /// `spent_outpoints` (e.g. `direct_tx`, `direct_from_cpfp_refund_tx`,
    /// `direct_refund_tx`). Callers use this as a hydration cache to avoid
    /// fetching transactions whose bytes are already known.
    pub known_spenders: Vec<Transaction>,
}

pub struct LeafTxCpfpPsbts {
    pub leaf_id: TreeNodeId,
    pub tx_cpfp_psbts: Vec<TxCpfpPsbt>,
}

pub struct UnilateralExitService {
    operator_pool: Arc<OperatorPool>,
    network: Network,
}

impl UnilateralExitService {
    pub fn new(operator_pool: Arc<OperatorPool>, network: Network) -> Self {
        UnilateralExitService {
            operator_pool,
            network,
        }
    }

    /// Fetches all ancestors for the given leaves, selects the profitable ones,
    /// and builds their CPFP PSBTs.
    ///
    /// Returns the selected leaves and the CPFP transaction data. Returns an
    /// empty vec pair when no leaves are profitable.
    pub async fn unilateral_exit_autoselect(
        &self,
        fee_rate: u64,
        leaf_ids: Vec<TreeNodeId>,
        inputs: Vec<CpfpInput>,
        destination_script_len: usize,
    ) -> Result<(Vec<SelectedLeaf>, Vec<LeafTxCpfpPsbts>, Vec<TreeNode>), ServiceError> {
        if inputs.is_empty() {
            return Err(ServiceError::ValidationError(
                "At least one CPFP input is required".to_string(),
            ));
        }
        if leaf_ids.is_empty() {
            return Ok((vec![], vec![], vec![]));
        }

        let tree_nodes_vec = self.fetch_leaves_parents(&leaf_ids).await?;
        let tree_nodes: HashMap<TreeNodeId, TreeNode> = tree_nodes_vec
            .into_iter()
            .map(|n| (n.id.clone(), n))
            .collect();

        let change_script = &inputs[0].witness_utxo.script_pubkey;
        let params = LeafExitCostParams {
            initial_cpfp_input_weight: inputs.iter().map(|i| i.signed_input_weight).sum(),
            single_cpfp_input_weight: inputs[0].signed_input_weight,
            change_script_len: change_script.len(),
            change_dust_limit: change_script.minimal_non_dust().to_sat(),
            total_cpfp_budget: inputs.iter().map(|i| i.witness_utxo.value.to_sat()).sum(),
            destination_script_len,
            fee_rate,
        };

        let selected = select_profitable_leaves(&tree_nodes, &leaf_ids, &params)?;
        if selected.is_empty() {
            return Ok((vec![], vec![], vec![]));
        }

        let selected_ids: Vec<TreeNodeId> = selected.iter().map(|s| s.id.clone()).collect();
        let prefetched: Vec<TreeNode> = tree_nodes.into_values().collect();
        let psbts = self
            .unilateral_exit(
                fee_rate,
                selected_ids,
                inputs,
                Some(prefetched.clone()),
                &HashSet::new(),
            )
            .await?;

        Ok((selected, psbts, prefetched))
    }

    pub async fn unilateral_exit(
        &self,
        fee_rate: u64,
        leaf_ids: Vec<TreeNodeId>,
        mut inputs: Vec<CpfpInput>,
        prefetched_nodes: Option<Vec<TreeNode>>,
        confirmed_outpoints: &HashSet<OutPoint>,
    ) -> Result<Vec<LeafTxCpfpPsbts>, ServiceError> {
        if leaf_ids.is_empty() {
            return Err(ServiceError::ValidationError(
                "At least one leaf ID is required".to_string(),
            ));
        }
        if inputs.is_empty() {
            return Err(ServiceError::ValidationError(
                "At least one CPFP input is required".to_string(),
            ));
        }

        let mut all_leaf_tx_cpfp_psbts = Vec::new();
        let mut checked_outpoints: HashSet<OutPoint> = HashSet::new();

        let tree_nodes: HashMap<TreeNodeId, TreeNode> = match prefetched_nodes {
            Some(nodes) => nodes.into_iter().map(|n| (n.id.clone(), n)).collect(),
            None => self
                .fetch_leaves_parents(&leaf_ids)
                .await?
                .into_iter()
                .map(|node| (node.id.clone(), node))
                .collect(),
        };
        for leaf_id in leaf_ids {
            let mut tx_cpfp_psbts = Vec::new();
            let mut nodes = Vec::new();

            let Some(mut node) = tree_nodes.get(&leaf_id) else {
                return Err(ServiceError::ValidationError(format!(
                    "Leaf ID {leaf_id} not found in the tree",
                )));
            };
            let Some(refund_tx) = &node.refund_tx else {
                return Err(ServiceError::ValidationError(format!(
                    "Leaf ID {leaf_id} does not have a refund transaction",
                )));
            };

            // Loop through the leaf's ancestors and collect them
            loop {
                nodes.insert(0, node);

                let Some(parent_node_id) = &node.parent_node_id else {
                    break;
                };
                let Some(parent) = tree_nodes.get(parent_node_id) else {
                    return Err(ServiceError::ValidationError(format!(
                        "Parent ID {parent_node_id} not found in the tree",
                    )));
                };
                trace!(
                    "Unilateral exit parent {}, txid {}",
                    parent.id,
                    parent.node_tx.compute_txid()
                );
                node = parent;
            }

            // Emit an entry per node (the CPFP node_tx step) plus, for the leaf,
            // a refund entry. Each entry is emitted unconditionally: `child_psbt`
            // becomes `None` when any of its tracked outpoints is already spent
            // on-chain. Node entries share the parent vout with the `direct_tx`
            // variant, so one entry covers both possible spenders. The leaf
            // refund has two candidate consumed outpoints when `direct_tx`
            // exists (`node_tx.output[0]` for the CPFP / direct-from-cpfp
            // refund paths, and `direct_tx.output[0]` for the direct-refund
            // path); a single refund entry carries both so whichever actually
            // materialized drives the decision.
            for node in nodes {
                let node_parent_outpoint = node.node_tx.input[0].previous_output;
                if !checked_outpoints.insert(node_parent_outpoint) {
                    continue;
                }

                let child_psbt = if confirmed_outpoints.contains(&node_parent_outpoint) {
                    None
                } else {
                    Some(create_tx_cpfp_psbt(&node.node_tx, &mut inputs, fee_rate)?)
                };
                let known_spenders: Vec<Transaction> = node.direct_tx.clone().into_iter().collect();

                tx_cpfp_psbts.push(TxCpfpPsbt {
                    node_id: node.id.clone(),
                    parent_tx: node.node_tx.clone(),
                    child_psbt,
                    spent_outpoints: vec![node_parent_outpoint],
                    known_spenders,
                });

                if node.id == leaf_id {
                    let cpfp_refund_outpoint = refund_tx.input[0].previous_output;
                    let mut spent_outpoints = vec![cpfp_refund_outpoint];
                    if let Some(direct_tx) = &node.direct_tx {
                        spent_outpoints.push(OutPoint {
                            txid: direct_tx.compute_txid(),
                            vout: 0,
                        });
                    }

                    let any_confirmed = spent_outpoints
                        .iter()
                        .any(|op| confirmed_outpoints.contains(op));
                    let child_psbt = if any_confirmed {
                        None
                    } else {
                        Some(create_tx_cpfp_psbt(refund_tx, &mut inputs, fee_rate)?)
                    };

                    let mut known_spenders: Vec<Transaction> = Vec::new();
                    if let Some(tx) = &node.direct_from_cpfp_refund_tx {
                        known_spenders.push(tx.clone());
                    }
                    if let Some(tx) = &node.direct_refund_tx {
                        known_spenders.push(tx.clone());
                    }

                    tx_cpfp_psbts.push(TxCpfpPsbt {
                        node_id: node.id.clone(),
                        parent_tx: refund_tx.clone(),
                        child_psbt,
                        spent_outpoints,
                        known_spenders,
                    });
                }
            }

            all_leaf_tx_cpfp_psbts.push(LeafTxCpfpPsbts {
                leaf_id,
                tx_cpfp_psbts,
            });
        }

        Ok(all_leaf_tx_cpfp_psbts)
    }

    async fn fetch_leaves_parents(
        &self,
        leaf_ids: &[TreeNodeId],
    ) -> Result<Vec<TreeNode>, ServiceError> {
        if leaf_ids.is_empty() {
            return Ok(Vec::new());
        }

        let client = &self.operator_pool.get_coordinator().client;
        let nodes = pager(
            |f| self.fetch_leaves_parents_inner(client, leaf_ids, f),
            PagingFilter::default(),
        )
        .await?;

        Ok(nodes.items)
    }

    async fn fetch_leaves_parents_inner(
        &self,
        client: &SparkRpcClient,
        leaf_ids: &[TreeNodeId],
        paging: PagingFilter,
    ) -> Result<PagingResult<TreeNode>, ServiceError> {
        trace!(
            "Fetching leaves parents with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let source = Source::NodeIds(TreeNodeIds {
            node_ids: leaf_ids.iter().map(|id| id.to_string()).collect(),
        });
        let nodes = client
            .query_nodes(QueryNodesRequest {
                include_parents: true,
                limit: paging.limit as i64,
                offset: paging.offset as i64,
                network: self.network.to_proto_network().into(),
                source: Some(source),
                statuses: vec![],
            })
            .await?;
        Ok(PagingResult {
            items: nodes
                .nodes
                .into_values()
                .map(TreeNode::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    ServiceError::Generic(format!("Failed to deserialize leaves: {e:?}"))
                })?,
            next: paging.next_from_offset(nodes.offset),
        })
    }
}

/// Computes the CPFP package fee for a parent-child transaction pair.
///
/// The child transaction pays for both itself and the parent (which carries no fee
/// via its ephemeral anchor). Returns the fee in satoshis.
///
/// # Arguments
/// * `parent_weight_wu` - Weight of the parent transaction in weight units
/// * `cpfp_input_weight_wu` - Total signed input weight of the CPFP inputs in weight units
/// * `change_script_len` - Length of the change output's scriptPubKey in bytes
/// * `fee_rate` - Fee rate in satoshis per virtual byte
pub fn compute_cpfp_package_fee(
    parent_weight_wu: u64,
    cpfp_input_weight_wu: u64,
    change_script_len: usize,
    fee_rate: u64,
) -> u64 {
    // Anchor input: 41 non-witness × 4 + 1 witness = 165 WU
    let anchor_weight: u64 = 165;
    // Output: (value(8) + scriptPubKey_len(1) + scriptPubKey(N)) × 4
    let output_weight: u64 = (9 + change_script_len as u64) * 4;
    // Overhead: (version(4) + input_count(1) + output_count(1) + locktime(4)) × 4 + marker(1) + flag(1) = 42
    let overhead_weight: u64 = 42;
    let child_weight = cpfp_input_weight_wu + anchor_weight + output_weight + overhead_weight;
    let package_weight = parent_weight_wu + child_weight;
    (fee_rate * package_weight).div_ceil(4)
}

/// Computes the fee for a sweep transaction spending P2TR refund outputs.
///
/// # Arguments
/// * `num_inputs` - Number of P2TR key-path inputs (one per exited leaf)
/// * `destination_script_len` - Length of the destination output's scriptPubKey in bytes
/// * `fee_rate` - Fee rate in satoshis per virtual byte
pub fn compute_sweep_fee(num_inputs: usize, destination_script_len: usize, fee_rate: u64) -> u64 {
    // P2TR key-path input: 41 non-witness × 4 + 66 witness = 230 WU
    let input_weight = num_inputs as u64 * 230;
    // Output: (value(8) + scriptPubKey_len(1) + scriptPubKey(N)) × 4
    let output_weight = (9 + destination_script_len as u64) * 4;
    // Overhead: 42 WU (same as CPFP)
    let total_weight = 42 + input_weight + output_weight;
    (fee_rate * total_weight).div_ceil(4)
}

/// A leaf selected by the greedy profitability algorithm.
#[derive(Debug, Clone)]
pub struct SelectedLeaf {
    pub id: TreeNodeId,
    pub value: u64,
    /// Estimated marginal exit cost (CPFP fees + sweep input fee).
    pub estimated_cost: u64,
}

/// Parameters for the greedy leaf-selection algorithm.
pub struct LeafExitCostParams {
    /// Total signed weight of all initial CPFP inputs (used for the very first
    /// CPFP child transaction in the chain).
    pub initial_cpfp_input_weight: u64,
    /// Signed weight of a single CPFP change-output input (used for every
    /// subsequent CPFP child transaction in the chain).
    pub single_cpfp_input_weight: u64,
    /// Length in bytes of the change scriptPubKey carried through the CPFP chain.
    pub change_script_len: usize,
    /// Dust limit in satoshis for the change scriptPubKey. The last CPFP in the
    /// chain must leave at least this much change, otherwise the transaction is
    /// non-standard.
    pub change_dust_limit: u64,
    /// Total satoshi value available across all CPFP inputs (fee budget).
    pub total_cpfp_budget: u64,
    /// Length in bytes of the destination scriptPubKey for the sweep transaction.
    pub destination_script_len: usize,
    /// Target fee rate in satoshis per virtual byte.
    pub fee_rate: u64,
}

/// Selects leaves that are profitable for unilateral exit.
///
/// Uses a greedy algorithm: leaves are sorted by value descending and each leaf
/// is included when its value strictly exceeds the marginal cost of exiting it.
/// Marginal cost comprises CPFP fees for any not-yet-covered ancestor and refund
/// transactions, plus the incremental sweep-transaction input fee. Leaves that
/// share ancestors with previously selected leaves benefit from reduced marginal
/// cost.
///
/// Returns the selected leaves with their estimated costs (order matches the
/// CPFP broadcast sequence: highest-value first).
pub fn select_profitable_leaves(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    params: &LeafExitCostParams,
) -> Result<Vec<SelectedLeaf>, ServiceError> {
    // Collect leaves we can actually look up, sorted by value descending.
    let mut leaves: Vec<_> = leaf_ids
        .iter()
        .filter_map(|id| tree_nodes.get(id).map(|node| (id, node)))
        .collect();
    leaves.sort_by(|a, b| b.1.value.cmp(&a.1.value));

    let mut selected: Vec<SelectedLeaf> = Vec::new();
    let mut covered_txids: HashSet<bitcoin::Txid> = HashSet::new();
    let mut total_cpfp_cost: u64 = 0;
    // The very first CPFP child tx in the chain spends all initial inputs;
    // every subsequent one spends a single change output.
    let mut first_cpfp_pending = true;

    for (leaf_id, leaf) in &leaves {
        // Leaves without a refund tx cannot be unilaterally exited.
        let Some(refund_tx) = &leaf.refund_tx else {
            continue;
        };

        // Walk the ancestor chain from root to leaf.
        let Some(ancestors) = ancestor_chain(tree_nodes, leaf) else {
            continue; // incomplete chain — skip
        };

        // --- marginal CPFP cost ------------------------------------------
        let mut cpfp_cost: u64 = 0;
        let mut local_first = first_cpfp_pending;

        for ancestor in &ancestors {
            let txid = ancestor.node_tx.compute_txid();
            if covered_txids.contains(&txid) {
                continue;
            }
            let input_w = if local_first {
                local_first = false;
                params.initial_cpfp_input_weight
            } else {
                params.single_cpfp_input_weight
            };
            cpfp_cost += compute_cpfp_package_fee(
                ancestor.node_tx.weight().to_wu(),
                input_w,
                params.change_script_len,
                params.fee_rate,
            );
        }

        // Refund-tx CPFP
        let refund_input_w = if local_first {
            params.initial_cpfp_input_weight
        } else {
            params.single_cpfp_input_weight
        };
        cpfp_cost += compute_cpfp_package_fee(
            refund_tx.weight().to_wu(),
            refund_input_w,
            params.change_script_len,
            params.fee_rate,
        );

        // --- marginal sweep cost -----------------------------------------
        let sweep_cost = if selected.is_empty() {
            compute_sweep_fee(1, params.destination_script_len, params.fee_rate)
        } else {
            compute_sweep_fee(
                selected.len() + 1,
                params.destination_script_len,
                params.fee_rate,
            ) - compute_sweep_fee(
                selected.len(),
                params.destination_script_len,
                params.fee_rate,
            )
        };

        let total_marginal_cost = cpfp_cost + sweep_cost;

        // Unprofitable leaves are skipped silently. Budget is checked after
        // the full pass so the caller sees the total shortfall, not just the
        // first leaf that couldn't fit.
        if leaf.value > total_marginal_cost {
            selected.push(SelectedLeaf {
                id: (*leaf_id).clone(),
                value: leaf.value,
                estimated_cost: total_marginal_cost,
            });
            total_cpfp_cost += cpfp_cost;
            first_cpfp_pending = false;
            for ancestor in &ancestors {
                covered_txids.insert(ancestor.node_tx.compute_txid());
            }
        }
    }

    if !selected.is_empty() {
        let required_budget = total_cpfp_cost + params.change_dust_limit;
        if required_budget > params.total_cpfp_budget {
            let shortfall = required_budget - params.total_cpfp_budget;
            return Err(ServiceError::ValidationError(format!(
                "CPFP input value ({} sats) is too low to exit the profitable leaves: need {} more sats (required {}, including {} sats to keep the final CPFP change above dust)",
                params.total_cpfp_budget, shortfall, required_budget, params.change_dust_limit,
            )));
        }
    }

    Ok(selected)
}

/// Walks from `leaf` up to the root, returning the chain `[root, …, leaf]`.
/// Returns `None` if any parent is missing from `tree_nodes` or the chain
/// doesn't reach a root (node with no `parent_node_id`).
fn ancestor_chain<'a>(
    tree_nodes: &'a HashMap<TreeNodeId, TreeNode>,
    leaf: &'a TreeNode,
) -> Option<Vec<&'a TreeNode>> {
    let mut chain = vec![leaf];
    let mut current = leaf;
    while let Some(pid) = &current.parent_node_id {
        let parent = tree_nodes.get(pid)?;
        chain.push(parent);
        current = parent;
    }
    chain.reverse();
    Some(chain)
}

/// Creates a Partially Signed Bitcoin Transaction (PSBT) to CPFP a parent transaction.
///
/// This function creates a PSBT that spends from both CPFP inputs and the ephemeral anchor output
/// of the parent transaction. The resulting PSBT can be signed and broadcast to CPFP the parent
/// transaction with a fee.
///
/// # Arguments
/// * `tx` - The parent transaction to be CPFP'd
/// * `inputs` - A mutable vector of CPFP inputs for fee payment, will be updated with the change output
/// * `fee_rate` - The desired fee rate in satoshis per vbyte
///
/// # Returns
/// A Result containing the PSBT or an error
fn create_tx_cpfp_psbt(
    tx: &Transaction,
    inputs: &mut Vec<CpfpInput>,
    fee_rate: u64,
) -> Result<psbt::Psbt, ServiceError> {
    use bitcoin::psbt::{Input as PsbtInput, Output as PsbtOutput, Psbt};

    // Find the ephemeral anchor output in the parent transaction
    let (vout, anchor_tx_out) = tx
        .output
        .iter()
        .enumerate()
        .find(|(_, tx_out)| is_ephemeral_anchor_output(tx_out))
        .ok_or(ServiceError::ValidationError(
            "Ephemeral anchor output not found".to_string(),
        ))?;

    // We need at least one input for fee payment
    if inputs.is_empty() {
        return Err(ServiceError::ValidationError(
            "At least one CPFP input is required for fee bumping".to_string(),
        ));
    }

    // Calculate total available value from all inputs
    let total_input_value: u64 = inputs.iter().map(|i| i.witness_utxo.value.to_sat()).sum();

    // Change output reuses the first input's scriptPubKey
    let change_script_pubkey = inputs[0].witness_utxo.script_pubkey.clone();
    let first_signed_input_weight = inputs[0].signed_input_weight;

    // Create transaction inputs for all CPFP inputs plus the ephemeral anchor
    let mut tx_inputs = Vec::with_capacity(inputs.len() + 1);

    // Add all CPFP inputs with RBF signaling (BIP 125)
    // TODO: Improve UTXO selection for fees
    let rbf_sequence = Sequence(0xffff_fffd);
    for cpfp_input in inputs.iter() {
        tx_inputs.push(TxIn {
            previous_output: cpfp_input.outpoint,
            sequence: rbf_sequence,
            ..Default::default()
        });
    }

    // Add the ephemeral anchor input
    tx_inputs.push(TxIn {
        previous_output: OutPoint {
            txid: tx.compute_txid(),
            vout: vout as u32,
        },
        sequence: rbf_sequence,
        ..Default::default()
    });

    let input_weight: u64 = inputs.iter().map(|i| i.signed_input_weight).sum();
    let fee_amount = compute_cpfp_package_fee(
        tx.weight().to_wu(),
        input_weight,
        change_script_pubkey.len(),
        fee_rate,
    );
    trace!("Calculated fee: {} sats", fee_amount);

    // Adjust output value to account for fees
    let adjusted_output_value = total_input_value.saturating_sub(fee_amount);
    trace!("Remaining value: {} sats", adjusted_output_value);

    // The change output funds the next CPFP in the chain (and ultimately becomes
    // the caller's spendable change). It must meet the dust limit for its script
    // type, or the transaction will be rejected by the network as non-standard.
    let dust_limit = change_script_pubkey.minimal_non_dust().to_sat();
    if adjusted_output_value < dust_limit {
        return Err(ServiceError::ValidationError(format!(
            "CPFP change output ({adjusted_output_value} sats) is below the dust limit ({dust_limit} sats) for the input address"
        )));
    }

    // Create the base transaction structure
    let fee_bump_tx = Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: tx_inputs,
        output: vec![TxOut {
            value: Amount::from_sat(adjusted_output_value),
            script_pubkey: change_script_pubkey.clone(),
        }],
    };

    // Create a PSBT from the transaction
    let mut psbt = Psbt::from_unsigned_tx(fee_bump_tx.clone())
        .map_err(|e| ServiceError::ValidationError(format!("Failed to create PSBT: {e}")))?;

    // Add PSBT input information for all inputs
    for (i, cpfp_input) in inputs.iter().enumerate() {
        psbt.inputs[i] = PsbtInput {
            witness_utxo: Some(cpfp_input.witness_utxo.clone()),
            ..Default::default()
        };
    }

    // Add information for the last input (the anchor input)
    // Although no signing is needed for the anchor since it uses OP_TRUE,
    // we still provide the witness UTXO information for completeness
    psbt.inputs[inputs.len()] = PsbtInput {
        witness_utxo: Some(anchor_tx_out.clone()),
        ..Default::default()
    };

    // Add details for the output
    psbt.outputs[0] = PsbtOutput::default();

    // Replace all consumed inputs with just the change output
    *inputs = vec![CpfpInput {
        outpoint: OutPoint {
            txid: fee_bump_tx.compute_txid(),
            vout: 0,
        },
        witness_utxo: TxOut {
            value: Amount::from_sat(adjusted_output_value),
            script_pubkey: change_script_pubkey,
        },
        signed_input_weight: first_signed_input_weight,
    }];

    Ok(psbt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{
        Address, CompressedPublicKey, ScriptBuf,
        hashes::Hash,
        key::Secp256k1,
        secp256k1::{PublicKey, SecretKey, rand},
    };
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// P2WPKH signed input weight: 41 * 4 + 108 = 272 WU
    const P2WPKH_INPUT_WEIGHT: u64 = 272;
    /// P2TR signed input weight: 41 * 4 + 66 = 230 WU
    const P2TR_INPUT_WEIGHT: u64 = 230;

    /// Creates a transaction with an ephemeral anchor output for testing.
    fn create_test_transaction_with_anchor() -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::ZERO,
                script_pubkey: ScriptBuf::new_p2a(),
            }],
        }
    }

    fn p2wpkh_script(pubkey: PublicKey) -> ScriptBuf {
        Address::p2wpkh(&CompressedPublicKey(pubkey), bitcoin::Network::Testnet).script_pubkey()
    }

    fn p2tr_script(pubkey: PublicKey) -> ScriptBuf {
        let secp = Secp256k1::new();
        let (xonly, _) = pubkey.x_only_public_key();
        Address::p2tr(&secp, xonly, None, bitcoin::Network::Testnet).script_pubkey()
    }

    fn create_test_input_p2wpkh(pubkey: PublicKey, value: u64) -> CpfpInput {
        let random_bytes = (0..32).map(|_| rand::random::<u8>()).collect::<Vec<_>>();
        let txid = bitcoin::Txid::from_slice(&random_bytes).unwrap();
        CpfpInput {
            outpoint: OutPoint { txid, vout: 0 },
            witness_utxo: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: p2wpkh_script(pubkey),
            },
            signed_input_weight: P2WPKH_INPUT_WEIGHT,
        }
    }

    fn create_test_input_p2tr(pubkey: PublicKey, value: u64) -> CpfpInput {
        let random_bytes = (0..32).map(|_| rand::random::<u8>()).collect::<Vec<_>>();
        let txid = bitcoin::Txid::from_slice(&random_bytes).unwrap();
        CpfpInput {
            outpoint: OutPoint { txid, vout: 0 },
            witness_utxo: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: p2tr_script(pubkey),
            },
            signed_input_weight: P2TR_INPUT_WEIGHT,
        }
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_success() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![create_test_input_p2wpkh(pubkey, 10_000)];

        let fee_rate = 10;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_ok());

        let psbt = result.unwrap();
        assert_eq!(psbt.inputs.len(), 2); // One for our input, one for the anchor
        assert_eq!(psbt.outputs.len(), 1); // Change output

        // Verify the output value accounts for fees (package = parent + child in WU)
        let parent_wu = tx.weight().to_wu();
        // P2WPKH scriptPubKey is 22 bytes → output weight = (9 + 22) * 4 = 124
        let child_wu: u64 = 272 + 165 + 124 + 42;
        let package_weight = parent_wu + child_wu;
        let expected_fee = (fee_rate * package_weight).div_ceil(4);
        let expected_output_value = 10_000 - expected_fee;

        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Verify inputs array has been updated with the change output
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].witness_utxo.value.to_sat(), expected_output_value);
        assert_eq!(inputs[0].outpoint.vout, 0);
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_multiple_inputs() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![
            create_test_input_p2wpkh(pubkey, 5_000),
            create_test_input_p2wpkh(pubkey, 3_000),
            create_test_input_p2wpkh(pubkey, 2_000),
        ];

        let fee_rate = 10;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_ok());

        let psbt = result.unwrap();
        assert_eq!(psbt.inputs.len(), 4); // Three inputs + anchor
        assert_eq!(psbt.outputs.len(), 1);

        let total_input_value = 5_000 + 3_000 + 2_000;
        let parent_wu = tx.weight().to_wu();
        let child_wu: u64 = (3 * 272) + 165 + 124 + 42;
        let package_weight = parent_wu + child_wu;
        let expected_fee = (fee_rate * package_weight).div_ceil(4);
        let expected_output_value = total_input_value - expected_fee;

        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].witness_utxo.value.to_sat(), expected_output_value);
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_no_inputs() {
        let tx = create_test_transaction_with_anchor();
        let mut inputs = Vec::new();
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, 10);
        assert!(result.is_err());
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_insufficient_value() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![create_test_input_p2wpkh(pubkey, 10)];
        let fee_rate = 100;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_err());
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_no_anchor_output() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: p2wpkh_script(pubkey),
            }],
        };

        let mut inputs = vec![create_test_input_p2wpkh(pubkey, 10_000)];
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, 10);
        assert!(result.is_err());
        if let Err(ServiceError::ValidationError(msg)) = result {
            assert!(msg.contains("Ephemeral anchor output not found"));
        } else {
            panic!("Expected ValidationError");
        }
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_p2tr_input() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![create_test_input_p2tr(pubkey, 10_000)];

        let fee_rate = 10;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_ok());

        let psbt = result.unwrap();
        assert_eq!(psbt.inputs.len(), 2);
        assert_eq!(psbt.outputs.len(), 1);

        // P2TR scriptPubKey is 34 bytes → output weight = (9 + 34) * 4 = 172
        let parent_wu = tx.weight().to_wu();
        let child_wu: u64 = 230 + 165 + 172 + 42;
        let package_weight = parent_wu + child_wu;
        let expected_fee = (fee_rate * package_weight).div_ceil(4);
        let expected_output_value = 10_000 - expected_fee;
        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Verify the output is a P2TR script
        let script = &psbt.unsigned_tx.output[0].script_pubkey;
        assert!(script.is_p2tr());

        // Verify the change preserves P2TR scriptPubKey and weight
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].witness_utxo.script_pubkey.is_p2tr());
        assert_eq!(inputs[0].signed_input_weight, P2TR_INPUT_WEIGHT);
        assert_eq!(inputs[0].witness_utxo.value.to_sat(), expected_output_value);
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_mixed_input_types() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![
            create_test_input_p2wpkh(pubkey, 5_000),
            create_test_input_p2tr(pubkey, 3_000),
        ];

        let fee_rate = 10;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_ok());

        let psbt = result.unwrap();
        assert_eq!(psbt.inputs.len(), 3); // 2 inputs + anchor

        // Mixed: 272 (p2wpkh) + 230 (p2tr) + 165 (anchor) + 124 (p2wpkh output) + 42 (overhead)
        let parent_wu = tx.weight().to_wu();
        let child_wu: u64 = 272 + 230 + 165 + 124 + 42;
        let package_weight = parent_wu + child_wu;
        let expected_fee = (fee_rate * package_weight).div_ceil(4);
        let expected_output_value = 8_000 - expected_fee;
        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Change output uses the first input's scriptPubKey (P2WPKH)
        let script = &psbt.unsigned_tx.output[0].script_pubkey;
        assert!(script.is_p2wpkh());
        // Change carries forward first input's weight
        assert_eq!(inputs[0].signed_input_weight, P2WPKH_INPUT_WEIGHT);
    }

    // ---- helpers for select_profitable_leaves tests ----

    use crate::tree::{SigningKeyshare, TreeNodeStatus};
    use frost_secp256k1_tr::Identifier;
    use std::str::FromStr;

    /// Creates a unique transaction with an ephemeral anchor output.
    ///
    /// Each call produces a different txid (via a random dummy input) while
    /// keeping the same weight as the basic anchor tx used in CPFP tests.
    fn create_unique_anchor_tx() -> Transaction {
        let random_bytes: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
        let txid = bitcoin::Txid::from_slice(&random_bytes).unwrap();
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint { txid, vout: 0 },
                ..Default::default()
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(1000),
                    script_pubkey: ScriptBuf::from(
                        vec![0x00, 0x14]
                            .into_iter()
                            .chain(std::iter::repeat_n(0x00, 20))
                            .collect::<Vec<_>>(),
                    ),
                },
                TxOut {
                    value: Amount::ZERO,
                    script_pubkey: ScriptBuf::new_p2a(),
                },
            ],
        }
    }

    /// Build a minimal [`TreeNode`] for selection tests.
    ///
    /// Each node gets a unique `node_tx` (and `refund_tx` when requested) so
    /// txid deduplication in `select_profitable_leaves` works correctly.
    fn make_node(
        id: &str,
        value: u64,
        parent_id: Option<&str>,
        has_refund: bool,
        pubkey: PublicKey,
    ) -> TreeNode {
        use crate::tree::TreeNode;

        TreeNode {
            id: TreeNodeId::from_str(id).unwrap(),
            tree_id: "test-tree".to_string(),
            value,
            parent_node_id: parent_id.map(|p| TreeNodeId::from_str(p).unwrap()),
            node_tx: create_unique_anchor_tx(),
            refund_tx: if has_refund {
                Some(create_unique_anchor_tx())
            } else {
                None
            },
            direct_tx: None,
            direct_refund_tx: None,
            direct_from_cpfp_refund_tx: None,
            vout: 0,
            verifying_public_key: pubkey,
            owner_identity_public_key: None,
            signing_keyshare: SigningKeyshare {
                owner_identifiers: vec![Identifier::try_from(1u16).unwrap()],
                threshold: 1,
                public_key: pubkey,
            },
            status: TreeNodeStatus::Available,
        }
    }

    /// Collect nodes into the HashMap expected by `select_profitable_leaves`.
    fn node_map(nodes: Vec<TreeNode>) -> HashMap<TreeNodeId, TreeNode> {
        nodes.into_iter().map(|n| (n.id.clone(), n)).collect()
    }

    const FEE_RATE: u64 = 4;
    const CHANGE_SCRIPT_LEN: usize = 34; // P2TR scriptPubKey
    const DEST_SCRIPT_LEN: usize = 34;

    /// Default params using a single P2TR CPFP input.
    fn default_params(budget: u64) -> LeafExitCostParams {
        LeafExitCostParams {
            initial_cpfp_input_weight: P2TR_INPUT_WEIGHT,
            single_cpfp_input_weight: P2TR_INPUT_WEIGHT,
            change_script_len: CHANGE_SCRIPT_LEN,
            change_dust_limit: 1,
            total_cpfp_budget: budget,
            destination_script_len: DEST_SCRIPT_LEN,
            fee_rate: FEE_RATE,
        }
    }

    /// Computes the CPFP fee for one of our test anchor transactions.
    fn cpfp_fee(input_weight: u64) -> u64 {
        let tx_weight = create_unique_anchor_tx().weight().to_wu();
        compute_cpfp_package_fee(tx_weight, input_weight, CHANGE_SCRIPT_LEN, FEE_RATE)
    }

    /// Total cost for the first depth-1 leaf (root→leaf) with a single
    /// P2TR CPFP input: 3 CPFP txs + sweep(1).
    fn first_depth1_leaf_cost() -> u64 {
        3 * cpfp_fee(P2TR_INPUT_WEIGHT) + compute_sweep_fee(1, DEST_SCRIPT_LEN, FEE_RATE)
    }

    /// Marginal cost for a second depth-1 leaf sharing the same root:
    /// 2 CPFP txs (leaf node + refund, root already covered) + marginal sweep.
    fn second_shared_root_leaf_cost() -> u64 {
        2 * cpfp_fee(P2TR_INPUT_WEIGHT) + compute_sweep_fee(2, DEST_SCRIPT_LEN, FEE_RATE)
            - compute_sweep_fee(1, DEST_SCRIPT_LEN, FEE_RATE)
    }

    /// Total CPFP-only cost (no sweep) for the first depth-1 leaf.
    fn first_depth1_cpfp_cost() -> u64 {
        3 * cpfp_fee(P2TR_INPUT_WEIGHT)
    }

    /// CPFP-only cost for a second leaf sharing the root.
    fn second_shared_root_cpfp_cost() -> u64 {
        2 * cpfp_fee(P2TR_INPUT_WEIGHT)
    }

    #[test_all]
    fn test_select_single_leaf_one_sat_profit() {
        let pubkey = test_pubkey();
        let cost = first_depth1_leaf_cost();
        let root = make_node("root", 0, None, false, pubkey);
        let leaf = make_node("leaf", cost + 1, Some("root"), true, pubkey);
        let nodes = node_map(vec![root, leaf]);
        let leaf_ids = vec![TreeNodeId::from_str("leaf").unwrap()];
        let params = default_params(100_000);

        let selected = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap();
        assert_eq!(
            selected.len(),
            1,
            "leaf with 1 sat profit should be selected"
        );
    }

    #[test_all]
    fn test_select_single_leaf_zero_profit() {
        let pubkey = test_pubkey();
        let cost = first_depth1_leaf_cost();
        let root = make_node("root", 0, None, false, pubkey);
        let leaf = make_node("leaf", cost, Some("root"), true, pubkey);
        let nodes = node_map(vec![root, leaf]);
        let leaf_ids = vec![TreeNodeId::from_str("leaf").unwrap()];
        let params = default_params(100_000);

        let selected = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap();
        assert!(
            selected.is_empty(),
            "leaf with 0 profit must not be selected"
        );
    }

    #[test_all]
    fn test_select_single_leaf_unprofitable() {
        let pubkey = test_pubkey();
        let cost = first_depth1_leaf_cost();
        let root = make_node("root", 0, None, false, pubkey);
        let leaf = make_node("leaf", cost - 1, Some("root"), true, pubkey);
        let nodes = node_map(vec![root, leaf]);
        let leaf_ids = vec![TreeNodeId::from_str("leaf").unwrap()];
        let params = default_params(100_000);

        let selected = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap();
        assert!(
            selected.is_empty(),
            "unprofitable leaf must not be selected"
        );
    }

    #[test_all]
    fn test_select_shared_ancestor_reduces_cost() {
        // Tree: root → leaf-a (high value), root → leaf-b (lower value)
        // leaf-b's marginal cost is lower because root CPFP is already covered.
        let pubkey = test_pubkey();
        let second_cost = second_shared_root_leaf_cost();
        let root = make_node("root", 0, None, false, pubkey);
        let leaf_a = make_node("leaf-a", 100_000, Some("root"), true, pubkey);
        let leaf_b = make_node("leaf-b", second_cost + 1, Some("root"), true, pubkey);
        let nodes = node_map(vec![root, leaf_a, leaf_b]);
        let leaf_ids = vec![
            TreeNodeId::from_str("leaf-a").unwrap(),
            TreeNodeId::from_str("leaf-b").unwrap(),
        ];
        let params = default_params(1_000_000);

        let selected = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap();
        assert_eq!(selected.len(), 2, "both leaves should be selected");
        assert_eq!(selected[0].id.to_string(), "leaf-a");
        assert_eq!(selected[1].id.to_string(), "leaf-b");
    }

    #[test_all]
    fn test_select_shared_ancestor_second_leaf_zero_profit() {
        let pubkey = test_pubkey();
        let second_cost = second_shared_root_leaf_cost();
        let root = make_node("root", 0, None, false, pubkey);
        let leaf_a = make_node("leaf-a", 100_000, Some("root"), true, pubkey);
        let leaf_b = make_node("leaf-b", second_cost, Some("root"), true, pubkey);
        let nodes = node_map(vec![root, leaf_a, leaf_b]);
        let leaf_ids = vec![
            TreeNodeId::from_str("leaf-a").unwrap(),
            TreeNodeId::from_str("leaf-b").unwrap(),
        ];
        let params = default_params(1_000_000);

        let selected = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap();
        assert_eq!(selected.len(), 1, "only leaf-a should be selected");
        assert_eq!(selected[0].id.to_string(), "leaf-a");
    }

    #[test_all]
    fn test_select_insufficient_budget_errors() {
        let pubkey = test_pubkey();
        let first_cpfp = first_depth1_cpfp_cost();
        let second_cpfp = second_shared_root_cpfp_cost();
        let root = make_node("root", 0, None, false, pubkey);
        let leaf_a = make_node("leaf-a", 500_000, Some("root"), true, pubkey);
        let leaf_b = make_node("leaf-b", 500_000, Some("root"), true, pubkey);
        let nodes = node_map(vec![root, leaf_a, leaf_b]);
        let leaf_ids = vec![
            TreeNodeId::from_str("leaf-a").unwrap(),
            TreeNodeId::from_str("leaf-b").unwrap(),
        ];
        // Both leaves are profitable; budget covers the CPFP fees exactly but
        // leaves nothing for the dust-sized final change output (default
        // dust_limit = 1 sat in tests), so the selection must error.
        let budget = first_cpfp + second_cpfp;
        let params = default_params(budget);

        let err = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap_err();
        match err {
            ServiceError::ValidationError(msg) => {
                assert!(
                    msg.contains("1 more sats"),
                    "error should report 1-sat shortfall: {msg}"
                );
            }
            other => panic!("expected ValidationError, got {other:?}"),
        }

        // One more sat of budget lets both leaves through.
        let params2 = default_params(budget + 1);
        let root2 = make_node("root", 0, None, false, pubkey);
        let leaf_a2 = make_node("leaf-a", 500_000, Some("root"), true, pubkey);
        let leaf_b2 = make_node("leaf-b", 500_000, Some("root"), true, pubkey);
        let nodes2 = node_map(vec![root2, leaf_a2, leaf_b2]);
        let selected2 = select_profitable_leaves(&nodes2, &leaf_ids, &params2).unwrap();
        assert_eq!(selected2.len(), 2, "budget + 1 should allow both leaves");
    }

    #[test_all]
    fn test_select_shortfall_accumulates_across_leaves() {
        // Two independent profitable leaves; budget covers neither. The error
        // message must quote the combined shortfall, not just the first leaf.
        let pubkey = test_pubkey();
        let first_cpfp = first_depth1_cpfp_cost();
        let root_a = make_node("root-a", 0, None, false, pubkey);
        let leaf_a = make_node("leaf-a", 500_000, Some("root-a"), true, pubkey);
        let root_b = make_node("root-b", 0, None, false, pubkey);
        let leaf_b = make_node("leaf-b", 500_000, Some("root-b"), true, pubkey);
        let nodes = node_map(vec![root_a, leaf_a, root_b, leaf_b]);
        let leaf_ids = vec![
            TreeNodeId::from_str("leaf-a").unwrap(),
            TreeNodeId::from_str("leaf-b").unwrap(),
        ];
        // Separate roots → no shared-ancestor discount; both leaves pay the
        // full first_depth1 CPFP cost.
        let required = 2 * first_cpfp + 1; // +1 for default dust_limit
        let budget = required - 10;
        let params = default_params(budget);

        let err = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap_err();
        match err {
            ServiceError::ValidationError(msg) => {
                assert!(
                    msg.contains("10 more sats"),
                    "error should report 10-sat combined shortfall: {msg}"
                );
            }
            other => panic!("expected ValidationError, got {other:?}"),
        }
    }

    #[test_all]
    fn test_select_skip_no_refund_tx() {
        let pubkey = test_pubkey();
        let root = make_node("root", 0, None, false, pubkey);
        let leaf = make_node("leaf", 100_000, Some("root"), false, pubkey);
        let nodes = node_map(vec![root, leaf]);
        let leaf_ids = vec![TreeNodeId::from_str("leaf").unwrap()];
        let params = default_params(100_000);

        let selected = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap();
        assert!(
            selected.is_empty(),
            "leaf without refund_tx must be skipped"
        );
    }

    #[test_all]
    fn test_select_skip_missing_parent() {
        let pubkey = test_pubkey();
        let leaf = make_node("leaf", 100_000, Some("root"), true, pubkey);
        let nodes = node_map(vec![leaf]);
        let leaf_ids = vec![TreeNodeId::from_str("leaf").unwrap()];
        let params = default_params(100_000);

        let selected = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap();
        assert!(
            selected.is_empty(),
            "leaf with missing parent must be skipped"
        );
    }

    #[test_all]
    fn test_select_empty_leaf_list() {
        let nodes = HashMap::new();
        let leaf_ids: Vec<TreeNodeId> = vec![];
        let params = default_params(100_000);

        let selected = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap();
        assert!(selected.is_empty());
    }

    #[test_all]
    fn test_select_initial_cpfp_weight_affects_first_leaf() {
        let pubkey = test_pubkey();
        let tx_weight = create_unique_anchor_tx().weight().to_wu();
        // First CPFP uses doubled input weight
        let first_fee = compute_cpfp_package_fee(
            tx_weight,
            2 * P2TR_INPUT_WEIGHT,
            CHANGE_SCRIPT_LEN,
            FEE_RATE,
        );
        let subsequent_fee = cpfp_fee(P2TR_INPUT_WEIGHT);
        let cost = first_fee + 2 * subsequent_fee + compute_sweep_fee(1, DEST_SCRIPT_LEN, FEE_RATE);

        let root = make_node("root", 0, None, false, pubkey);
        let leaf = make_node("leaf", cost + 1, Some("root"), true, pubkey);
        let nodes = node_map(vec![root, leaf]);
        let leaf_ids = vec![TreeNodeId::from_str("leaf").unwrap()];
        let mut params = default_params(100_000);
        params.initial_cpfp_input_weight = 2 * P2TR_INPUT_WEIGHT;

        let selected = select_profitable_leaves(&nodes, &leaf_ids, &params).unwrap();
        assert_eq!(selected.len(), 1, "should be selected with 1 sat profit");

        // Value exactly at cost → not selected
        let root2 = make_node("root", 0, None, false, pubkey);
        let leaf2 = make_node("leaf", cost, Some("root"), true, pubkey);
        let nodes2 = node_map(vec![root2, leaf2]);
        let selected2 = select_profitable_leaves(&nodes2, &leaf_ids, &params).unwrap();
        assert!(selected2.is_empty(), "zero profit must not be selected");
    }

    #[test_all]
    fn test_compute_cpfp_package_fee_matches_create_psbt() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![create_test_input_p2tr(pubkey, 50_000)];
        let fee_rate = 7;

        let expected_fee =
            compute_cpfp_package_fee(tx.weight().to_wu(), P2TR_INPUT_WEIGHT, 34, fee_rate);

        let psbt = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate).unwrap();
        let actual_fee = 50_000 - psbt.unsigned_tx.output[0].value.to_sat();
        assert_eq!(actual_fee, expected_fee);
    }

    fn test_pubkey() -> PublicKey {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        PublicKey::from_secret_key(&secp, &secret_key)
    }

    #[test_all]
    fn test_is_ephemeral_anchor_output() {
        let valid_anchor = TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new_p2a(),
        };
        assert!(is_ephemeral_anchor_output(&valid_anchor));

        let non_zero_value = TxOut {
            value: Amount::from_sat(1),
            script_pubkey: ScriptBuf::new_p2a(),
        };
        assert!(!is_ephemeral_anchor_output(&non_zero_value));

        let different_script = TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::from(vec![0x51]),
        };
        assert!(!is_ephemeral_anchor_output(&different_script));
    }
}
