use std::collections::{HashMap, HashSet};

use bitcoin::{
    Amount, OutPoint, Psbt, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Weight, Witness,
    absolute::LockTime,
    psbt,
    secp256k1::constants::{MAX_SIGNATURE_SIZE, PUBLIC_KEY_SIZE, SCHNORR_SIGNATURE_SIZE},
    transaction::Version,
};
use tracing::{debug, trace, warn};

use crate::{
    services::ServiceError,
    tree::{TreeNode, TreeNodeId, TreeNodeStatus},
    utils::transactions::is_ephemeral_anchor_output,
};

/// Statuses where a node still belongs to an exit chain. `OnChain` is kept
/// (the SO marks a node `ON_CHAIN` once its tx confirms, still mid-exit);
/// `SplitLocked` is kept because a timelock renewal leaves a permanent
/// `SplitLocked` node above the renewed leaf that the walk must cross.
const EXIT_CHAIN_STATUSES: [TreeNodeStatus; 4] = [
    TreeNodeStatus::Available,
    TreeNodeStatus::Splitted,
    TreeNodeStatus::SplitLocked,
    TreeNodeStatus::OnChain,
];

/// Returns a leaf's ancestor chain, root → leaf, stopping above any node outside
/// [`EXIT_CHAIN_STATUSES`]. `Err(parent_id)` names the first ancestor missing
/// from `node_map` for the caller to re-fetch.
pub fn walk_exit_chain<'a>(
    node_map: &'a HashMap<TreeNodeId, TreeNode>,
    leaf: &'a TreeNode,
) -> Result<Vec<&'a TreeNode>, TreeNodeId> {
    let mut chain = Vec::new();
    let mut visited: HashSet<TreeNodeId> = HashSet::new();
    let mut current = leaf;
    loop {
        if !EXIT_CHAIN_STATUSES.contains(&current.status) {
            break;
        }
        // Cycle guard on semi-trusted parent ids. Returning an id already in the
        // map is how `build_exit_chain` tells a cycle from a missing parent.
        if !visited.insert(current.id.clone()) {
            return Err(current.id.clone());
        }
        chain.push(current);
        let Some(parent_node_id) = &current.parent_node_id else {
            break;
        };
        let Some(parent) = node_map.get(parent_node_id) else {
            return Err(parent_node_id.clone());
        };
        current = parent;
    }
    chain.reverse();
    Ok(chain)
}

/// Builds a leaf's exit chain from `node_map`, re-fetching absent ancestors via
/// `fetch_by_ids`. Re-fetch is needed because the SO's ancestor expansion skips
/// the root for legacy mainnet trees, omitting it from the bulk response.
pub async fn build_exit_chain<F, Fut>(
    leaf: TreeNode,
    node_map: &mut HashMap<TreeNodeId, TreeNode>,
    mut fetch_by_ids: F,
) -> Result<Vec<TreeNode>, ServiceError>
where
    F: FnMut(Vec<TreeNodeId>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<TreeNode>, ServiceError>>,
{
    loop {
        match walk_exit_chain(node_map, &leaf) {
            Ok(chain) => return Ok(chain.into_iter().cloned().collect()),
            Err(missing) => {
                // Already in the map => a cycle, not an absent parent.
                if node_map.contains_key(&missing) {
                    return Err(ServiceError::ValidationError(format!(
                        "Exit chain contains a parent cycle at node {missing}",
                    )));
                }
                debug!(
                    "Parent {missing} missing from query_nodes response; re-fetching by node ID"
                );
                for node in fetch_by_ids(vec![missing.clone()]).await? {
                    node_map.insert(node.id.clone(), node);
                }
                if !node_map.contains_key(&missing) {
                    return Err(ServiceError::ValidationError(format!(
                        "Parent node {missing} not returned by query_nodes; exit chain incomplete",
                    )));
                }
            }
        }
    }
}

/// A funding UTXO for CPFP fee-bumping.
#[derive(Clone, Debug)]
pub struct CpfpInput {
    pub outpoint: OutPoint,
    pub witness_utxo: TxOut,
    /// Upper bound on the signed weight: fees size from it, so a shorter real
    /// signature overpays slightly, never underpays.
    pub signed_input_weight: u64,
}

pub struct CpfpChild {
    pub psbt: Psbt,
    pub change_input: CpfpInput,
    pub fee_sat: u64,
}

#[derive(Clone, Debug)]
pub struct ExitPlan {
    pub selected_leaves: Vec<SelectedLeaf>,
    /// Set when inputs can't be matched 1:1 to branches; one output per branch.
    pub fan_out_psbt: Option<psbt::Psbt>,
    /// Leaf id -> the inputs funding that branch's first CPFP child.
    pub per_branch_funding: Vec<(TreeNodeId, Vec<CpfpInput>)>,
    pub tree_nodes: Vec<TreeNode>,
}

/// Selects which leaves to exit and maps funding inputs to branches. Never
/// fetches: works offline as long as `tree_nodes` holds each selected leaf's
/// full ancestor chain.
pub fn plan_exit(
    tree_nodes: HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    filter: LeafFilter,
    inputs: Vec<CpfpInput>,
    fee_rate_sat_per_kw: u64,
    destination_script_len: usize,
) -> Result<ExitPlan, ServiceError> {
    if inputs.is_empty() {
        return Err(ServiceError::ValidationError(
            "At least one CPFP input is required".to_string(),
        ));
    }
    if leaf_ids.is_empty() {
        return Ok(ExitPlan {
            selected_leaves: vec![],
            fan_out_psbt: None,
            per_branch_funding: vec![],
            tree_nodes: tree_nodes.into_values().collect(),
        });
    }

    let change_script = &inputs[0].witness_utxo.script_pubkey;
    let params = LeafExitCostParams {
        initial_cpfp_input_weight: Weight::from_wu(
            inputs
                .iter()
                .map(|i| i.signed_input_weight)
                .fold(0u64, u64::saturating_add),
        ),
        single_cpfp_input_weight: Weight::from_wu(inputs[0].signed_input_weight),
        change_script_len: change_script.len(),
        change_dust_limit: change_script.minimal_non_dust().to_sat(),
        total_cpfp_budget: inputs
            .iter()
            .map(|i| i.witness_utxo.value.to_sat())
            .fold(0u64, u64::saturating_add),
        destination_script_len,
        fee_rate_sat_per_kw,
    };

    let selected = evaluate_leaf_exit_costs(&tree_nodes, leaf_ids, &params, filter)?;
    if selected.is_empty() {
        return Ok(ExitPlan {
            selected_leaves: vec![],
            fan_out_psbt: None,
            per_branch_funding: vec![],
            tree_nodes: tree_nodes.into_values().collect(),
        });
    }

    let (per_branch_funding, fan_out_psbt) = if selected.len() == 1 {
        (vec![(selected[0].id.clone(), inputs)], None)
    } else if let Some(assignment) =
        assign_inputs_to_leaves(&inputs, &selected, params.change_dust_limit)
    {
        (assignment, None)
    } else {
        let (psbt, per_leaf) = build_fan_out_psbt(
            &inputs,
            &selected,
            fee_rate_sat_per_kw,
            params.change_dust_limit,
        )?;
        (
            per_leaf
                .into_iter()
                .map(|(id, input)| (id, vec![input]))
                .collect(),
            Some(psbt),
        )
    };

    let plan = ExitPlan {
        selected_leaves: selected,
        fan_out_psbt,
        per_branch_funding,
        tree_nodes: tree_nodes.into_values().collect(),
    };
    debug!(
        selected_leaves = plan.selected_leaves.len(),
        branches = plan.per_branch_funding.len(),
        has_fan_out = plan.fan_out_psbt.is_some(),
        tree_nodes = plan.tree_nodes.len(),
        "plan_exit: planned"
    );
    Ok(plan)
}

/// A chain-independent unilateral-exit quote: which leaves would exit and the
/// funding they need, sized from the funding kind's weight with no actual UTXOs.
pub struct ExitQuote {
    pub selected_leaves: Vec<SelectedLeaf>,
    /// Per-branch funding to avoid a fan-out: (leaf id, minimum sats).
    pub per_branch_funding: Vec<(TreeNodeId, u64)>,
    pub single_utxo_funding_sat: u64,
    pub fanout_fee_sat: u64,
    pub total_fee_sat: u64,
}

/// Like [`plan_exit`] but sizes fees from a funding kind's weight with no actual
/// UTXOs and never rejects on budget: it only reports the funding required.
#[allow(clippy::too_many_arguments)]
pub fn quote_exit(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    filter: LeafFilter,
    funding_input_weight: u64,
    funding_output_script_len: usize,
    change_dust_limit: u64,
    fee_rate_sat_per_kw: u64,
    destination_script_len: usize,
) -> Result<ExitQuote, ServiceError> {
    let params = LeafExitCostParams {
        initial_cpfp_input_weight: Weight::from_wu(funding_input_weight),
        single_cpfp_input_weight: Weight::from_wu(funding_input_weight),
        change_script_len: funding_output_script_len,
        change_dust_limit,
        total_cpfp_budget: u64::MAX,
        destination_script_len,
        fee_rate_sat_per_kw,
    };

    let selected = evaluate_leaf_exit_costs(tree_nodes, leaf_ids, &params, filter)?;
    if selected.is_empty() {
        return Ok(ExitQuote {
            selected_leaves: vec![],
            per_branch_funding: vec![],
            single_utxo_funding_sat: 0,
            fanout_fee_sat: 0,
            total_fee_sat: 0,
        });
    }

    let per_branch_funding: Vec<(TreeNodeId, u64)> = selected
        .iter()
        .map(|l| {
            (
                l.id.clone(),
                l.estimated_cost.saturating_add(change_dust_limit),
            )
        })
        .collect();
    let leaves_total: u64 = per_branch_funding
        .iter()
        .map(|(_, sat)| *sat)
        .fold(0u64, u64::saturating_add);
    let sum_estimated: u64 = selected
        .iter()
        .map(|l| l.estimated_cost)
        .fold(0u64, u64::saturating_add);

    let fanout_fee_sat = if selected.len() == 1 {
        0
    } else {
        fan_out_fee(
            Weight::from_wu(funding_input_weight),
            funding_output_script_len,
            selected.len(),
            fee_rate_sat_per_kw,
        )
    };

    Ok(ExitQuote {
        single_utxo_funding_sat: leaves_total.saturating_add(fanout_fee_sat),
        total_fee_sat: sum_estimated.saturating_add(fanout_fee_sat),
        selected_leaves: selected,
        per_branch_funding,
        fanout_fee_sat,
    })
}

/// `tx`'s relative CSV timelock in blocks, or `None` when it has no block-based
/// relative timelock.
pub fn csv_timelock(tx: &Transaction) -> Option<u32> {
    tx.input
        .iter()
        .filter_map(|input| match input.sequence.to_relative_lock_time()? {
            bitcoin::relative::LockTime::Blocks(h) => {
                let v = u32::from(h.value());
                (v > 0).then_some(v)
            }
            bitcoin::relative::LockTime::Time(_) => None,
        })
        .max()
}

pub fn p2tr_key_path_input_weight() -> Weight {
    input_segwit_weight(&[SCHNORR_SIGNATURE_SIZE])
}

pub fn p2wpkh_input_weight() -> Weight {
    input_segwit_weight(&[MAX_SIGNATURE_SIZE, PUBLIC_KEY_SIZE])
}

#[derive(Debug, Clone)]
pub struct SelectedLeaf {
    pub id: TreeNodeId,
    pub value: u64,
    /// Marginal exit cost (CPFP fees + sweep input fee). Order-dependent: a shared
    /// ancestor is charged to the first selected leaf reaching it, not a fair share.
    pub estimated_cost: u64,
}

pub struct LeafExitCostParams {
    /// Weight of the first CPFP child's inputs in a leaf's chain.
    pub initial_cpfp_input_weight: Weight,
    /// Weight of each subsequent child's single (chained-change) input.
    pub single_cpfp_input_weight: Weight,
    pub change_script_len: usize,
    pub change_dust_limit: u64,
    pub total_cpfp_budget: u64,
    pub destination_script_len: usize,
    pub fee_rate_sat_per_kw: u64,
}

/// Signed weight of one input with the given witness-element lengths.
/// `TxIn::segwit_weight` counts the empty-witness `00` varint even for a
/// witness-less input, matching SegWit serialization.
fn input_segwit_weight(witness_element_lens: &[usize]) -> Weight {
    let mut witness = Witness::new();
    for &len in witness_element_lens {
        witness.push(vec![0u8; len]);
    }
    TxIn {
        witness,
        ..Default::default()
    }
    .segwit_weight()
}

fn anchor_input_weight() -> Weight {
    input_segwit_weight(&[])
}

/// SegWit transaction overhead. A zero-input tx still serializes in SegWit
/// format, so its weight already includes the marker + flag every CPFP,
/// fan-out, and sweep tx carries.
fn tx_overhead_weight() -> Weight {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![],
        output: vec![],
    }
    .weight()
}

fn output_weight(script_len: usize) -> Weight {
    TxOut {
        value: Amount::ZERO,
        script_pubkey: ScriptBuf::from_bytes(vec![0u8; script_len]),
    }
    .weight()
}

fn fee_sat(fee_rate_sat_per_kw: u64, weight: Weight) -> u64 {
    fee_rate_sat_per_kw
        .saturating_mul(weight.to_wu())
        .div_ceil(1000)
}

/// Fee for a parent-child CPFP pair: the child pays for both, since the parent
/// carries no fee of its own (it spends via an ephemeral anchor).
pub fn compute_cpfp_package_fee(
    parent_weight: Weight,
    cpfp_input_weight: Weight,
    change_script_len: usize,
    fee_rate_sat_per_kw: u64,
) -> u64 {
    let child_weight = cpfp_input_weight
        + anchor_input_weight()
        + output_weight(change_script_len)
        + tx_overhead_weight();
    fee_sat(fee_rate_sat_per_kw, parent_weight + child_weight)
}

/// Fee for the sweep. The caller passes the total input weight directly because
/// the sweep mixes P2TR refund inputs and external CPFP-change inputs.
pub fn compute_sweep_fee(
    total_input_weight: Weight,
    destination_script_len: usize,
    fee_rate_sat_per_kw: u64,
) -> u64 {
    let weight = total_input_weight + output_weight(destination_script_len) + tx_overhead_weight();
    fee_sat(fee_rate_sat_per_kw, weight)
}

fn fan_out_weight(
    total_input_weight: Weight,
    output_script_len: usize,
    output_count: usize,
) -> Weight {
    let outputs = output_weight(output_script_len)
        .to_wu()
        .saturating_mul(output_count as u64);
    total_input_weight + Weight::from_wu(outputs) + tx_overhead_weight()
}

/// Fee for a fan-out (no change output).
pub fn fan_out_fee(
    total_input_weight: Weight,
    output_script_len: usize,
    output_count: usize,
    fee_rate_sat_per_kw: u64,
) -> u64 {
    fee_sat(
        fee_rate_sat_per_kw,
        fan_out_weight(total_input_weight, output_script_len, output_count),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafFilter {
    /// Keep every requested leaf, even when its exit cost exceeds its value.
    All,
    /// Keep only leaves whose value strictly exceeds their marginal exit cost.
    ProfitableOnly,
}

/// A leaf the caller named ([`LeafFilter::All`]) that can't be exited is an
/// error; under [`LeafFilter::ProfitableOnly`] it is warned and skipped.
fn report_unexitable(
    filter: LeafFilter,
    leaf_id: &TreeNodeId,
    reason: &str,
) -> Result<(), ServiceError> {
    if filter == LeafFilter::All {
        return Err(ServiceError::ValidationError(format!(
            "Leaf {leaf_id} cannot be exited: {reason}"
        )));
    }
    warn!("Leaf {leaf_id} cannot be exited: {reason}; skipping");
    Ok(())
}

/// Selects the leaves to exit, highest value first. Greedy: a leaf is kept when
/// its value exceeds its marginal cost (CPFP fees for its not-yet-covered
/// ancestors and refund, plus the incremental sweep input). A shared ancestor is
/// charged only to the first leaf reaching it, so order matters; `All` keeps all.
pub fn evaluate_leaf_exit_costs(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    params: &LeafExitCostParams,
    filter: LeafFilter,
) -> Result<Vec<SelectedLeaf>, ServiceError> {
    let mut leaves: Vec<(&TreeNodeId, &TreeNode)> = Vec::with_capacity(leaf_ids.len());
    for id in leaf_ids {
        match tree_nodes.get(id) {
            Some(node) => leaves.push((id, node)),
            None => report_unexitable(filter, id, "not found in the tree node map")?,
        }
    }
    leaves.sort_by(|a, b| b.1.value.cmp(&a.1.value).then_with(|| a.0.cmp(b.0)));

    let mut selected: Vec<SelectedLeaf> = Vec::new();
    let mut covered_txids: HashSet<bitcoin::Txid> = HashSet::new();
    let mut total_cpfp_cost: u64 = 0;

    for (leaf_id, leaf) in &leaves {
        let Some(refund_tx) = &leaf.refund_tx else {
            report_unexitable(filter, leaf_id, "no refund transaction")?;
            continue;
        };
        let ancestors = match walk_exit_chain(tree_nodes, leaf) {
            Ok(ancestors) => ancestors,
            Err(missing) => {
                report_unexitable(
                    filter,
                    leaf_id,
                    &format!(
                        "incomplete ancestor chain (parent {missing} missing from the tree map)"
                    ),
                )?;
                continue;
            }
        };

        let mut cpfp_cost: u64 = 0;
        let mut already_funded_ancestor = false;
        for ancestor in &ancestors {
            let txid = ancestor.node_tx.compute_txid();
            if covered_txids.contains(&txid) {
                continue;
            }
            // On-chain ancestor is already confirmed, so its CPFP fee is already paid.
            if ancestor.status == TreeNodeStatus::OnChain {
                continue;
            }
            let input_weight = if already_funded_ancestor {
                params.single_cpfp_input_weight
            } else {
                already_funded_ancestor = true;
                params.initial_cpfp_input_weight
            };
            cpfp_cost = cpfp_cost.saturating_add(compute_cpfp_package_fee(
                ancestor.node_tx.weight(),
                input_weight,
                params.change_script_len,
                params.fee_rate_sat_per_kw,
            ));
        }
        let refund_input_weight = if already_funded_ancestor {
            params.single_cpfp_input_weight
        } else {
            params.initial_cpfp_input_weight
        };
        cpfp_cost = cpfp_cost.saturating_add(compute_cpfp_package_fee(
            refund_tx.weight(),
            refund_input_weight,
            params.change_script_len,
            params.fee_rate_sat_per_kw,
        ));

        let per_leaf_input_weight = p2tr_key_path_input_weight() + params.single_cpfp_input_weight;
        let sweep_input_weight =
            |count: u64| Weight::from_wu(count.saturating_mul(per_leaf_input_weight.to_wu()));
        let sweep_cost = if selected.is_empty() {
            compute_sweep_fee(
                per_leaf_input_weight,
                params.destination_script_len,
                params.fee_rate_sat_per_kw,
            )
        } else {
            let selected_count = selected.len() as u64;
            compute_sweep_fee(
                sweep_input_weight(selected_count.saturating_add(1)),
                params.destination_script_len,
                params.fee_rate_sat_per_kw,
            )
            .saturating_sub(compute_sweep_fee(
                sweep_input_weight(selected_count),
                params.destination_script_len,
                params.fee_rate_sat_per_kw,
            ))
        };

        let total_marginal_cost = cpfp_cost.saturating_add(sweep_cost);

        if filter == LeafFilter::All || leaf.value > total_marginal_cost {
            selected.push(SelectedLeaf {
                id: (*leaf_id).clone(),
                value: leaf.value,
                estimated_cost: total_marginal_cost,
            });
            total_cpfp_cost = total_cpfp_cost.saturating_add(cpfp_cost);
            for ancestor in &ancestors {
                covered_txids.insert(ancestor.node_tx.compute_txid());
            }
        }
    }

    if !selected.is_empty() {
        // Each branch's CPFP chain ends in a terminal change output the sweep
        // consumes, so reserve one dust limit per branch.
        let terminal_change_reserve =
            (selected.len() as u64).saturating_mul(params.change_dust_limit);
        let required_budget = total_cpfp_cost.saturating_add(terminal_change_reserve);
        if required_budget > params.total_cpfp_budget {
            return Err(ServiceError::InsufficientCpfpBudget {
                required_sat: required_budget,
            });
        }
    }

    Ok(selected)
}

/// Partitions the CPFP inputs across branches so each is funded by its own
/// subset, avoiding a fan-out. Greedy, costliest branch first, holding one input
/// in reserve per not-yet-funded branch. `None` when no partition fits.
///
/// Returned in `selected_leaves` order (value-descending, as
/// [`evaluate_leaf_exit_costs`] emits), not the internal greedy order. The
/// funding sizes each branch assuming a shared ancestor is charged to the first
/// leaf in value order; `build_exit` charges it to the first branch it iterates.
/// Returning in value order keeps those two the same branch, so no branch is
/// left short of a shared ancestor's fee and fails its dust check.
pub fn assign_inputs_to_leaves(
    inputs: &[CpfpInput],
    selected_leaves: &[SelectedLeaf],
    change_dust_limit: u64,
) -> Option<Vec<(TreeNodeId, Vec<CpfpInput>)>> {
    if inputs.len() < selected_leaves.len() {
        return None;
    }
    let mut remaining: Vec<&CpfpInput> = inputs.iter().collect();
    remaining.sort_by(|a, b| {
        b.witness_utxo
            .value
            .cmp(&a.witness_utxo.value)
            .then_with(|| a.outpoint.cmp(&b.outpoint))
    });
    let mut sorted_leaves: Vec<&SelectedLeaf> = selected_leaves.iter().collect();
    sorted_leaves.sort_by(|a, b| {
        b.estimated_cost
            .cmp(&a.estimated_cost)
            .then_with(|| a.id.cmp(&b.id))
    });

    let leaf_count = sorted_leaves.len();
    let mut assigned_by_leaf: HashMap<TreeNodeId, Vec<CpfpInput>> =
        HashMap::with_capacity(leaf_count);
    for (i, leaf) in sorted_leaves.iter().enumerate() {
        let required = leaf.estimated_cost.saturating_add(change_dust_limit);
        let branches_left_after = leaf_count.saturating_sub(i + 1);
        let mut assigned: Vec<CpfpInput> = Vec::new();
        let mut sum: u64 = 0;
        while sum < required {
            if remaining.len() <= branches_left_after {
                return None;
            }
            let input = remaining.remove(0);
            sum = sum.saturating_add(input.witness_utxo.value.to_sat());
            assigned.push(input.clone());
        }
        assigned_by_leaf.insert(leaf.id.clone(), assigned);
    }
    Some(
        selected_leaves
            .iter()
            .map(|leaf| {
                (
                    leaf.id.clone(),
                    assigned_by_leaf.remove(&leaf.id).unwrap_or_default(),
                )
            })
            .collect(),
    )
}

/// Builds an unsigned fan-out PSBT with one output per selected leaf. No change
/// output: surplus input value is folded into the per-branch outputs (the
/// caller's own funding script), where it doubles as fee headroom for a
/// higher-fee resume that reuses this confirmed fan-out. RBF-signaled so an
/// unconfirmed fan-out can be replaced.
pub fn build_fan_out_psbt(
    inputs: &[CpfpInput],
    selected_leaves: &[SelectedLeaf],
    fee_rate_sat_per_kw: u64,
    change_dust_limit: u64,
) -> Result<(psbt::Psbt, Vec<(TreeNodeId, CpfpInput)>), ServiceError> {
    if inputs.is_empty() {
        return Err(ServiceError::ValidationError(
            "fan-out: at least one CPFP input is required".to_string(),
        ));
    }
    if selected_leaves.is_empty() {
        return Err(ServiceError::ValidationError(
            "fan-out: at least one selected leaf is required".to_string(),
        ));
    }

    let script_pubkey = inputs[0].witness_utxo.script_pubkey.clone();
    let signed_input_weight = inputs[0].signed_input_weight;

    let total_input_value: u64 = inputs
        .iter()
        .map(|i| i.witness_utxo.value.to_sat())
        .fold(0u64, u64::saturating_add);
    let total_input_weight: u64 = inputs
        .iter()
        .map(|i| i.signed_input_weight)
        .fold(0u64, u64::saturating_add);

    let per_leaf_value: Vec<u64> = selected_leaves
        .iter()
        .map(|l| l.estimated_cost.saturating_add(change_dust_limit))
        .collect();
    let leaves_total: u64 = per_leaf_value
        .iter()
        .copied()
        .fold(0u64, u64::saturating_add);

    let fee_no_change = fan_out_fee(
        Weight::from_wu(total_input_weight),
        script_pubkey.len(),
        selected_leaves.len(),
        fee_rate_sat_per_kw,
    );

    if total_input_value < leaves_total.saturating_add(fee_no_change) {
        return Err(ServiceError::InsufficientCpfpBudget {
            required_sat: leaves_total.saturating_add(fee_no_change),
        });
    }

    let surplus = total_input_value
        .saturating_sub(leaves_total)
        .saturating_sub(fee_no_change);
    let mut output_values: Vec<u64> = per_leaf_value.clone();
    if surplus > 0 {
        let cost_total: u128 = selected_leaves
            .iter()
            .map(|l| u128::from(l.estimated_cost))
            .sum();
        let mut distributed: u64 = 0;
        for (idx, leaf) in selected_leaves.iter().enumerate() {
            // checked_div guards cost_total == 0 (all branch costs zero): no share
            // is distributed and the whole surplus falls to the first branch below.
            let share = u128::from(surplus)
                .saturating_mul(u128::from(leaf.estimated_cost))
                .checked_div(cost_total)
                .and_then(|s| u64::try_from(s).ok())
                .unwrap_or(0);
            output_values[idx] = output_values[idx].saturating_add(share);
            distributed = distributed.saturating_add(share);
        }
        output_values[0] = output_values[0].saturating_add(surplus.saturating_sub(distributed));
    }

    let tx_inputs: Vec<TxIn> = inputs
        .iter()
        .map(|i| TxIn {
            previous_output: i.outpoint,
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            ..Default::default()
        })
        .collect();
    let tx_outputs: Vec<TxOut> = output_values
        .iter()
        .map(|&v| TxOut {
            value: Amount::from_sat(v),
            script_pubkey: script_pubkey.clone(),
        })
        .collect();

    let tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: tx_inputs,
        output: tx_outputs,
    };
    let txid = tx.compute_txid();

    let mut psbt_unsigned = psbt::Psbt::from_unsigned_tx(tx).map_err(|e| {
        ServiceError::ValidationError(format!("Failed to create fan-out PSBT: {e}"))
    })?;
    for (i, cpfp_input) in inputs.iter().enumerate() {
        psbt_unsigned.inputs[i] = psbt::Input {
            witness_utxo: Some(cpfp_input.witness_utxo.clone()),
            ..Default::default()
        };
    }

    let per_leaf_inputs: Vec<(TreeNodeId, CpfpInput)> = selected_leaves
        .iter()
        .enumerate()
        .map(|(idx, leaf)| {
            (
                leaf.id.clone(),
                CpfpInput {
                    outpoint: OutPoint {
                        txid,
                        vout: idx as u32,
                    },
                    witness_utxo: TxOut {
                        value: Amount::from_sat(output_values[idx]),
                        script_pubkey: script_pubkey.clone(),
                    },
                    signed_input_weight,
                },
            )
        })
        .collect();

    trace!(
        inputs = inputs.len(),
        branches = selected_leaves.len(),
        total_input_value,
        fee = fee_no_change,
        "build_fan_out_psbt"
    );
    Ok((psbt_unsigned, per_leaf_inputs))
}

/// Builds a single CPFP child for `parent_tx`, spending the parent's ephemeral
/// anchor plus the funding inputs at the exact fee for `fee_rate`. Its one change
/// output (the first input's script) is returned as [`CpfpChild::change_input`]
/// to fund the next child in a chain.
pub fn build_cpfp_child(
    parent_tx: &Transaction,
    funding_inputs: &[CpfpInput],
    fee_rate_sat_per_kw: u64,
) -> Result<CpfpChild, ServiceError> {
    use bitcoin::psbt::{Input as PsbtInput, Output as PsbtOutput, Psbt};

    let (vout, anchor_tx_out) = parent_tx
        .output
        .iter()
        .enumerate()
        .find(|(_, tx_out)| is_ephemeral_anchor_output(tx_out))
        .ok_or(ServiceError::ValidationError(
            "Ephemeral anchor output not found".to_string(),
        ))?;

    if funding_inputs.is_empty() {
        return Err(ServiceError::ValidationError(
            "At least one CPFP input is required for fee bumping".to_string(),
        ));
    }

    let total_input_value: u64 = funding_inputs
        .iter()
        .map(|i| i.witness_utxo.value.to_sat())
        .fold(0u64, u64::saturating_add);
    let change_script_pubkey = funding_inputs[0].witness_utxo.script_pubkey.clone();
    let first_signed_input_weight = funding_inputs[0].signed_input_weight;

    let rbf_sequence = Sequence(0xffff_fffd);
    let mut tx_inputs = Vec::with_capacity(funding_inputs.len() + 1);
    for cpfp_input in funding_inputs {
        tx_inputs.push(TxIn {
            previous_output: cpfp_input.outpoint,
            sequence: rbf_sequence,
            ..Default::default()
        });
    }
    tx_inputs.push(TxIn {
        previous_output: OutPoint {
            txid: parent_tx.compute_txid(),
            vout: vout as u32,
        },
        sequence: rbf_sequence,
        ..Default::default()
    });

    let input_weight: u64 = funding_inputs
        .iter()
        .map(|i| i.signed_input_weight)
        .fold(0u64, u64::saturating_add);
    let fee_amount = compute_cpfp_package_fee(
        parent_tx.weight(),
        Weight::from_wu(input_weight),
        change_script_pubkey.len(),
        fee_rate_sat_per_kw,
    );

    let adjusted_output_value = total_input_value.saturating_sub(fee_amount);
    let dust_limit = change_script_pubkey.minimal_non_dust().to_sat();
    if adjusted_output_value < dust_limit {
        return Err(ServiceError::ValidationError(format!(
            "CPFP change output ({adjusted_output_value} sats) is below the dust limit ({dust_limit} sats) for the input address"
        )));
    }
    trace!(
        parent_txid = %parent_tx.compute_txid(),
        funding_inputs = funding_inputs.len(),
        total_input_value,
        fee_amount,
        change_value = adjusted_output_value,
        "build_cpfp_child"
    );

    let fee_bump_tx = Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: tx_inputs,
        output: vec![TxOut {
            value: Amount::from_sat(adjusted_output_value),
            script_pubkey: change_script_pubkey.clone(),
        }],
    };

    let mut psbt = Psbt::from_unsigned_tx(fee_bump_tx.clone())
        .map_err(|e| ServiceError::ValidationError(format!("Failed to create PSBT: {e}")))?;
    for (i, cpfp_input) in funding_inputs.iter().enumerate() {
        psbt.inputs[i] = PsbtInput {
            witness_utxo: Some(cpfp_input.witness_utxo.clone()),
            ..Default::default()
        };
    }
    psbt.inputs[funding_inputs.len()] = PsbtInput {
        witness_utxo: Some(anchor_tx_out.clone()),
        ..Default::default()
    };
    psbt.outputs[0] = PsbtOutput::default();

    let change_input = CpfpInput {
        outpoint: OutPoint {
            txid: fee_bump_tx.compute_txid(),
            vout: 0,
        },
        witness_utxo: TxOut {
            value: Amount::from_sat(adjusted_output_value),
            script_pubkey: change_script_pubkey,
        },
        signed_input_weight: first_signed_input_weight,
    };

    Ok(CpfpChild {
        psbt,
        change_input,
        fee_sat: fee_amount,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{
        Address, CompressedPublicKey, ScriptBuf, Txid,
        hashes::Hash,
        key::Secp256k1,
        secp256k1::{PublicKey, SecretKey},
    };
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn create_test_transaction_with_anchor() -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
            }],
        }
    }

    #[test_all]
    fn test_is_ephemeral_anchor_output() {
        let valid_anchor = TxOut {
            value: Amount::from_sat(0),
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        };
        assert!(is_ephemeral_anchor_output(&valid_anchor));

        let non_zero_value = TxOut {
            value: Amount::from_sat(1),
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        };
        assert!(!is_ephemeral_anchor_output(&non_zero_value));

        let different_script = TxOut {
            value: Amount::from_sat(0),
            script_pubkey: ScriptBuf::from(vec![0x51]),
        };
        assert!(!is_ephemeral_anchor_output(&different_script));
    }

    mod exit_chain {
        use super::*;
        use crate::tree::tests::create_test_tree_node;
        use std::str::FromStr;

        const ROOT: &str = "root";
        const MID: &str = "mid";
        const LEAF: &str = "leaf";

        fn node(id: &str, parent: Option<&str>, status: TreeNodeStatus) -> TreeNode {
            let mut n = create_test_tree_node(id, 1_000);
            n.parent_node_id = parent.map(|p| TreeNodeId::from_str(p).unwrap());
            n.status = status;
            n
        }

        fn chain_ids(chain: &[TreeNode]) -> Vec<String> {
            chain.iter().map(|n| n.id.to_string()).collect()
        }

        #[macros::async_test_all]
        async fn full_map_no_refetch() {
            let root = node(ROOT, None, TreeNodeStatus::Available);
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Splitted);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);

            let mut map: HashMap<TreeNodeId, TreeNode> = [&root, &mid, &leaf]
                .into_iter()
                .map(|n| (n.id.clone(), n.clone()))
                .collect();

            let mut fetched = false;
            let chain = super::super::build_exit_chain(leaf, &mut map, |_ids| {
                fetched = true;
                async move { Ok(Vec::new()) }
            })
            .await
            .unwrap();

            assert!(
                !fetched,
                "fetcher must not be called when chain is complete"
            );
            assert_eq!(chain_ids(&chain), vec![ROOT, MID, LEAF]);
        }

        #[macros::async_test_all]
        async fn refetches_missing_root() {
            let root = node(ROOT, None, TreeNodeStatus::Available);
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Splitted);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);

            let mut map: HashMap<TreeNodeId, TreeNode> = [&mid, &leaf]
                .into_iter()
                .map(|n| (n.id.clone(), n.clone()))
                .collect();

            let server: HashMap<TreeNodeId, TreeNode> =
                [(root.id.clone(), root.clone())].into_iter().collect();
            let mut requested: Vec<TreeNodeId> = Vec::new();

            let chain = super::super::build_exit_chain(leaf, &mut map, |ids: Vec<TreeNodeId>| {
                requested.extend_from_slice(&ids);
                let nodes: Vec<TreeNode> =
                    ids.iter().filter_map(|i| server.get(i).cloned()).collect();
                async move { Ok(nodes) }
            })
            .await
            .unwrap();

            assert_eq!(requested, vec![root.id.clone()]);
            assert_eq!(chain_ids(&chain), vec![ROOT, MID, LEAF]);
        }

        #[macros::async_test_all]
        async fn walks_through_split_locked_parent() {
            let root = node(ROOT, None, TreeNodeStatus::Available);
            let mid = node(MID, Some(ROOT), TreeNodeStatus::SplitLocked);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);

            let mut map: HashMap<TreeNodeId, TreeNode> = [&root, &mid, &leaf]
                .into_iter()
                .map(|n| (n.id.clone(), n.clone()))
                .collect();

            let chain =
                super::super::build_exit_chain(leaf, &mut map, |_ids| async { Ok(Vec::new()) })
                    .await
                    .unwrap();

            assert_eq!(chain_ids(&chain), vec![ROOT, MID, LEAF]);
        }

        #[macros::async_test_all]
        async fn stops_on_non_exit_status() {
            let root = node(ROOT, None, TreeNodeStatus::Available);
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Exited);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);

            let mut map: HashMap<TreeNodeId, TreeNode> = [&root, &mid, &leaf]
                .into_iter()
                .map(|n| (n.id.clone(), n.clone()))
                .collect();

            let chain =
                super::super::build_exit_chain(leaf, &mut map, |_ids| async { Ok(Vec::new()) })
                    .await
                    .unwrap();

            assert_eq!(chain_ids(&chain), vec![LEAF]);
        }

        #[macros::async_test_all]
        async fn parent_unavailable_errors() {
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Splitted);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);

            let mut map: HashMap<TreeNodeId, TreeNode> = [&mid, &leaf]
                .into_iter()
                .map(|n| (n.id.clone(), n.clone()))
                .collect();

            let err = super::super::build_exit_chain(leaf, &mut map, async |_ids| Ok(Vec::new()))
                .await
                .unwrap_err();

            match err {
                ServiceError::ValidationError(msg) => {
                    assert!(msg.contains("exit chain incomplete"))
                }
                other => panic!("expected ValidationError, got {other:?}"),
            }
        }

        #[macros::async_test_all]
        async fn cycle_errors_without_looping() {
            let root = node(ROOT, Some(MID), TreeNodeStatus::Available);
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Available);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);

            let mut map: HashMap<TreeNodeId, TreeNode> = [&root, &mid, &leaf]
                .into_iter()
                .map(|n| (n.id.clone(), n.clone()))
                .collect();

            let mut fetched = false;
            let err = super::super::build_exit_chain(leaf, &mut map, |_ids| {
                fetched = true;
                async move { Ok(Vec::new()) }
            })
            .await
            .unwrap_err();

            assert!(!fetched, "a cycle is detected without re-fetching");
            match err {
                ServiceError::ValidationError(msg) => assert!(msg.contains("cycle")),
                other => panic!("expected a cycle ValidationError, got {other:?}"),
            }
        }

        #[macros::async_test_all]
        async fn refetch_missing_node_errors_without_looping() {
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Splitted);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);
            let other = node("other", None, TreeNodeStatus::Available);

            let mut map: HashMap<TreeNodeId, TreeNode> = [&mid, &leaf]
                .into_iter()
                .map(|n| (n.id.clone(), n.clone()))
                .collect();

            let mut calls = 0u32;
            let err = super::super::build_exit_chain(leaf, &mut map, |_ids: Vec<TreeNodeId>| {
                calls += 1;
                let nodes = vec![other.clone()];
                async move { Ok(nodes) }
            })
            .await
            .unwrap_err();

            assert_eq!(calls, 1, "the wrong re-fetch is not retried in a loop");
            match err {
                ServiceError::ValidationError(msg) => assert!(msg.contains("incomplete")),
                other => panic!("expected an incomplete ValidationError, got {other:?}"),
            }
        }
    }

    mod v2_planner {
        use super::*;
        use std::str::FromStr;

        fn test_script() -> bitcoin::ScriptBuf {
            let secp = Secp256k1::new();
            let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
            let pk = PublicKey::from_secret_key(&secp, &sk);
            Address::p2wpkh(&CompressedPublicKey(pk), bitcoin::Network::Testnet).script_pubkey()
        }

        fn cpfp_input(value: u64, vout: u32) -> CpfpInput {
            CpfpInput {
                outpoint: OutPoint {
                    txid: Txid::from_byte_array([7u8; 32]),
                    vout,
                },
                witness_utxo: TxOut {
                    value: Amount::from_sat(value),
                    script_pubkey: test_script(),
                },
                signed_input_weight: 272,
            }
        }

        fn selected(id: &str, value: u64, cost: u64) -> SelectedLeaf {
            SelectedLeaf {
                id: TreeNodeId::from_str(id).unwrap(),
                value,
                estimated_cost: cost,
            }
        }

        #[test_all]
        fn cpfp_package_fee_is_exact() {
            let (parent, input) = (Weight::from_wu(400), Weight::from_wu(272));
            assert_eq!(compute_cpfp_package_fee(parent, input, 22, 500), 502);
            assert!(
                compute_cpfp_package_fee(parent, input, 22, 1000)
                    > compute_cpfp_package_fee(parent, input, 22, 500)
            );
        }

        #[test_all]
        fn sweep_fee_is_exact() {
            assert_eq!(compute_sweep_fee(Weight::from_wu(230), 22, 500), 198);
        }

        // Guards the rust-bitcoin-derived weights against upstream drift.
        #[test_all]
        fn structural_weights_are_exact() {
            assert_eq!(p2tr_key_path_input_weight().to_wu(), 230);
            assert_eq!(p2wpkh_input_weight().to_wu(), 272);
            assert_eq!(anchor_input_weight().to_wu(), 165);
            assert_eq!(tx_overhead_weight().to_wu(), 42);
        }

        #[test_all]
        fn assign_inputs_matches_greedy_descending() {
            let inputs = vec![cpfp_input(10_000, 0), cpfp_input(5_000, 1)];
            let leaves = vec![selected("a", 50_000, 3_000), selected("b", 20_000, 1_000)];
            let got = assign_inputs_to_leaves(&inputs, &leaves, 330).expect("should fit");
            assert_eq!(got.len(), 2);
            assert!(got.iter().all(|(_, ins)| ins.len() == 1));
        }

        #[test_all]
        fn assign_inputs_returns_value_order_when_richest_is_not_costliest() {
            // Leaf "a" is richer but cheaper; "b" is poorer but costlier. Input
            // is value order (a, b); the greedy pass runs costliest-first (b, a).
            let inputs = vec![cpfp_input(10_000, 0), cpfp_input(5_000, 1)];
            let leaves = vec![selected("a", 50_000, 1_000), selected("b", 20_000, 3_000)];
            let got = assign_inputs_to_leaves(&inputs, &leaves, 330).expect("should fit");
            // Returned in value order, matching evaluate_leaf_exit_costs.
            assert_eq!(got[0].0, leaves[0].id);
            assert_eq!(got[1].0, leaves[1].id);
            // The costlier branch still greedily took the larger input.
            assert_eq!(got[1].1[0].witness_utxo.value.to_sat(), 10_000);
            assert_eq!(got[0].1[0].witness_utxo.value.to_sat(), 5_000);
        }

        #[test_all]
        fn assign_inputs_combines_multiple_inputs_per_branch() {
            let inputs = vec![
                cpfp_input(10_000, 0),
                cpfp_input(1_000, 1),
                cpfp_input(1_000, 2),
            ];
            let leaves = vec![selected("a", 50_000, 3_000), selected("b", 20_000, 1_500)];
            let got = assign_inputs_to_leaves(&inputs, &leaves, 330).expect("should fit");
            assert_eq!(got.len(), 2);
            let total_inputs: usize = got.iter().map(|(_, ins)| ins.len()).sum();
            assert_eq!(total_inputs, 3);
            assert!(got.iter().any(|(_, ins)| ins.len() == 2));
        }

        #[test_all]
        fn assign_inputs_rejects_too_few_or_underfunded() {
            let leaves = vec![selected("a", 50_000, 3_000), selected("b", 20_000, 1_000)];
            assert!(assign_inputs_to_leaves(&[cpfp_input(10_000, 0)], &leaves, 330).is_none());
            let small = vec![cpfp_input(3_000, 0), cpfp_input(1_000, 1)];
            assert!(assign_inputs_to_leaves(&small, &leaves, 330).is_none());
        }

        #[test_all]
        fn fan_out_emits_one_output_per_branch_and_is_deterministic() {
            let inputs = vec![cpfp_input(100_000, 0)];
            let leaves = vec![selected("a", 50_000, 3_000), selected("b", 40_000, 2_000)];
            let (psbt, per_leaf) = build_fan_out_psbt(&inputs, &leaves, 250, 330).unwrap();
            assert_eq!(psbt.unsigned_tx.output.len(), 2);
            assert_eq!(per_leaf.len(), 2);
            assert!(per_leaf[0].1.witness_utxo.value.to_sat() > 3_000 + 330);
            assert!(per_leaf[1].1.witness_utxo.value.to_sat() > 2_000 + 330);
            assert_eq!(per_leaf[0].1.outpoint.vout, 0);
            assert_eq!(per_leaf[1].1.outpoint.vout, 1);
            assert_eq!(
                psbt.unsigned_tx.input[0].sequence,
                Sequence::ENABLE_RBF_NO_LOCKTIME
            );
            let (psbt2, _) = build_fan_out_psbt(&inputs, &leaves, 250, 330).unwrap();
            assert_eq!(
                psbt.unsigned_tx.compute_txid(),
                psbt2.unsigned_tx.compute_txid()
            );
        }

        #[test_all]
        fn fan_out_rejects_insufficient_funding() {
            let inputs = vec![cpfp_input(4_000, 0)];
            let leaves = vec![selected("a", 50_000, 3_000), selected("b", 40_000, 2_000)];
            assert!(build_fan_out_psbt(&inputs, &leaves, 250, 330).is_err());
        }

        #[test_all]
        fn cpfp_child_spends_anchor_and_chains_change() {
            let parent = create_test_transaction_with_anchor();
            let funding = vec![cpfp_input(10_000, 0)];
            let child = build_cpfp_child(&parent, &funding, 1250).unwrap();
            assert_eq!(child.psbt.unsigned_tx.input.len(), 2);
            assert_eq!(child.psbt.unsigned_tx.output.len(), 1);
            assert_eq!(child.change_input.outpoint.vout, 0);
            assert_eq!(
                child.change_input.outpoint.txid,
                child.psbt.unsigned_tx.compute_txid()
            );
            assert!(child.change_input.witness_utxo.value.to_sat() < 10_000);
        }

        #[test_all]
        fn cpfp_child_rejects_dust_change() {
            let parent = create_test_transaction_with_anchor();
            let funding = vec![cpfp_input(200, 0)];
            assert!(build_cpfp_child(&parent, &funding, 12500).is_err());
        }

        fn leaf_node(id: &str, value: u64) -> TreeNode {
            let mut n = crate::tree::tests::create_test_tree_node(id, value);
            n.node_tx = create_test_transaction_with_anchor();
            n.refund_tx = Some(create_test_transaction_with_anchor());
            n
        }

        fn cost_params() -> LeafExitCostParams {
            LeafExitCostParams {
                initial_cpfp_input_weight: Weight::from_wu(272),
                single_cpfp_input_weight: Weight::from_wu(272),
                change_script_len: 22,
                change_dust_limit: 330,
                total_cpfp_budget: 1_000_000,
                destination_script_len: 22,
                fee_rate_sat_per_kw: 250,
            }
        }

        #[test_all]
        fn select_auto_keeps_profitable_drops_unprofitable() {
            let node = leaf_node("leaf", 1_000_000);
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();

            let sel = evaluate_leaf_exit_costs(
                &nodes,
                std::slice::from_ref(&id),
                &cost_params(),
                LeafFilter::ProfitableOnly,
            )
            .unwrap();
            assert_eq!(sel.len(), 1);

            let small = leaf_node("leaf", 10);
            let sid = small.id.clone();
            let small_nodes: HashMap<TreeNodeId, TreeNode> =
                [(sid.clone(), small)].into_iter().collect();
            let sel = evaluate_leaf_exit_costs(
                &small_nodes,
                &[sid],
                &cost_params(),
                LeafFilter::ProfitableOnly,
            )
            .unwrap();
            assert!(sel.is_empty());
        }

        #[test_all]
        fn profitability_boundary_is_strict() {
            let probe = leaf_node("leaf", 1_000_000);
            let pid = probe.id.clone();
            let probe_nodes: HashMap<TreeNodeId, TreeNode> =
                [(pid.clone(), probe)].into_iter().collect();
            let cost =
                evaluate_leaf_exit_costs(&probe_nodes, &[pid], &cost_params(), LeafFilter::All)
                    .unwrap()[0]
                    .estimated_cost;
            assert!(
                cost > 1,
                "cost must exceed 1 sat for the boundary to be meaningful"
            );

            let at = leaf_node("leaf", cost);
            let at_id = at.id.clone();
            let at_nodes: HashMap<TreeNodeId, TreeNode> =
                [(at_id.clone(), at)].into_iter().collect();
            assert!(
                evaluate_leaf_exit_costs(
                    &at_nodes,
                    &[at_id],
                    &cost_params(),
                    LeafFilter::ProfitableOnly
                )
                .unwrap()
                .is_empty(),
                "a leaf worth exactly its exit cost must be dropped under Auto"
            );

            let above = leaf_node("leaf", cost + 1);
            let above_id = above.id.clone();
            let above_nodes: HashMap<TreeNodeId, TreeNode> =
                [(above_id.clone(), above)].into_iter().collect();
            let sel = evaluate_leaf_exit_costs(
                &above_nodes,
                &[above_id],
                &cost_params(),
                LeafFilter::ProfitableOnly,
            )
            .unwrap();
            assert_eq!(
                sel.len(),
                1,
                "a leaf worth exit cost + 1 must be kept under Auto"
            );
            assert_eq!(sel[0].estimated_cost, cost);
        }

        #[test_all]
        fn evaluate_all_keeps_unprofitable() {
            let small = leaf_node("leaf", 10);
            let sid = small.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(sid.clone(), small)].into_iter().collect();
            let sel =
                evaluate_leaf_exit_costs(&nodes, &[sid], &cost_params(), LeafFilter::All).unwrap();
            assert_eq!(sel.len(), 1);
        }

        #[test_all]
        fn evaluate_unexitable_errors_under_all_but_skips_under_profitable_only() {
            let mut node = leaf_node("leaf", 1_000_000);
            node.refund_tx = None;
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();

            assert!(
                evaluate_leaf_exit_costs(
                    &nodes,
                    std::slice::from_ref(&id),
                    &cost_params(),
                    LeafFilter::All
                )
                .is_err()
            );
            let sel =
                evaluate_leaf_exit_costs(&nodes, &[id], &cost_params(), LeafFilter::ProfitableOnly)
                    .unwrap();
            assert!(sel.is_empty());
        }

        const DUST: u64 = 330;

        #[test_all]
        fn quote_single_leaf_has_no_fanout_fee() {
            let node = leaf_node("leaf", 1_000_000);
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();

            let quote = quote_exit(
                &nodes,
                &[id],
                LeafFilter::ProfitableOnly,
                272,
                22,
                DUST,
                250,
                22,
            )
            .unwrap();
            assert_eq!(quote.selected_leaves.len(), 1);
            let est = quote.selected_leaves[0].estimated_cost;

            assert_eq!(quote.fanout_fee_sat, 0);
            assert_eq!(quote.per_branch_funding.len(), 1);
            assert_eq!(quote.per_branch_funding[0].1, est + DUST);
            assert_eq!(quote.single_utxo_funding_sat, est + DUST);
            assert_eq!(quote.total_fee_sat, est);
        }

        #[test_all]
        fn quote_two_leaves_adds_fanout_fee() {
            let a = leaf_node("a", 1_000_000);
            let b = leaf_node("b", 1_000_000);
            let (ida, idb) = (a.id.clone(), b.id.clone());
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(ida.clone(), a), (idb.clone(), b)].into_iter().collect();

            let quote = quote_exit(
                &nodes,
                &[ida, idb],
                LeafFilter::ProfitableOnly,
                272,
                22,
                DUST,
                250,
                22,
            )
            .unwrap();
            assert_eq!(quote.selected_leaves.len(), 2);

            let expected_fanout = fan_out_fee(Weight::from_wu(272), 22, 2, 250);
            assert!(expected_fanout > 0);
            assert_eq!(quote.fanout_fee_sat, expected_fanout);

            let sum_est: u64 = quote.selected_leaves.iter().map(|l| l.estimated_cost).sum();
            let leaves_total: u64 = quote.per_branch_funding.iter().map(|(_, s)| *s).sum();
            assert_eq!(leaves_total, sum_est + 2 * DUST);
            assert_eq!(quote.total_fee_sat, sum_est + expected_fanout);
            assert_eq!(
                quote.single_utxo_funding_sat,
                leaves_total + expected_fanout
            );
            for (_, sat) in &quote.per_branch_funding {
                assert!(*sat >= DUST);
            }
        }

        fn anchor_tx_n(nonce: u32) -> Transaction {
            Transaction {
                version: Version::non_standard(3),
                lock_time: LockTime::from_height(nonce).unwrap(),
                input: Vec::new(),
                output: vec![TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
                }],
            }
        }

        #[test_all]
        fn onchain_ancestor_lowers_estimated_cost() {
            let chain = |root_status: TreeNodeStatus| -> HashMap<TreeNodeId, TreeNode> {
                let mut root = crate::tree::tests::create_test_tree_node("root", 1_000_000);
                root.node_tx = anchor_tx_n(1);
                root.status = root_status;
                let mut leaf = crate::tree::tests::create_test_tree_node("leaf", 1_000_000);
                leaf.node_tx = anchor_tx_n(2);
                leaf.refund_tx = Some(anchor_tx_n(3));
                leaf.parent_node_id = Some(TreeNodeId::from_str("root").unwrap());
                [(root.id.clone(), root), (leaf.id.clone(), leaf)]
                    .into_iter()
                    .collect()
            };
            let leaf_id = TreeNodeId::from_str("leaf").unwrap();
            let cost_of = |nodes: &HashMap<TreeNodeId, TreeNode>| {
                evaluate_leaf_exit_costs(
                    nodes,
                    std::slice::from_ref(&leaf_id),
                    &cost_params(),
                    LeafFilter::All,
                )
                .unwrap()[0]
                    .estimated_cost
            };

            let cost_all = cost_of(&chain(TreeNodeStatus::Available));
            let cost_onchain = cost_of(&chain(TreeNodeStatus::OnChain));
            assert!(
                cost_onchain < cost_all,
                "an OnChain ancestor is already paid, so it must lower the estimated cost \
                 ({cost_onchain} vs {cost_all})"
            );
        }

        #[test_all]
        fn quote_auto_drops_all_unprofitable_to_zero() {
            let node = leaf_node("leaf", 10);
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();

            let quote = quote_exit(
                &nodes,
                &[id],
                LeafFilter::ProfitableOnly,
                272,
                22,
                DUST,
                250,
                22,
            )
            .unwrap();
            assert!(quote.selected_leaves.is_empty());
            assert_eq!(quote.per_branch_funding.len(), 0);
            assert_eq!(quote.single_utxo_funding_sat, 0);
            assert_eq!(quote.fanout_fee_sat, 0);
            assert_eq!(quote.total_fee_sat, 0);
        }
    }
}
