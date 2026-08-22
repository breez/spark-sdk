use std::{
    cmp,
    collections::{HashMap, HashSet},
};

use bitcoin::{
    Amount, OutPoint, Psbt, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Weight, Witness,
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
pub fn walk_unilateral_exit_chain<'a>(
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
        // map is how a caller tells a cycle from a missing parent.
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

/// How an exit's funding UTXOs map to the transactions whose fees they pay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CpfpFundingShape {
    /// One funding UTXO per branch. A branch's inputs all fund its first CPFP
    /// child, and every child after that spends the previous child's change.
    #[default]
    PerBranch,
    /// One funding UTXO per fee-bumped transaction: every tree node the exit
    /// broadcasts, and every leaf's refund. No CPFP child spends another child's
    /// output, so each child's inputs are settled before any fee rate is picked.
    /// Each child leaves its own change on the funding script, so a branch ends
    /// with one change output per transaction rather than one in total.
    PerNode,
}

/// One transaction a per-node exit fee-bumps, with the sats its funding UTXO
/// must hold: the CPFP package fee plus the non-dust change every child writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnilateralExitNodeFunding {
    /// The leaf whose branch the transaction belongs to.
    pub leaf_id: TreeNodeId,
    /// The tree node it belongs to, which is the leaf itself for a refund.
    pub node_id: TreeNodeId,
    /// The transaction being fee-bumped.
    pub txid: Txid,
    /// Set for a leaf's `refund_tx`, clear for a node's `node_tx`.
    pub refund: bool,
    pub funding_sat: u64,
}

#[derive(Clone, Debug)]
pub struct UnilateralExitPlan {
    pub selected_leaves: Vec<UnilateralExitSelectedLeaf>,
    /// How `per_branch_funding`'s inputs pay for a branch's transactions.
    pub funding_shape: CpfpFundingShape,
    /// Set when inputs can't be matched 1:1 to what they fund; one output each.
    pub fan_out_psbt: Option<psbt::Psbt>,
    /// Leaf id -> the inputs funding that branch. Under
    /// [`CpfpFundingShape::PerBranch`] they all fund its first CPFP child, whose
    /// change then funds the next. Under [`CpfpFundingShape::PerNode`] there is
    /// one per transaction the branch fee-bumps, in `per_node_funding` order.
    pub per_branch_funding: Vec<(TreeNodeId, Vec<CpfpInput>)>,
    /// Empty under [`CpfpFundingShape::PerBranch`]. Under
    /// [`CpfpFundingShape::PerNode`], the transaction each of
    /// `per_branch_funding`'s inputs fee-bumps, grouped and ordered to match it.
    pub per_node_funding: Vec<(TreeNodeId, Vec<UnilateralExitNodeFunding>)>,
    /// The exit tree, keyed by node id. Every selected leaf's full ancestor chain
    /// is present, so the build resolves offline without re-fetching.
    pub tree_nodes: HashMap<TreeNodeId, TreeNode>,
}

/// Selects which leaves to exit and maps funding inputs to the transactions they
/// pay for. Never fetches: works offline as long as `tree_nodes` holds each
/// selected leaf's full ancestor chain.
#[allow(clippy::too_many_arguments)]
pub fn plan_unilateral_exit(
    tree_nodes: HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    filter: UnilateralExitLeafFilter,
    inputs: Vec<CpfpInput>,
    funding_shape: CpfpFundingShape,
    fee_rate_sat_per_kw: u64,
    destination_script_len: usize,
) -> Result<UnilateralExitPlan, ServiceError> {
    if inputs.is_empty() {
        return Err(ServiceError::ValidationError(
            "At least one CPFP input is required".to_string(),
        ));
    }
    if leaf_ids.is_empty() {
        return Ok(empty_plan(tree_nodes, funding_shape));
    }

    let change_script = &inputs[0].witness_utxo.script_pubkey;
    let change_dust_limit = change_script.minimal_non_dust().to_sat();
    let params = leaf_cost_params(
        Weight::from_wu(inputs[0].signed_input_weight),
        change_script.len(),
        destination_script_len,
        fee_rate_sat_per_kw,
        funding_shape,
    );

    let selected = evaluate_unilateral_exit_leaf_costs(&tree_nodes, leaf_ids, &params, filter)?;
    if selected.is_empty() {
        return Ok(empty_plan(tree_nodes, funding_shape));
    }

    let (per_branch_funding, node_funding, fan_out_psbt) = match funding_shape {
        CpfpFundingShape::PerNode => {
            let named = per_node_funding(&tree_nodes, &selected, &params, change_dust_limit);
            let (funding, fan_out) = assign_inputs_to_nodes(inputs, &named, fee_rate_sat_per_kw)?;
            (funding, named, fan_out)
        }
        CpfpFundingShape::PerBranch => {
            let (funding, fan_out) = assign_inputs_to_branches(
                &tree_nodes,
                &selected,
                inputs,
                change_dust_limit,
                fee_rate_sat_per_kw,
                destination_script_len,
            )?;
            (funding, Vec::new(), fan_out)
        }
    };

    let plan = UnilateralExitPlan {
        selected_leaves: selected,
        funding_shape,
        fan_out_psbt,
        per_branch_funding,
        per_node_funding: node_funding,
        tree_nodes,
    };
    debug!(
        selected_leaves = plan.selected_leaves.len(),
        ?funding_shape,
        branches = plan.per_branch_funding.len(),
        has_fan_out = plan.fan_out_psbt.is_some(),
        tree_nodes = plan.tree_nodes.len(),
        "plan_unilateral_exit: planned"
    );
    Ok(plan)
}

/// Funds each branch's first CPFP child, whose change then funds the rest: a
/// single branch takes every input, several are partitioned one subset each, and
/// a set that partitions no way is fanned out into one output per branch.
fn assign_inputs_to_branches(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    selected: &[UnilateralExitSelectedLeaf],
    inputs: Vec<CpfpInput>,
    change_dust_limit: u64,
    fee_rate_sat_per_kw: u64,
    destination_script_len: usize,
) -> Result<(BranchInputs, Option<psbt::Psbt>), ServiceError> {
    let funding = if selected.len() == 1 {
        // The single-leaf arm hands every input to the one branch, so unlike the
        // multi-branch paths it has no partition step to reject underfunding. Gate
        // it on build_cpfp_child's physical floor (CPFP fees + dust); the sweep is
        // paid from the swept value, not this funding UTXO, so estimated_cost's
        // sweep component (the quote's headroom) must not inflate the hard gate.
        //
        // The build funds the first CPFP child with ALL supplied inputs, so size
        // the floor on their combined weight via first_child_cpfp_floor: the
        // selection pass sized it on one input, which under-gates a single leaf
        // funded by several UTXOs. With one input that estimate already reflects the
        // real weight, so skip the re-cost.
        let cpfp_cost = if inputs.len() > 1 {
            first_child_cpfp_floor(
                tree_nodes,
                &selected[0].id,
                &inputs,
                destination_script_len,
                fee_rate_sat_per_kw,
            )
            .unwrap_or(selected[0].cpfp_cost)
        } else {
            selected[0].cpfp_cost
        };
        let required = cpfp_cost.saturating_add(change_dust_limit);
        let available = inputs
            .iter()
            .map(|i| i.witness_utxo.value.to_sat())
            .fold(0u64, u64::saturating_add);
        if available < required {
            return Err(ServiceError::InsufficientCpfpBudget {
                required_sat: required,
            });
        }
        (vec![(selected[0].id.clone(), inputs)], None)
    } else if let Some(assignment) = assign_inputs_to_leaves(&inputs, selected, change_dust_limit)
        .filter(|a| {
            assignment_covers_first_child(
                a,
                tree_nodes,
                destination_script_len,
                fee_rate_sat_per_kw,
            )
        })
    {
        (assignment, None)
    } else {
        let output_values: Vec<u64> = selected
            .iter()
            .map(|l| branch_required_funding(l, change_dust_limit))
            .collect();
        let surplus_weights: Vec<u64> = selected.iter().map(|l| l.estimated_cost).collect();
        let (psbt, fanned) = build_fan_out_psbt(
            &inputs,
            &output_values,
            &surplus_weights,
            fee_rate_sat_per_kw,
        )?;
        (
            selected
                .iter()
                .zip(fanned)
                .map(|(leaf, input)| (leaf.id.clone(), vec![input]))
                .collect(),
            Some(psbt),
        )
    };
    Ok(funding)
}

/// A plan that exits nothing, so there is nothing to fund.
fn empty_plan(
    tree_nodes: HashMap<TreeNodeId, TreeNode>,
    funding_shape: CpfpFundingShape,
) -> UnilateralExitPlan {
    UnilateralExitPlan {
        selected_leaves: vec![],
        funding_shape,
        fan_out_psbt: None,
        per_branch_funding: vec![],
        per_node_funding: vec![],
        tree_nodes,
    }
}

/// Cost parameters for one funding script and shape. Per-node funding gives every
/// CPFP child a UTXO of its own, so no child carries a combined input weight.
fn leaf_cost_params(
    funding_input_weight: Weight,
    change_script_len: usize,
    destination_script_len: usize,
    fee_rate_sat_per_kw: u64,
    funding_shape: CpfpFundingShape,
) -> UnilateralExitLeafCostParams {
    UnilateralExitLeafCostParams {
        initial_cpfp_input_weight: funding_input_weight,
        single_cpfp_input_weight: funding_input_weight,
        change_script_len,
        destination_script_len,
        fee_rate_sat_per_kw,
        funding_shape,
    }
}

/// The inputs funding each branch, in the shape [`UnilateralExitPlan`] carries.
type BranchInputs = Vec<(TreeNodeId, Vec<CpfpInput>)>;

/// Matches one supplied input to each transaction the exit fee-bumps. Exactly
/// one input per transaction, each covering the amount named at its position,
/// is taken in that order: the caller has pinned each UTXO to its transaction,
/// and the pinning holds at every fee rate, which is what lets the same UTXO
/// fund the same child across a set of pre-signed alternatives. Otherwise the
/// inputs are matched by size, leaving any beyond the count untouched. Too few,
/// or a set that covers no way, is fanned out into one output per transaction:
/// a shortfall would pass the plan and then fail in [`build_cpfp_child`].
fn assign_inputs_to_nodes(
    inputs: Vec<CpfpInput>,
    node_funding: &[(TreeNodeId, Vec<UnilateralExitNodeFunding>)],
    fee_rate_sat_per_kw: u64,
) -> Result<(BranchInputs, Option<psbt::Psbt>), ServiceError> {
    let required: Vec<u64> = node_funding
        .iter()
        .flat_map(|(_, funding)| funding.iter().map(|n| n.funding_sat))
        .collect();
    if required.is_empty() {
        return Err(ServiceError::ValidationError(
            "Nothing left to fee-bump in this exit".to_string(),
        ));
    }

    // Every amount was sized on the first input's weight, script length and dust
    // limit, so it describes any input sharing those three. A set mixing them is
    // fanned out instead, where a single script pays every output, rather than
    // passed through to fail on whichever transaction drew the odd input. A fresh
    // address per UTXO is not mixing: the three are what the sizing reads.
    let funding_kind = |input: &CpfpInput| {
        let script = &input.witness_utxo.script_pubkey;
        (
            input.signed_input_weight,
            script.len(),
            script.minimal_non_dust().to_sat(),
        )
    };
    let uniform = inputs
        .first()
        .map(&funding_kind)
        .is_some_and(|reference| inputs.iter().all(|input| funding_kind(input) == reference));
    let covers_in_order = inputs.len() == required.len()
        && inputs
            .iter()
            .zip(&required)
            .all(|(input, need)| input.witness_utxo.value.to_sat() >= *need);
    let (assigned, fan_out_psbt) = if uniform && covers_in_order {
        (inputs, None)
    } else if uniform && let Some(by_size) = cover_by_size(&inputs, &required) {
        let mut slots: Vec<Option<CpfpInput>> = inputs.into_iter().map(Some).collect();
        (
            by_size
                .into_iter()
                .map(|index| slots[index].take().expect("each input is used once"))
                .collect(),
            None,
        )
    } else {
        // The surplus is split by the same amounts, so the transactions that cost
        // the most get the most headroom for a higher-rate resume.
        let (psbt, fanned) =
            build_fan_out_psbt(&inputs, &required, &required, fee_rate_sat_per_kw)?;
        (fanned, Some(psbt))
    };

    let mut remaining = assigned.into_iter();
    let per_branch_funding = node_funding
        .iter()
        .map(|(leaf_id, funding)| {
            (
                leaf_id.clone(),
                remaining.by_ref().take(funding.len()).collect(),
            )
        })
        .collect();
    Ok((per_branch_funding, fan_out_psbt))
}

/// Pairs each amount with the smallest unused input that covers it, largest
/// amount first: the index into `inputs` to use for each entry of `required`,
/// in `required` order. The inputs share one funding kind, so which pays which
/// fee is a matter of size alone, and taking the tightest fit leaves any larger
/// surplus input untouched. `None` when no pairing covers, which the greedy
/// pass decides exactly: an amount that the tightest fit cannot cover is one no
/// other assignment could cover without taking an input a larger amount needs.
fn cover_by_size(inputs: &[CpfpInput], required: &[u64]) -> Option<Vec<usize>> {
    let value = |i: usize| inputs[i].witness_utxo.value.to_sat();
    let mut by_value: Vec<usize> = (0..inputs.len()).collect();
    by_value.sort_by_key(|&i| value(i));
    let mut amounts: Vec<(usize, u64)> = required.iter().copied().enumerate().collect();
    amounts.sort_by_key(|(_, need)| cmp::Reverse(*need));

    let mut used = vec![false; inputs.len()];
    let mut chosen = vec![0; required.len()];
    for (slot, need) in amounts {
        let fit = by_value
            .iter()
            .position(|&i| !used[i] && value(i) >= need)?;
        used[by_value[fit]] = true;
        chosen[slot] = by_value[fit];
    }
    Some(chosen)
}

/// A chain-independent unilateral-exit quote: which leaves would exit and the
/// funding they need, sized from the funding kind's weight with no actual UTXOs.
pub struct UnilateralExitQuote {
    pub selected_leaves: Vec<UnilateralExitSelectedLeaf>,
    /// Per-branch funding to avoid a fan-out: (leaf id, minimum sats). Empty
    /// unless the quote is for [`CpfpFundingShape::PerBranch`].
    pub per_branch_funding: Vec<(TreeNodeId, u64)>,
    /// Per-transaction funding to avoid a fan-out. Empty unless the quote is for
    /// [`CpfpFundingShape::PerNode`].
    pub per_node_funding: Vec<UnilateralExitNodeFunding>,
    pub single_utxo_funding_sat: u64,
    pub fanout_fee_sat: u64,
    pub total_fee_sat: u64,
}

/// Like [`plan_unilateral_exit`] but sizes fees from a funding kind's weight with no actual
/// UTXOs and never rejects on budget: it only reports the funding required.
#[allow(clippy::too_many_arguments)]
pub fn quote_unilateral_exit(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    filter: UnilateralExitLeafFilter,
    funding_input_weight: u64,
    funding_output_script_len: usize,
    change_dust_limit: u64,
    funding_shape: CpfpFundingShape,
    fee_rate_sat_per_kw: u64,
    destination_script_len: usize,
) -> Result<UnilateralExitQuote, ServiceError> {
    let params = leaf_cost_params(
        Weight::from_wu(funding_input_weight),
        funding_output_script_len,
        destination_script_len,
        fee_rate_sat_per_kw,
        funding_shape,
    );

    let selected = evaluate_unilateral_exit_leaf_costs(tree_nodes, leaf_ids, &params, filter)?;
    if selected.is_empty() {
        return Ok(UnilateralExitQuote {
            selected_leaves: vec![],
            per_branch_funding: vec![],
            per_node_funding: vec![],
            single_utxo_funding_sat: 0,
            fanout_fee_sat: 0,
            total_fee_sat: 0,
        });
    }

    // Each shape names exactly one list to fund, and a single UTXO is fanned out
    // into one output per entry of it: a branch under per-branch funding, a
    // transaction under per-node funding.
    let (per_branch_funding, node_funding): (Vec<(TreeNodeId, u64)>, Vec<_>) = match funding_shape {
        CpfpFundingShape::PerBranch => (
            selected
                .iter()
                .map(|l| (l.id.clone(), branch_required_funding(l, change_dust_limit)))
                .collect(),
            Vec::new(),
        ),
        CpfpFundingShape::PerNode => (
            Vec::new(),
            per_node_funding(tree_nodes, &selected, &params, change_dust_limit),
        ),
    };
    let funded_total: u64 = per_branch_funding
        .iter()
        .map(|(_, sat)| *sat)
        .chain(
            node_funding
                .iter()
                .flat_map(|(_, funding)| funding.iter().map(|n| n.funding_sat)),
        )
        .fold(0u64, u64::saturating_add);
    let fan_out_outputs =
        per_branch_funding.len() + node_funding.iter().map(|(_, f)| f.len()).sum::<usize>();
    let sum_estimated: u64 = selected
        .iter()
        .map(|l| l.estimated_cost)
        .fold(0u64, u64::saturating_add);

    let fanout_fee_sat = if fan_out_outputs <= 1 {
        0
    } else {
        fan_out_fee(
            Weight::from_wu(funding_input_weight),
            funding_output_script_len,
            fan_out_outputs,
            fee_rate_sat_per_kw,
        )
    };

    Ok(UnilateralExitQuote {
        single_utxo_funding_sat: funded_total.saturating_add(fanout_fee_sat),
        total_fee_sat: sum_estimated.saturating_add(fanout_fee_sat),
        selected_leaves: selected,
        per_branch_funding,
        per_node_funding: node_funding.into_iter().flat_map(|(_, f)| f).collect(),
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
pub struct UnilateralExitSelectedLeaf {
    pub id: TreeNodeId,
    pub value: u64,
    /// Marginal exit cost (CPFP fees + sweep input fee). Order-dependent: a shared
    /// ancestor is charged to the first selected leaf reaching it, not a fair share.
    pub estimated_cost: u64,
    /// CPFP package fees only, without the sweep input fee: the physical funding
    /// floor, since the sweep is paid from the swept value rather than the funding
    /// UTXO. Always `<= estimated_cost`.
    pub cpfp_cost: u64,
}

pub struct UnilateralExitLeafCostParams {
    /// Weight of the first CPFP child's inputs in a leaf's chain. Equal to
    /// `single_cpfp_input_weight` under [`CpfpFundingShape::PerNode`], where every
    /// child is funded by one UTXO of its own.
    pub initial_cpfp_input_weight: Weight,
    /// Weight of the single input every other child is funded by.
    pub single_cpfp_input_weight: Weight,
    pub change_script_len: usize,
    pub destination_script_len: usize,
    pub fee_rate_sat_per_kw: u64,
    pub funding_shape: CpfpFundingShape,
}

/// Sats a branch's funding inputs must provide: its marginal exit cost plus the
/// terminal CPFP-change output, which the sweep later consumes so it must clear
/// dust. Single source of truth every affordability gate and the quote share.
#[inline]
pub fn branch_required_funding(leaf: &UnilateralExitSelectedLeaf, change_dust_limit: u64) -> u64 {
    leaf.estimated_cost.saturating_add(change_dust_limit)
}

/// Sats the UTXO fee-bumping one transaction must provide: its CPFP package fee
/// plus the non-dust change [`build_cpfp_child`] always writes.
fn bumped_tx_funding(
    parent_weight: Weight,
    params: &UnilateralExitLeafCostParams,
    change_dust_limit: u64,
) -> u64 {
    compute_cpfp_package_fee(
        parent_weight,
        params.single_cpfp_input_weight,
        params.change_script_len,
        params.fee_rate_sat_per_kw,
    )
    .saturating_add(change_dust_limit)
}

/// The transactions a per-node exit may fee-bump, in the order their funding
/// UTXOs are consumed: for each selected leaf in turn, the ancestors no earlier
/// leaf already funds, root to leaf, then that leaf's refund. Structural, since
/// only the chain says which still need a child and it is read after the funding
/// is gathered; a UTXO for one that does not is left unspent.
pub fn per_node_funding(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    selected: &[UnilateralExitSelectedLeaf],
    params: &UnilateralExitLeafCostParams,
    change_dust_limit: u64,
) -> Vec<(TreeNodeId, Vec<UnilateralExitNodeFunding>)> {
    let mut per_leaf = Vec::with_capacity(selected.len());
    let mut covered_txids: HashSet<Txid> = HashSet::new();
    for leaf in selected {
        let Some(leaf_node) = tree_nodes.get(&leaf.id) else {
            continue;
        };
        let Ok(ancestors) = walk_unilateral_exit_chain(tree_nodes, leaf_node) else {
            continue;
        };
        let mut funding = Vec::with_capacity(ancestors.len() + 1);
        for ancestor in &ancestors {
            let txid = ancestor.node_tx.compute_txid();
            if !covered_txids.insert(txid) {
                continue;
            }
            funding.push(UnilateralExitNodeFunding {
                leaf_id: leaf.id.clone(),
                node_id: ancestor.id.clone(),
                txid,
                refund: false,
                funding_sat: bumped_tx_funding(
                    ancestor.node_tx.weight(),
                    params,
                    change_dust_limit,
                ),
            });
        }
        if let Some(refund_tx) = &leaf_node.refund_tx {
            funding.push(UnilateralExitNodeFunding {
                leaf_id: leaf.id.clone(),
                node_id: leaf.id.clone(),
                txid: refund_tx.compute_txid(),
                refund: true,
                funding_sat: bumped_tx_funding(refund_tx.weight(), params, change_dust_limit),
            });
        }
        per_leaf.push((leaf.id.clone(), funding));
    }
    per_leaf
}

/// The CPFP fee floor for funding a branch whose first child is fed all of
/// `branch_inputs`, as the build does: the first child is sized on their combined
/// weight, each chained child on the first input. This is the physical floor
/// `build_cpfp_child` enforces, independent of the sweep (paid from the swept
/// value, not the funding UTXO). `None` only when the leaf cannot be costed.
fn first_child_cpfp_floor(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_id: &TreeNodeId,
    branch_inputs: &[CpfpInput],
    destination_script_len: usize,
    fee_rate_sat_per_kw: u64,
) -> Option<u64> {
    let first = branch_inputs.first()?;
    let total_input_weight = branch_inputs
        .iter()
        .map(|i| i.signed_input_weight)
        .fold(0u64, u64::saturating_add);
    let params = UnilateralExitLeafCostParams {
        initial_cpfp_input_weight: Weight::from_wu(total_input_weight),
        single_cpfp_input_weight: Weight::from_wu(first.signed_input_weight),
        change_script_len: first.witness_utxo.script_pubkey.len(),
        destination_script_len,
        fee_rate_sat_per_kw,
        funding_shape: CpfpFundingShape::PerBranch,
    };
    evaluate_unilateral_exit_leaf_costs(
        tree_nodes,
        std::slice::from_ref(leaf_id),
        &params,
        UnilateralExitLeafFilter::All,
    )
    .ok()
    .and_then(|leaves| leaves.into_iter().next())
    .map(|leaf| leaf.cpfp_cost)
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
pub enum UnilateralExitLeafFilter {
    /// Keep every requested leaf, even when its exit cost exceeds its value.
    All,
    /// Keep only leaves whose value strictly exceeds their marginal exit cost.
    ProfitableOnly,
}

/// A leaf the caller named ([`UnilateralExitLeafFilter::All`]) that can't be exited is an
/// error; under [`UnilateralExitLeafFilter::ProfitableOnly`] it is warned and skipped.
fn report_unexitable(
    filter: UnilateralExitLeafFilter,
    leaf_id: &TreeNodeId,
    reason: &str,
) -> Result<(), ServiceError> {
    if filter == UnilateralExitLeafFilter::All {
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
pub fn evaluate_unilateral_exit_leaf_costs(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    params: &UnilateralExitLeafCostParams,
    filter: UnilateralExitLeafFilter,
) -> Result<Vec<UnilateralExitSelectedLeaf>, ServiceError> {
    let mut leaves: Vec<(&TreeNodeId, &TreeNode)> = Vec::with_capacity(leaf_ids.len());
    for id in leaf_ids {
        match tree_nodes.get(id) {
            Some(node) => leaves.push((id, node)),
            None => report_unexitable(filter, id, "not found in the tree node map")?,
        }
    }
    leaves.sort_by(|a, b| b.1.value.cmp(&a.1.value).then_with(|| a.0.cmp(b.0)));

    let mut selected: Vec<UnilateralExitSelectedLeaf> = Vec::new();
    let mut covered_txids: HashSet<bitcoin::Txid> = HashSet::new();

    for (leaf_id, leaf) in &leaves {
        let Some(refund_tx) = &leaf.refund_tx else {
            report_unexitable(filter, leaf_id, "no refund transaction")?;
            continue;
        };
        let ancestors = match walk_unilateral_exit_chain(tree_nodes, leaf) {
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

        // Per-node funding leaves every child's change on the funding script
        // instead of folding a branch's last one into the sweep, so a leaf brings
        // only its refund input.
        let per_leaf_input_weight = match params.funding_shape {
            CpfpFundingShape::PerNode => p2tr_key_path_input_weight(),
            CpfpFundingShape::PerBranch => {
                p2tr_key_path_input_weight() + params.single_cpfp_input_weight
            }
        };
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

        if filter == UnilateralExitLeafFilter::All || leaf.value > total_marginal_cost {
            selected.push(UnilateralExitSelectedLeaf {
                id: (*leaf_id).clone(),
                value: leaf.value,
                estimated_cost: total_marginal_cost,
                cpfp_cost,
            });
            for ancestor in &ancestors {
                covered_txids.insert(ancestor.node_tx.compute_txid());
            }
        }
    }

    Ok(selected)
}

/// Partitions the CPFP inputs across branches so each is funded by its own
/// subset, avoiding a fan-out. Greedy, costliest branch first, holding one input
/// in reserve per not-yet-funded branch. `None` when no partition fits.
///
/// Returned in `selected_leaves` order (value-descending, as
/// [`evaluate_unilateral_exit_leaf_costs`] emits), not the internal greedy order. The
/// funding sizes each branch assuming a shared ancestor is charged to the first
/// leaf in value order; `build_exit` charges it to the first branch it iterates.
/// Returning in value order keeps those two the same branch, so no branch is
/// left short of a shared ancestor's fee and fails its dust check.
pub fn assign_inputs_to_leaves(
    inputs: &[CpfpInput],
    selected_leaves: &[UnilateralExitSelectedLeaf],
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
    let mut sorted_leaves: Vec<&UnilateralExitSelectedLeaf> = selected_leaves.iter().collect();
    sorted_leaves.sort_by(|a, b| {
        b.estimated_cost
            .cmp(&a.estimated_cost)
            .then_with(|| a.id.cmp(&b.id))
    });

    let leaf_count = sorted_leaves.len();
    let mut assigned_by_leaf: HashMap<TreeNodeId, Vec<CpfpInput>> =
        HashMap::with_capacity(leaf_count);
    for (i, leaf) in sorted_leaves.iter().enumerate() {
        let required = branch_required_funding(leaf, change_dust_limit);
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

/// Whether every branch of `assignment` funds its first CPFP child. `build_exit`
/// feeds a branch's whole input set to that first child, sizing its fee on their
/// combined weight, so a branch short of that would pass the plan then fail
/// `build_cpfp_child`; rejecting the assignment falls back to a fan-out (one
/// output, so one input, per branch). Each branch is costed on its own inputs and
/// its own change-script dust, so a branch funded by an input heavier than the
/// reference `inputs[0]` (a mixed or Custom funding kind) is gated correctly, not
/// just the multi-input branches. Costs each branch's chain in isolation: exact
/// for independent branches, conservative when they share an ancestor (charged to
/// every branch here, to only one in the build).
fn assignment_covers_first_child(
    assignment: &[(TreeNodeId, Vec<CpfpInput>)],
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    destination_script_len: usize,
    fee_rate_sat_per_kw: u64,
) -> bool {
    assignment.iter().all(|(leaf_id, branch_inputs)| {
        let Some(first) = branch_inputs.first() else {
            return false;
        };
        let dust = first.witness_utxo.script_pubkey.minimal_non_dust().to_sat();
        let available = branch_inputs
            .iter()
            .map(|i| i.witness_utxo.value.to_sat())
            .fold(0u64, u64::saturating_add);
        match first_child_cpfp_floor(
            tree_nodes,
            leaf_id,
            branch_inputs,
            destination_script_len,
            fee_rate_sat_per_kw,
        ) {
            Some(cpfp_cost) => available >= cpfp_cost.saturating_add(dust),
            None => false,
        }
    })
}

/// Builds an unsigned fan-out PSBT paying `output_values` to the funding script,
/// one output each. No change output: surplus input value is folded into those
/// outputs in proportion to `surplus_weights`, where it doubles as fee headroom
/// for a higher-fee resume that reuses this confirmed fan-out. RBF-signaled so an
/// unconfirmed fan-out can be replaced.
pub fn build_fan_out_psbt(
    inputs: &[CpfpInput],
    output_values: &[u64],
    surplus_weights: &[u64],
    fee_rate_sat_per_kw: u64,
) -> Result<(psbt::Psbt, Vec<CpfpInput>), ServiceError> {
    if inputs.is_empty() {
        return Err(ServiceError::ValidationError(
            "fan-out: at least one CPFP input is required".to_string(),
        ));
    }
    if output_values.is_empty() {
        return Err(ServiceError::ValidationError(
            "fan-out: at least one output is required".to_string(),
        ));
    }
    if output_values.len() != surplus_weights.len() {
        return Err(ServiceError::ValidationError(
            "fan-out: one surplus weight per output is required".to_string(),
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

    let required_total: u64 = output_values
        .iter()
        .copied()
        .fold(0u64, u64::saturating_add);

    let fee_no_change = fan_out_fee(
        Weight::from_wu(total_input_weight),
        script_pubkey.len(),
        output_values.len(),
        fee_rate_sat_per_kw,
    );

    if total_input_value < required_total.saturating_add(fee_no_change) {
        return Err(ServiceError::InsufficientCpfpBudget {
            required_sat: required_total.saturating_add(fee_no_change),
        });
    }

    let surplus = total_input_value
        .saturating_sub(required_total)
        .saturating_sub(fee_no_change);
    let mut output_values: Vec<u64> = output_values.to_vec();
    if surplus > 0 {
        let weight_total: u128 = surplus_weights.iter().copied().map(u128::from).sum();
        let mut distributed: u64 = 0;
        for (idx, weight) in surplus_weights.iter().enumerate() {
            // checked_div guards weight_total == 0 (all weights zero): no share is
            // distributed and the whole surplus falls to the first output below.
            let share = u128::from(surplus)
                .saturating_mul(u128::from(*weight))
                .checked_div(weight_total)
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

    let fan_out_inputs: Vec<CpfpInput> = output_values
        .iter()
        .enumerate()
        .map(|(idx, &value)| CpfpInput {
            outpoint: OutPoint {
                txid,
                vout: idx as u32,
            },
            witness_utxo: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: script_pubkey.clone(),
            },
            signed_input_weight,
        })
        .collect();

    trace!(
        inputs = inputs.len(),
        outputs = fan_out_inputs.len(),
        total_input_value,
        fee = fee_no_change,
        "build_fan_out_psbt"
    );
    Ok((psbt_unsigned, fan_out_inputs))
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

    let rbf_sequence = Sequence::ENABLE_RBF_NO_LOCKTIME;
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
        // The authoritative funding check: computed from the real inputs, this is
        // where a branch the plan sized on one input but funded with several
        // surfaces. The floor is the fee plus a non-dust change.
        return Err(ServiceError::InsufficientCpfpBudget {
            required_sat: fee_amount.saturating_add(dust_limit),
        });
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

        fn node_map(nodes: &[&TreeNode]) -> HashMap<TreeNodeId, TreeNode> {
            nodes.iter().map(|n| (n.id.clone(), (*n).clone())).collect()
        }

        fn chain_ids(chain: &[&TreeNode]) -> Vec<String> {
            chain.iter().map(|n| n.id.to_string()).collect()
        }

        #[test_all]
        fn walks_leaf_to_root() {
            let root = node(ROOT, None, TreeNodeStatus::Available);
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Splitted);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);
            let map = node_map(&[&root, &mid, &leaf]);

            let chain = walk_unilateral_exit_chain(&map, &leaf).unwrap();

            assert_eq!(chain_ids(&chain), vec![ROOT, MID, LEAF]);
        }

        #[test_all]
        fn walks_through_split_locked_parent() {
            let root = node(ROOT, None, TreeNodeStatus::Available);
            let mid = node(MID, Some(ROOT), TreeNodeStatus::SplitLocked);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);
            let map = node_map(&[&root, &mid, &leaf]);

            let chain = walk_unilateral_exit_chain(&map, &leaf).unwrap();

            assert_eq!(chain_ids(&chain), vec![ROOT, MID, LEAF]);
        }

        #[test_all]
        fn stops_on_non_exit_status() {
            let root = node(ROOT, None, TreeNodeStatus::Available);
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Exited);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);
            let map = node_map(&[&root, &mid, &leaf]);

            let chain = walk_unilateral_exit_chain(&map, &leaf).unwrap();

            assert_eq!(chain_ids(&chain), vec![LEAF]);
        }

        #[test_all]
        fn missing_parent_names_the_gap() {
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Splitted);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);
            let map = node_map(&[&mid, &leaf]);

            let missing = walk_unilateral_exit_chain(&map, &leaf).unwrap_err();

            assert_eq!(missing.to_string(), ROOT);
        }

        #[test_all]
        fn cycle_returns_a_node_the_map_holds() {
            let root = node(ROOT, Some(MID), TreeNodeStatus::Available);
            let mid = node(MID, Some(ROOT), TreeNodeStatus::Available);
            let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);
            let map = node_map(&[&root, &mid, &leaf]);

            let revisited = walk_unilateral_exit_chain(&map, &leaf).unwrap_err();

            // A caller tells a cycle from a gap by the returned id being stored.
            assert!(map.contains_key(&revisited));
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

        fn selected(id: &str, value: u64, cost: u64) -> UnilateralExitSelectedLeaf {
            UnilateralExitSelectedLeaf {
                id: TreeNodeId::from_str(id).unwrap(),
                value,
                estimated_cost: cost,
                cpfp_cost: cost,
            }
        }

        #[test_all]
        fn cpfp_package_fee_is_exact() {
            let (parent, input) = (Weight::from_wu(400), Weight::from_wu(272));
            assert_eq!(compute_cpfp_package_fee(parent, input, 22, 500), 502);
            assert_eq!(compute_cpfp_package_fee(parent, input, 22, 1000), 1003);
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
            assert_eq!(got[0].0, leaves[0].id);
            assert_eq!(
                got[0]
                    .1
                    .iter()
                    .map(|i| i.witness_utxo.value.to_sat())
                    .collect::<Vec<_>>(),
                vec![10_000]
            );
            assert_eq!(got[1].0, leaves[1].id);
            assert_eq!(
                got[1]
                    .1
                    .iter()
                    .map(|i| i.witness_utxo.value.to_sat())
                    .collect::<Vec<_>>(),
                vec![5_000]
            );
        }

        #[test_all]
        fn assign_inputs_returns_value_order_when_richest_is_not_costliest() {
            // Leaf "a" is richer but cheaper; "b" is poorer but costlier. Input
            // is value order (a, b); the greedy pass runs costliest-first (b, a).
            let inputs = vec![cpfp_input(10_000, 0), cpfp_input(5_000, 1)];
            let leaves = vec![selected("a", 50_000, 1_000), selected("b", 20_000, 3_000)];
            let got = assign_inputs_to_leaves(&inputs, &leaves, 330).expect("should fit");
            // Returned in value order, matching evaluate_unilateral_exit_leaf_costs.
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
            // b (cost 1_500 + 330 dust) needs both small inputs; a takes the 10_000.
            assert_eq!(
                got[0]
                    .1
                    .iter()
                    .map(|i| i.witness_utxo.value.to_sat())
                    .collect::<Vec<_>>(),
                vec![10_000]
            );
            assert_eq!(
                got[1]
                    .1
                    .iter()
                    .map(|i| i.witness_utxo.value.to_sat())
                    .collect::<Vec<_>>(),
                vec![1_000, 1_000]
            );
        }

        #[test_all]
        fn assign_inputs_rejects_fewer_inputs_than_branches() {
            let leaves = vec![selected("a", 50_000, 3_000), selected("b", 20_000, 1_000)];
            assert!(assign_inputs_to_leaves(&[cpfp_input(10_000, 0)], &leaves, 330).is_none());
        }

        #[test_all]
        fn assign_inputs_funding_boundary_is_exact() {
            // Per-branch requirement is estimated_cost + dust: a needs 3_330, b 1_330.
            let leaves = vec![selected("a", 50_000, 3_000), selected("b", 20_000, 1_000)];
            let exact = vec![cpfp_input(3_330, 0), cpfp_input(1_330, 1)];
            assert!(assign_inputs_to_leaves(&exact, &leaves, 330).is_some());
            let short = vec![cpfp_input(3_330, 0), cpfp_input(1_329, 1)];
            assert!(assign_inputs_to_leaves(&short, &leaves, 330).is_none());
        }

        #[test_all]
        fn assignment_covers_first_child_gates_multi_input_branch() {
            // A branch of four inputs: build feeds all four to the first CPFP child,
            // so its fee is sized on their combined weight, above the one-input
            // estimate assign_inputs_to_leaves used. The guard rejects funding short
            // of that (the plan then falls back to a one-output-per-branch fan-out).
            let leaf = leaf_node_n("a", 1_000_000, 1);
            let leaf_id = leaf.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(leaf_id.clone(), leaf)].into_iter().collect();

            let probe = cpfp_input(0, 0);
            let change_len = probe.witness_utxo.script_pubkey.len();
            let dust = probe.witness_utxo.script_pubkey.minimal_non_dust().to_sat();
            let input_weight = probe.signed_input_weight;

            // The exact first-child floor for four inputs, as the guard computes it.
            let cpfp_cost = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                std::slice::from_ref(&leaf_id),
                &UnilateralExitLeafCostParams {
                    initial_cpfp_input_weight: Weight::from_wu(4 * input_weight),
                    single_cpfp_input_weight: Weight::from_wu(input_weight),
                    change_script_len: change_len,
                    destination_script_len: change_len,
                    fee_rate_sat_per_kw: 250,
                    funding_shape: CpfpFundingShape::PerBranch,
                },
                UnilateralExitLeafFilter::All,
            )
            .unwrap()[0]
                .cpfp_cost;
            let floor = cpfp_cost + dust;

            let four = |total: u64| {
                let each = total / 4;
                vec![(
                    leaf_id.clone(),
                    vec![
                        cpfp_input(each, 0),
                        cpfp_input(each, 1),
                        cpfp_input(each, 2),
                        cpfp_input(total - 3 * each, 3),
                    ],
                )]
            };

            assert!(assignment_covers_first_child(
                &four(floor),
                &nodes,
                change_len,
                250
            ));
            assert!(!assignment_covers_first_child(
                &four(floor - 1),
                &nodes,
                change_len,
                250
            ));
            // A one-input branch is gated on its own weight too: funded above its
            // one-input floor it is covered.
            let one_floor = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                std::slice::from_ref(&leaf_id),
                &UnilateralExitLeafCostParams {
                    initial_cpfp_input_weight: Weight::from_wu(input_weight),
                    single_cpfp_input_weight: Weight::from_wu(input_weight),
                    change_script_len: change_len,
                    destination_script_len: change_len,
                    fee_rate_sat_per_kw: 250,
                    funding_shape: CpfpFundingShape::PerBranch,
                },
                UnilateralExitLeafFilter::All,
            )
            .unwrap()[0]
                .cpfp_cost
                + dust;
            let one = vec![(leaf_id.clone(), vec![cpfp_input(one_floor, 0)])];
            assert!(assignment_covers_first_child(&one, &nodes, change_len, 250));
            let one_short = vec![(leaf_id.clone(), vec![cpfp_input(one_floor - 1, 0)])];
            assert!(!assignment_covers_first_child(
                &one_short, &nodes, change_len, 250
            ));

            // A single input heavier than the reference kind (a Custom funding kind)
            // is gated on its real weight, not inputs[0]'s: an input whose value only
            // meets the light-weight floor is rejected, so the plan can fan out.
            let mut heavy = cpfp_input(one_floor, 0);
            heavy.signed_input_weight = 4 * input_weight;
            let heavy_branch = vec![(leaf_id.clone(), vec![heavy])];
            assert!(!assignment_covers_first_child(
                &heavy_branch,
                &nodes,
                change_len,
                250
            ));
        }

        /// The two branch outputs of the leaves used by the fan-out tests, and the
        /// costs their surplus is split by.
        fn two_branch_fan_out() -> (Vec<u64>, Vec<u64>) {
            let leaves = [selected("a", 50_000, 3_000), selected("b", 40_000, 2_000)];
            (
                leaves
                    .iter()
                    .map(|l| branch_required_funding(l, 330))
                    .collect(),
                leaves.iter().map(|l| l.estimated_cost).collect(),
            )
        }

        #[test_all]
        fn fan_out_emits_one_output_per_branch_and_is_deterministic() {
            let inputs = vec![cpfp_input(100_000, 0)];
            let (values, weights) = two_branch_fan_out();
            let (psbt, fanned) = build_fan_out_psbt(&inputs, &values, &weights, 250).unwrap();
            assert_eq!(psbt.unsigned_tx.output.len(), 2);
            assert_eq!(fanned.len(), 2);
            // 100_000 - 141 fee - 5_660 base is split by cost (3:2), remainder to a.
            assert_eq!(fanned[0].witness_utxo.value.to_sat(), 59_850);
            assert_eq!(fanned[1].witness_utxo.value.to_sat(), 40_009);
            let out_total: u64 = psbt
                .unsigned_tx
                .output
                .iter()
                .map(|o| o.value.to_sat())
                .sum();
            assert_eq!(out_total, 100_000 - 141);
            assert_eq!(fanned[0].outpoint.vout, 0);
            assert_eq!(fanned[1].outpoint.vout, 1);
            assert_eq!(
                psbt.unsigned_tx.input[0].sequence,
                Sequence::ENABLE_RBF_NO_LOCKTIME
            );
            let (psbt2, _) = build_fan_out_psbt(&inputs, &values, &weights, 250).unwrap();
            assert_eq!(
                psbt.unsigned_tx.compute_txid(),
                psbt2.unsigned_tx.compute_txid()
            );
        }

        #[test_all]
        fn fan_out_funding_boundary_is_exact() {
            // base 5_660 (two branches at cost + 330 dust) + 141 fan-out fee = 5_801.
            let (values, weights) = two_branch_fan_out();
            assert!(build_fan_out_psbt(&[cpfp_input(5_801, 0)], &values, &weights, 250).is_ok());
            assert!(build_fan_out_psbt(&[cpfp_input(5_800, 0)], &values, &weights, 250).is_err());
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
            assert_eq!(child.fee_sat, 872);
            assert_eq!(child.change_input.witness_utxo.value.to_sat(), 10_000 - 872);
        }

        #[test_all]
        fn cpfp_child_dust_boundary_is_exact() {
            let parent = create_test_transaction_with_anchor();
            let dust = test_script().minimal_non_dust().to_sat();
            let fee = compute_cpfp_package_fee(parent.weight(), Weight::from_wu(272), 22, 1250);
            let exact = build_cpfp_child(&parent, &[cpfp_input(fee + dust, 0)], 1250).unwrap();
            assert_eq!(exact.change_input.witness_utxo.value.to_sat(), dust);

            // One sat under the floor: the gate rejects with the exact funding the
            // input needed (fee + non-dust change), so the caller can top up precisely.
            match build_cpfp_child(&parent, &[cpfp_input(fee + dust - 1, 0)], 1250).map(|_| ()) {
                Err(ServiceError::InsufficientCpfpBudget { required_sat }) => {
                    assert_eq!(required_sat, fee + dust);
                }
                other => panic!("expected InsufficientCpfpBudget, got {other:?}"),
            }
        }

        #[test_all]
        fn cpfp_child_two_input_funding_boundary_is_exact() {
            // A branch the plan sized on one input but funded with two UTXOs pays
            // the higher two-input fee, so its real floor is fee_2 + dust, above the
            // one-input floor the plan assumed. build_cpfp_child is where that
            // shortfall surfaces, reported exactly.
            let parent = create_test_transaction_with_anchor();
            let dust = test_script().minimal_non_dust().to_sat();
            let fee_1 = compute_cpfp_package_fee(parent.weight(), Weight::from_wu(272), 22, 1250);
            let fee_2 = compute_cpfp_package_fee(parent.weight(), Weight::from_wu(544), 22, 1250);
            assert!(fee_2 > fee_1);

            let split =
                |total: u64| vec![cpfp_input(total / 2, 0), cpfp_input(total - total / 2, 1)];

            let ok = build_cpfp_child(&parent, &split(fee_2 + dust), 1250).unwrap();
            assert_eq!(ok.change_input.witness_utxo.value.to_sat(), dust);

            match build_cpfp_child(&parent, &split(fee_2 + dust - 1), 1250).map(|_| ()) {
                Err(ServiceError::InsufficientCpfpBudget { required_sat }) => {
                    assert_eq!(required_sat, fee_2 + dust);
                }
                other => panic!("expected InsufficientCpfpBudget, got {other:?}"),
            }
        }

        fn leaf_node(id: &str, value: u64) -> TreeNode {
            let mut n = crate::tree::tests::create_test_tree_node(id, value);
            n.node_tx = create_test_transaction_with_anchor();
            n.refund_tx = Some(create_test_transaction_with_anchor());
            n
        }

        /// A leaf whose node and refund txs are unique to `nonce`, so independent
        /// leaves don't collide on a shared txid (which the ancestor walk would
        /// treat as an already-covered ancestor).
        fn leaf_node_n(id: &str, value: u64, nonce: u32) -> TreeNode {
            let mut n = crate::tree::tests::create_test_tree_node(id, value);
            n.node_tx = anchor_tx_n(nonce);
            n.refund_tx = Some(anchor_tx_n(nonce + 1_000));
            n
        }

        fn cost_params() -> UnilateralExitLeafCostParams {
            UnilateralExitLeafCostParams {
                initial_cpfp_input_weight: Weight::from_wu(272),
                single_cpfp_input_weight: Weight::from_wu(272),
                change_script_len: 22,
                destination_script_len: 22,
                fee_rate_sat_per_kw: 250,
                funding_shape: CpfpFundingShape::PerBranch,
            }
        }

        #[test_all]
        fn select_auto_keeps_profitable_drops_unprofitable() {
            let node = leaf_node("leaf", 1_000_000);
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();

            let sel = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                std::slice::from_ref(&id),
                &cost_params(),
                UnilateralExitLeafFilter::ProfitableOnly,
            )
            .unwrap();
            assert_eq!(sel.len(), 1);

            let small = leaf_node("leaf", 10);
            let sid = small.id.clone();
            let small_nodes: HashMap<TreeNodeId, TreeNode> =
                [(sid.clone(), small)].into_iter().collect();
            let sel = evaluate_unilateral_exit_leaf_costs(
                &small_nodes,
                &[sid],
                &cost_params(),
                UnilateralExitLeafFilter::ProfitableOnly,
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
            let cost = evaluate_unilateral_exit_leaf_costs(
                &probe_nodes,
                &[pid],
                &cost_params(),
                UnilateralExitLeafFilter::All,
            )
            .unwrap()[0]
                .estimated_cost;
            assert_eq!(cost, 517);

            let at = leaf_node("leaf", cost);
            let at_id = at.id.clone();
            let at_nodes: HashMap<TreeNodeId, TreeNode> =
                [(at_id.clone(), at)].into_iter().collect();
            assert!(
                evaluate_unilateral_exit_leaf_costs(
                    &at_nodes,
                    &[at_id],
                    &cost_params(),
                    UnilateralExitLeafFilter::ProfitableOnly
                )
                .unwrap()
                .is_empty(),
                "a leaf worth exactly its exit cost must be dropped under Auto"
            );

            let above = leaf_node("leaf", cost + 1);
            let above_id = above.id.clone();
            let above_nodes: HashMap<TreeNodeId, TreeNode> =
                [(above_id.clone(), above)].into_iter().collect();
            let sel = evaluate_unilateral_exit_leaf_costs(
                &above_nodes,
                &[above_id],
                &cost_params(),
                UnilateralExitLeafFilter::ProfitableOnly,
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
            let sel = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                &[sid],
                &cost_params(),
                UnilateralExitLeafFilter::All,
            )
            .unwrap();
            assert_eq!(sel.len(), 1);
        }

        #[test_all]
        fn evaluate_unexitable_errors_under_all_but_skips_under_profitable_only() {
            let mut node = leaf_node("leaf", 1_000_000);
            node.refund_tx = None;
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();

            assert!(
                evaluate_unilateral_exit_leaf_costs(
                    &nodes,
                    std::slice::from_ref(&id),
                    &cost_params(),
                    UnilateralExitLeafFilter::All
                )
                .is_err()
            );
            let sel = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                &[id],
                &cost_params(),
                UnilateralExitLeafFilter::ProfitableOnly,
            )
            .unwrap();
            assert!(sel.is_empty());
        }

        const DUST: u64 = 330;

        #[test_all]
        fn quote_single_leaf_has_no_fanout_fee() {
            let node = leaf_node("leaf", 1_000_000);
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();

            let quote = quote_unilateral_exit(
                &nodes,
                &[id],
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                DUST,
                CpfpFundingShape::PerBranch,
                250,
                22,
            )
            .unwrap();
            assert_eq!(quote.selected_leaves.len(), 1);
            let est = quote.selected_leaves[0].estimated_cost;
            assert_eq!(est, 517);

            assert_eq!(quote.fanout_fee_sat, 0);
            assert_eq!(quote.per_branch_funding.len(), 1);
            assert_eq!(quote.per_branch_funding[0].1, est + DUST);
            assert_eq!(quote.single_utxo_funding_sat, est + DUST);
            assert_eq!(quote.total_fee_sat, est);
        }

        #[test_all]
        fn quote_two_leaves_adds_fanout_fee() {
            let a = leaf_node_n("a", 1_000_000, 1);
            let b = leaf_node_n("b", 1_000_000, 3);
            let (ida, idb) = (a.id.clone(), b.id.clone());
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(ida.clone(), a), (idb.clone(), b)].into_iter().collect();

            let quote = quote_unilateral_exit(
                &nodes,
                &[ida, idb],
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                DUST,
                CpfpFundingShape::PerBranch,
                250,
                22,
            )
            .unwrap();

            // a is the first selected, so it carries the initial CPFP input and a
            // full sweep input (517); b's sweep cost is only the incremental input
            // (476). The fan-out fee (141) is charged once over both branches.
            let est: Vec<u64> = quote
                .selected_leaves
                .iter()
                .map(|l| l.estimated_cost)
                .collect();
            assert_eq!(est, vec![517, 476]);
            assert_eq!(quote.fanout_fee_sat, 141);
            let funding: Vec<u64> = quote.per_branch_funding.iter().map(|(_, s)| *s).collect();
            assert_eq!(funding, vec![517 + DUST, 476 + DUST]);
            assert_eq!(quote.total_fee_sat, 517 + 476 + 141);
            assert_eq!(
                quote.single_utxo_funding_sat,
                (517 + DUST) + (476 + DUST) + 141
            );
        }

        #[test_all]
        fn plan_multi_leaf_funded_at_quote_amounts_needs_no_fan_out() {
            let a = leaf_node_n("a", 1_000_000, 1);
            let b = leaf_node_n("b", 1_000_000, 3);
            let (ida, idb) = (a.id.clone(), b.id.clone());
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(ida.clone(), a), (idb.clone(), b)].into_iter().collect();

            // Quote against the real script dust that plan_unilateral_exit derives
            // from the funding UTXO, so quote and plan size the branches identically.
            let dust = test_script().minimal_non_dust().to_sat();
            let quote = quote_unilateral_exit(
                &nodes,
                &[ida.clone(), idb.clone()],
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                dust,
                CpfpFundingShape::PerBranch,
                250,
                22,
            )
            .unwrap();
            let funding: Vec<u64> = quote.per_branch_funding.iter().map(|(_, s)| *s).collect();
            assert_eq!(funding, vec![811, 770]);

            let fund_at = |a_sat: u64, b_sat: u64| {
                plan_unilateral_exit(
                    nodes.clone(),
                    &[ida.clone(), idb.clone()],
                    UnilateralExitLeafFilter::ProfitableOnly,
                    vec![cpfp_input(a_sat, 0), cpfp_input(b_sat, 1)],
                    CpfpFundingShape::PerBranch,
                    250,
                    22,
                )
            };

            // Funded at exactly the quote: a clean one-UTXO-per-branch plan, no fan-out.
            let plan = fund_at(811, 770).unwrap();
            assert!(plan.fan_out_psbt.is_none());
            assert_eq!(plan.per_branch_funding.len(), 2);
            assert!(
                plan.per_branch_funding
                    .iter()
                    .all(|(_, ins)| ins.len() == 1)
            );

            // One sat short on either branch and the exit can no longer be funded.
            assert!(fund_at(810, 770).is_err());
            assert!(fund_at(811, 769).is_err());
        }

        #[test_all]
        fn plan_single_utxo_funding_amount_fans_out() {
            let a = leaf_node_n("a", 1_000_000, 1);
            let b = leaf_node_n("b", 1_000_000, 3);
            let (ida, idb) = (a.id.clone(), b.id.clone());
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(ida.clone(), a), (idb.clone(), b)].into_iter().collect();

            let dust = test_script().minimal_non_dust().to_sat();
            let quote = quote_unilateral_exit(
                &nodes,
                &[ida.clone(), idb.clone()],
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                dust,
                CpfpFundingShape::PerBranch,
                250,
                22,
            )
            .unwrap();
            // Two per-branch amounts (811 + 770) plus one fan-out fee (141).
            assert_eq!(quote.single_utxo_funding_sat, 1_722);

            let fund_one = |sat: u64| {
                plan_unilateral_exit(
                    nodes.clone(),
                    &[ida.clone(), idb.clone()],
                    UnilateralExitLeafFilter::ProfitableOnly,
                    vec![cpfp_input(sat, 0)],
                    CpfpFundingShape::PerBranch,
                    250,
                    22,
                )
            };

            // One UTXO can't fund two branches directly, so the plan fans out. At the
            // exact quote there is no surplus, so each branch output is its quoted amount.
            let plan = fund_one(quote.single_utxo_funding_sat).unwrap();
            assert!(plan.fan_out_psbt.is_some());
            let branch_outputs: Vec<u64> = plan
                .per_branch_funding
                .iter()
                .map(|(_, ins)| ins[0].witness_utxo.value.to_sat())
                .collect();
            assert_eq!(branch_outputs, vec![811, 770]);

            // One sat under the quoted single-UTXO amount and the fan-out can't be funded.
            assert!(fund_one(quote.single_utxo_funding_sat - 1).is_err());
        }

        #[test_all]
        fn plan_single_leaf_funding_boundary_is_exact() {
            let a = leaf_node_n("a", 1_000_000, 1);
            let id = a.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), a)].into_iter().collect();

            let dust = test_script().minimal_non_dust().to_sat();
            let quote = quote_unilateral_exit(
                &nodes,
                std::slice::from_ref(&id),
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                dust,
                CpfpFundingShape::PerBranch,
                250,
                22,
            )
            .unwrap();
            // One branch, no fan-out fee: the single-UTXO recommendation is the
            // branch's own, and reserves sweep-fee headroom over the hard floor.
            let recommended = quote.per_branch_funding[0].1;
            assert_eq!(quote.single_utxo_funding_sat, recommended);

            let fund = |sat: u64| {
                plan_unilateral_exit(
                    nodes.clone(),
                    std::slice::from_ref(&id),
                    UnilateralExitLeafFilter::ProfitableOnly,
                    vec![cpfp_input(sat, 0)],
                    CpfpFundingShape::PerBranch,
                    250,
                    22,
                )
            };

            // The plan's hard floor is build_cpfp_child's basis (CPFP fees + dust),
            // below the recommendation by the sweep fee the funding UTXO need not
            // cover (the sweep is paid from the swept value).
            let floor = match fund(0) {
                Err(ServiceError::InsufficientCpfpBudget { required_sat }) => required_sat,
                other => panic!("expected InsufficientCpfpBudget, got {other:?}"),
            };
            assert!(
                floor < recommended,
                "floor {floor} not below recommendation {recommended}"
            );

            // Exactly the floor plans; one sat short rejects up front with that floor.
            let plan = fund(floor).unwrap();
            assert!(plan.fan_out_psbt.is_none());
            assert_eq!(plan.per_branch_funding.len(), 1);
            match fund(floor - 1) {
                Err(ServiceError::InsufficientCpfpBudget { required_sat }) => {
                    assert_eq!(required_sat, floor);
                }
                other => panic!("expected InsufficientCpfpBudget, got {other:?}"),
            }
        }

        #[test_all]
        fn plan_falls_back_to_fan_out_when_a_branch_input_is_too_heavy() {
            // assign_inputs_to_leaves matches by value and would give each of two
            // leaves one UTXO. One UTXO is far heavier than the reference inputs[0]
            // (a Custom funding kind), so its own first-child fee exceeds its value:
            // assignment_covers_first_child rejects the assignment and the plan fans
            // out, rather than passing a plan build_cpfp_child would then fail.
            let a = leaf_node_n("a", 1_000_000, 1);
            let b = leaf_node_n("b", 1_000_000, 3);
            let (ida, idb) = (a.id.clone(), b.id.clone());
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(ida.clone(), a), (idb.clone(), b)].into_iter().collect();

            let dust = test_script().minimal_non_dust().to_sat();
            let quote = quote_unilateral_exit(
                &nodes,
                &[ida.clone(), idb.clone()],
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                dust,
                CpfpFundingShape::PerBranch,
                250,
                22,
            )
            .unwrap();
            // b's per-branch funding covers only a light (272 wu) first child.
            let b_required = quote.per_branch_funding[1].1;

            // inputs[0] is the light reference the params size on; a large light
            // input goes to branch a (also covering the fan-out surplus), and a heavy
            // input worth exactly b's light requirement goes to branch b.
            let mut heavy = cpfp_input(b_required, 1);
            heavy.signed_input_weight = 5_000;
            let plan = plan_unilateral_exit(
                nodes,
                &[ida, idb],
                UnilateralExitLeafFilter::ProfitableOnly,
                vec![cpfp_input(50_000, 0), heavy],
                CpfpFundingShape::PerBranch,
                250,
                22,
            )
            .unwrap();
            assert!(
                plan.fan_out_psbt.is_some(),
                "a branch funded by an over-heavy input must fall back to a fan-out"
            );
        }

        #[test_all]
        fn plan_three_leaves_two_utxos_fans_out() {
            // Case e: more leaves than funding UTXOs. The count check routes to a
            // multi-input fan-out (2 inputs, 3 outputs), a shape no single-UTXO
            // fan-out test exercises.
            let a = leaf_node_n("a", 1_000_000, 1);
            let b = leaf_node_n("b", 1_000_000, 3);
            let c = leaf_node_n("c", 1_000_000, 5);
            let (ida, idb, idc) = (a.id.clone(), b.id.clone(), c.id.clone());
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(ida.clone(), a), (idb.clone(), b), (idc.clone(), c)]
                    .into_iter()
                    .collect();

            // Two UTXOs, three leaves: unmatchable one-per-branch, so the plan fans
            // out. Funded generously so the fan-out itself is affordable.
            let plan = plan_unilateral_exit(
                nodes,
                &[ida, idb, idc],
                UnilateralExitLeafFilter::ProfitableOnly,
                vec![cpfp_input(10_000, 0), cpfp_input(10_000, 1)],
                CpfpFundingShape::PerBranch,
                250,
                22,
            )
            .unwrap();
            assert!(plan.fan_out_psbt.is_some());
            assert_eq!(plan.per_branch_funding.len(), 3);
            let fan_out = plan.fan_out_psbt.as_ref().unwrap();
            assert_eq!(fan_out.unsigned_tx.input.len(), 2);
            assert_eq!(fan_out.unsigned_tx.output.len(), 3);
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
                evaluate_unilateral_exit_leaf_costs(
                    nodes,
                    std::slice::from_ref(&leaf_id),
                    &cost_params(),
                    UnilateralExitLeafFilter::All,
                )
                .unwrap()[0]
                    .estimated_cost
            };

            // Available root: root + leaf + refund each bumped (175) plus the sweep
            // input (167). OnChain root is already paid, dropping its 175 bump.
            let cost_all = cost_of(&chain(TreeNodeStatus::Available));
            let cost_onchain = cost_of(&chain(TreeNodeStatus::OnChain));
            assert_eq!(cost_all, 692);
            assert_eq!(cost_onchain, 517);
        }

        #[test_all]
        fn quote_auto_drops_all_unprofitable_to_zero() {
            let node = leaf_node("leaf", 10);
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();

            let quote = quote_unilateral_exit(
                &nodes,
                &[id],
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                DUST,
                CpfpFundingShape::PerBranch,
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

        /// A leaf under a root, the smallest chain with an ancestor to fund.
        fn root_and_leaf(
            root_status: TreeNodeStatus,
        ) -> (HashMap<TreeNodeId, TreeNode>, TreeNodeId) {
            let mut root = crate::tree::tests::create_test_tree_node("root", 1_000_000);
            root.node_tx = anchor_tx_n(1);
            root.status = root_status;
            let mut leaf = crate::tree::tests::create_test_tree_node("leaf", 1_000_000);
            leaf.node_tx = anchor_tx_n(2);
            leaf.refund_tx = Some(anchor_tx_n(3));
            leaf.parent_node_id = Some(root.id.clone());
            let leaf_id = leaf.id.clone();
            (
                [(root.id.clone(), root), (leaf_id.clone(), leaf)]
                    .into_iter()
                    .collect(),
                leaf_id,
            )
        }

        /// Two leaves under one root, so the root is an ancestor of both.
        fn shared_root() -> (HashMap<TreeNodeId, TreeNode>, TreeNodeId, TreeNodeId) {
            let mut root = crate::tree::tests::create_test_tree_node("root", 2_000_000);
            root.node_tx = anchor_tx_n(1);
            let mut rich = crate::tree::tests::create_test_tree_node("rich", 1_000_000);
            rich.node_tx = anchor_tx_n(2);
            rich.refund_tx = Some(anchor_tx_n(3));
            rich.parent_node_id = Some(root.id.clone());
            let mut poor = crate::tree::tests::create_test_tree_node("poor", 900_000);
            poor.node_tx = anchor_tx_n(4);
            poor.refund_tx = Some(anchor_tx_n(5));
            poor.parent_node_id = Some(root.id.clone());
            let (rich_id, poor_id) = (rich.id.clone(), poor.id.clone());
            (
                [
                    (root.id.clone(), root),
                    (rich_id.clone(), rich),
                    (poor_id.clone(), poor),
                ]
                .into_iter()
                .collect(),
                rich_id,
                poor_id,
            )
        }

        fn per_node_params() -> UnilateralExitLeafCostParams {
            UnilateralExitLeafCostParams {
                funding_shape: CpfpFundingShape::PerNode,
                ..cost_params()
            }
        }

        fn costed(
            nodes: &HashMap<TreeNodeId, TreeNode>,
            leaf_ids: &[TreeNodeId],
        ) -> Vec<UnilateralExitSelectedLeaf> {
            evaluate_unilateral_exit_leaf_costs(
                nodes,
                leaf_ids,
                &per_node_params(),
                UnilateralExitLeafFilter::All,
            )
            .unwrap()
        }

        /// The amounts a per-node quote names for `leaf_id`'s branch, in order.
        fn named_amounts(
            nodes: &HashMap<TreeNodeId, TreeNode>,
            leaf_id: &TreeNodeId,
            dust: u64,
        ) -> Vec<u64> {
            per_node_funding(
                nodes,
                &costed(nodes, std::slice::from_ref(leaf_id)),
                &per_node_params(),
                dust,
            )
            .into_iter()
            .flat_map(|(_, funding)| funding.into_iter().map(|n| n.funding_sat))
            .collect()
        }

        #[test_all]
        fn per_node_funding_names_every_bumped_transaction() {
            // Funding a branch per node has to reach the ancestors too, not just the
            // leaf: a chain of two nodes needs three UTXOs, the last one the refund's.
            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let selected = costed(&nodes, std::slice::from_ref(&leaf_id));
            let funding = per_node_funding(&nodes, &selected, &per_node_params(), 294);

            assert_eq!(funding.len(), 1, "one branch");
            let (branch, entries) = &funding[0];
            assert_eq!(branch, &leaf_id);
            let named: Vec<(String, bool)> = entries
                .iter()
                .map(|n| (n.node_id.to_string(), n.refund))
                .collect();
            assert_eq!(
                named,
                vec![
                    ("root".to_string(), false),
                    ("leaf".to_string(), false),
                    ("leaf".to_string(), true),
                ],
                "root to leaf, then the leaf's refund"
            );
            // Each is its own package fee (175 at this rate) plus a non-dust change.
            assert!(entries.iter().all(|n| n.funding_sat == 175 + 294));
        }

        #[test_all]
        fn per_node_funding_names_a_shared_ancestor_once() {
            // A shared ancestor is broadcast once, so only one UTXO pays for it.
            // Funding it twice would leave the second branch's UTXO unspendable by
            // the exit and the caller short of the branch that does bump it.
            let (nodes, rich_id, poor_id) = shared_root();
            let selected = costed(&nodes, &[rich_id.clone(), poor_id.clone()]);
            let funding = per_node_funding(&nodes, &selected, &per_node_params(), 294);

            let root_entries: Vec<&TreeNodeId> = funding
                .iter()
                .flat_map(|(_, entries)| entries.iter())
                .filter(|n| n.node_id.to_string() == "root")
                .map(|n| &n.leaf_id)
                .collect();
            assert_eq!(root_entries.len(), 1, "the root is funded once");
            assert_eq!(
                root_entries[0], &rich_id,
                "charged to the first leaf in selection order"
            );
            // The richer leaf brings the root, its own node and its refund; the other
            // brings only its own node and refund.
            let sizes: Vec<usize> = funding.iter().map(|(_, e)| e.len()).collect();
            assert_eq!(sizes, vec![3, 2]);
        }

        #[test_all]
        fn per_node_funding_names_an_onchain_ancestor_too() {
            // Only the chain says which transactions still need a child, and it is
            // read after the funding is gathered. Leaving out the ones the operators
            // call on-chain would leave the build with a node to bump and no UTXO
            // named for it, which no amount of extra funding could answer.
            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::OnChain);
            let selected = costed(&nodes, std::slice::from_ref(&leaf_id));
            let funding = per_node_funding(&nodes, &selected, &per_node_params(), 294);

            let named: Vec<String> = funding[0].1.iter().map(|n| n.node_id.to_string()).collect();
            assert_eq!(
                named,
                vec!["root".to_string(), "leaf".to_string(), "leaf".to_string()]
            );

            // Its fee is already paid, so it is funded without being charged for.
            let charged: u64 = selected.iter().map(|l| l.cpfp_cost).sum();
            let asked: u64 = funding[0].1.iter().map(|n| n.funding_sat).sum();
            assert!(
                asked > charged + 3 * 294,
                "the on-chain node is funded on top of what the quote charges"
            );
        }

        #[test_all]
        fn per_node_funding_matches_the_costed_transactions() {
            // The quote costs the transactions an exit bumps, and this names them. The
            // two walk the tree separately, so they are pinned to each other here.
            // With nothing on-chain the two coincide; a divergence would quote a fee
            // for transactions the caller never funds.
            let (nodes, rich_id, poor_id) = shared_root();
            let selected = costed(&nodes, &[rich_id, poor_id]);
            let funding = per_node_funding(&nodes, &selected, &per_node_params(), 294);

            let fees: u64 = funding
                .iter()
                .flat_map(|(_, entries)| entries.iter())
                .map(|n| n.funding_sat - 294)
                .sum();
            let costed: u64 = selected.iter().map(|l| l.cpfp_cost).sum();
            assert_eq!(fees, costed);
        }

        #[test_all]
        fn plan_per_node_funds_each_transaction_from_its_own_utxo() {
            // One UTXO per transaction is matched straight through, in the order the
            // quote named them, so no CPFP child has to wait on another child's change.
            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let inputs: Vec<CpfpInput> = (0..3u32).map(|vout| cpfp_input(469, vout)).collect();
            let plan = plan_unilateral_exit(
                nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::ProfitableOnly,
                inputs.clone(),
                CpfpFundingShape::PerNode,
                250,
                22,
            )
            .unwrap();

            assert!(plan.fan_out_psbt.is_none(), "an exact set needs no fan-out");
            assert_eq!(plan.funding_shape, CpfpFundingShape::PerNode);
            assert_eq!(plan.per_branch_funding.len(), 1);
            let branch_inputs = &plan.per_branch_funding[0].1;
            assert_eq!(
                branch_inputs
                    .iter()
                    .map(|i| i.outpoint.vout)
                    .collect::<Vec<_>>(),
                vec![0, 1, 2],
                "supplied order is preserved"
            );
            assert_eq!(plan.per_node_funding[0].1.len(), branch_inputs.len());
        }

        #[test_all]
        fn plan_per_node_funding_boundary_is_exact() {
            // Funding each transaction at exactly what the quote named plans AND
            // builds: an amount in the gap would otherwise pass the plan and then
            // fail in build_cpfp_child. A sat under on any one of them is no longer a
            // one-to-one set, so it falls back to a fan-out and reports what that
            // whole fan-out needs.
            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let dust = test_script().minimal_non_dust().to_sat();
            let named = named_amounts(&nodes, &leaf_id, dust);
            assert_eq!(named.len(), 3);

            let plan_with = |amounts: &[u64]| {
                let inputs: Vec<CpfpInput> = amounts
                    .iter()
                    .enumerate()
                    .map(|(vout, sat)| cpfp_input(*sat, vout as u32))
                    .collect();
                plan_unilateral_exit(
                    nodes.clone(),
                    std::slice::from_ref(&leaf_id),
                    UnilateralExitLeafFilter::ProfitableOnly,
                    inputs,
                    CpfpFundingShape::PerNode,
                    250,
                    22,
                )
            };

            let plan = plan_with(&named).expect("the named amounts plan");
            assert!(plan.fan_out_psbt.is_none());
            for (_, funding) in &plan.per_branch_funding {
                for input in funding {
                    // Every transaction here weighs the same, so a named amount buys
                    // its package fee and leaves change at exactly the dust limit.
                    let child = build_cpfp_child(&anchor_tx_n(1), std::slice::from_ref(input), 250)
                        .expect("funding at the named amount builds a child");
                    assert_eq!(child.change_input.witness_utxo.value.to_sat(), dust);
                }
            }

            let mut short = named.clone();
            short[0] -= 1;
            let fan_out = fan_out_fee(Weight::from_wu(3 * 272), 22, 3, 250);
            match plan_with(&short) {
                Err(ServiceError::InsufficientCpfpBudget { required_sat }) => {
                    assert_eq!(required_sat, named.iter().sum::<u64>() + fan_out);
                }
                other => panic!("expected InsufficientCpfpBudget, got {other:?}"),
            }
        }

        #[test_all]
        fn per_node_funding_lands_p2tr_change_on_the_dust_limit() {
            // The amounts are only as good as the kind they were sized for. P2TR is
            // the kind the SDK quotes by default, so funding one of its transactions
            // at exactly the named amount has to leave a change output on the dust
            // limit, not a sat under it.
            let secp = Secp256k1::new();
            let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
            let pk = PublicKey::from_secret_key(&secp, &sk);
            let script = Address::p2tr(
                &secp,
                pk.x_only_public_key().0,
                None,
                bitcoin::Network::Testnet,
            )
            .script_pubkey();
            let weight = p2tr_key_path_input_weight().to_wu();
            let dust = script.minimal_non_dust().to_sat();
            assert_eq!((weight, script.len(), dust), (230, 34, 330));

            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let params = UnilateralExitLeafCostParams {
                initial_cpfp_input_weight: Weight::from_wu(weight),
                single_cpfp_input_weight: Weight::from_wu(weight),
                change_script_len: script.len(),
                destination_script_len: script.len(),
                fee_rate_sat_per_kw: 250,
                funding_shape: CpfpFundingShape::PerNode,
            };
            let selected = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                std::slice::from_ref(&leaf_id),
                &params,
                UnilateralExitLeafFilter::All,
            )
            .unwrap();

            for (_, funding) in per_node_funding(&nodes, &selected, &params, dust) {
                for named in funding {
                    let input = CpfpInput {
                        outpoint: OutPoint {
                            txid: named.txid,
                            vout: 0,
                        },
                        witness_utxo: TxOut {
                            value: Amount::from_sat(named.funding_sat),
                            script_pubkey: script.clone(),
                        },
                        signed_input_weight: weight,
                    };
                    // Every transaction in this tree weighs the same, so any of them
                    // stands in as the parent.
                    let child =
                        build_cpfp_child(&anchor_tx_n(1), std::slice::from_ref(&input), 250)
                            .expect("the named amount funds its child");
                    assert_eq!(child.change_input.witness_utxo.value.to_sat(), dust);
                }
            }
        }

        #[test_all]
        fn plan_per_node_keeps_a_pinned_input_on_its_transaction_at_every_rate() {
            // A caller pre-signing one child per fee rate builds the exit once per
            // rate from the same UTXOs, and every child has to spend the same UTXO
            // each time, or the alternatives for one transaction conflict with
            // nothing and replace nothing. Inputs supplied one per named amount, in
            // the named order, are taken in that order at any rate: matched by size
            // instead, a low rate would hand the heaviest transaction the smallest
            // input that still covers it.
            let (mut nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let root_id = nodes[&leaf_id].parent_node_id.clone().unwrap();
            let root = nodes.get_mut(&root_id).unwrap();
            for _ in 0..3 {
                root.node_tx.output.push(TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: test_script(),
                });
            }
            // Sized for the top rate, root first as the quote names it.
            let inputs = vec![
                cpfp_input(100_000, 0),
                cpfp_input(50_000, 1),
                cpfp_input(50_000, 2),
            ];
            for fee_rate in [250, 7_000, 38_250] {
                let plan = plan_unilateral_exit(
                    nodes.clone(),
                    std::slice::from_ref(&leaf_id),
                    UnilateralExitLeafFilter::ProfitableOnly,
                    inputs.clone(),
                    CpfpFundingShape::PerNode,
                    fee_rate,
                    22,
                )
                .unwrap();
                assert!(plan.fan_out_psbt.is_none());
                let named: Vec<&TreeNodeId> = plan.per_node_funding[0]
                    .1
                    .iter()
                    .map(|n| &n.node_id)
                    .collect();
                assert_eq!(named[0], &root_id, "the root is named first");
                assert_eq!(
                    plan.per_branch_funding[0]
                        .1
                        .iter()
                        .map(|i| i.outpoint.vout)
                        .collect::<Vec<_>>(),
                    vec![0, 1, 2],
                    "at {fee_rate} sat/kw each transaction keeps the input supplied for it"
                );
            }
        }

        #[test_all]
        fn plan_per_node_takes_one_address_per_utxo() {
            // The amounts read a funding UTXO's weight, script length and dust limit,
            // none of which a fresh address changes. Funding each transaction from its
            // own address is ordinary wallet hygiene and has to match straight
            // through, not fall into a fan-out the quoted amounts cannot pay for.
            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let dust = test_script().minimal_non_dust().to_sat();
            let named = named_amounts(&nodes, &leaf_id, dust);

            let inputs: Vec<CpfpInput> = named
                .iter()
                .enumerate()
                .map(|(i, sat)| {
                    let mut input = cpfp_input(*sat, i as u32);
                    // A distinct P2WPKH address per UTXO: same kind, different script.
                    let secp = Secp256k1::new();
                    let sk = SecretKey::from_slice(&[i as u8 + 0x40; 32]).unwrap();
                    let pk = PublicKey::from_secret_key(&secp, &sk);
                    input.witness_utxo.script_pubkey =
                        Address::p2wpkh(&CompressedPublicKey(pk), bitcoin::Network::Testnet)
                            .script_pubkey();
                    input
                })
                .collect();
            let scripts: HashSet<&ScriptBuf> = inputs
                .iter()
                .map(|i| &i.witness_utxo.script_pubkey)
                .collect();
            assert_eq!(scripts.len(), 3, "three different addresses");

            let plan = plan_unilateral_exit(
                nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::ProfitableOnly,
                inputs,
                CpfpFundingShape::PerNode,
                250,
                22,
            )
            .unwrap();
            assert!(
                plan.fan_out_psbt.is_none(),
                "one address per UTXO still matches one to one"
            );
        }

        #[test_all]
        fn plan_per_node_accepts_the_named_amounts_in_any_order() {
            // The inputs share one funding kind, so which UTXO pays which fee is a
            // matter of size alone: the named amounts supplied in any order have to
            // match through, not fall into a fan-out the amounts cannot pay for.
            let (mut nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            // A heavier leaf transaction and a heavier refund still, so all three
            // named amounts differ.
            let heavier = |base: Transaction, extra_outputs: usize| {
                let mut tx = base;
                for _ in 0..extra_outputs {
                    tx.output.push(TxOut {
                        value: Amount::from_sat(0),
                        script_pubkey: test_script(),
                    });
                }
                tx
            };
            let leaf = nodes.get_mut(&leaf_id).unwrap();
            leaf.node_tx = heavier(anchor_tx_n(2), 1);
            leaf.refund_tx = Some(heavier(anchor_tx_n(3), 2));

            let dust = test_script().minimal_non_dust().to_sat();
            let named = named_amounts(&nodes, &leaf_id, dust);
            assert_eq!(named.len(), 3);
            let mut reversed = named.clone();
            reversed.reverse();
            assert_ne!(
                named, reversed,
                "the amounts have to differ for the shuffle"
            );

            let inputs: Vec<CpfpInput> = reversed
                .iter()
                .enumerate()
                .map(|(vout, sat)| cpfp_input(*sat, vout as u32))
                .collect();
            let plan = plan_unilateral_exit(
                nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::ProfitableOnly,
                inputs,
                CpfpFundingShape::PerNode,
                250,
                22,
            )
            .unwrap();
            assert!(
                plan.fan_out_psbt.is_none(),
                "any order of the amounts matches"
            );
            for (funding, need) in plan.per_branch_funding[0].1.iter().zip(&named) {
                assert!(
                    funding.witness_utxo.value.to_sat() >= *need,
                    "each transaction's input covers its amount"
                );
            }
        }

        #[test_all]
        fn plan_per_node_leaves_a_surplus_input_untouched() {
            // A coin left over from an earlier resume sits next to the ones the quote
            // asked for. Fanning everything out because of it would charge a fan-out
            // fee and lock that coin into change, so the covering subset is matched
            // and the rest is left alone, as per-branch funding already does.
            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let dust = test_script().minimal_non_dust().to_sat();
            let named = named_amounts(&nodes, &leaf_id, dust);
            let mut inputs: Vec<CpfpInput> = named
                .iter()
                .enumerate()
                .map(|(vout, sat)| cpfp_input(*sat, vout as u32))
                .collect();
            inputs.push(cpfp_input(100_000, 9));

            let plan = plan_unilateral_exit(
                nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::ProfitableOnly,
                inputs,
                CpfpFundingShape::PerNode,
                250,
                22,
            )
            .unwrap();
            assert!(
                plan.fan_out_psbt.is_none(),
                "a surplus input forces no fan-out"
            );
            let assigned: Vec<u32> = plan.per_branch_funding[0]
                .1
                .iter()
                .map(|i| i.outpoint.vout)
                .collect();
            assert_eq!(assigned.len(), 3);
            assert!(
                !assigned.contains(&9),
                "the exact fits are taken and the surplus coin is the one left over"
            );
        }

        #[test_all]
        fn plan_per_node_fans_out_a_funding_set_of_mixed_kinds() {
            // The named amounts describe one kind of funding UTXO. An input of
            // another kind at the same amount buys a different package fee and a
            // different dust limit, so it is fanned out rather than matched straight
            // through to fail on whichever transaction drew it.
            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let mut inputs: Vec<CpfpInput> =
                (0..3u32).map(|vout| cpfp_input(50_000, vout)).collect();
            inputs[1].signed_input_weight = 230;

            let plan = plan_unilateral_exit(
                nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::ProfitableOnly,
                inputs,
                CpfpFundingShape::PerNode,
                250,
                22,
            )
            .unwrap();
            assert!(
                plan.fan_out_psbt.is_some(),
                "a mixed funding set is fanned out onto one script"
            );
        }

        #[test_all]
        fn plan_per_node_fans_out_one_output_per_transaction() {
            // A caller with a single UTXO can still fund per node: the fan-out splits
            // it one output per transaction rather than one per branch.
            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let plan = plan_unilateral_exit(
                nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::ProfitableOnly,
                vec![cpfp_input(50_000, 0)],
                CpfpFundingShape::PerNode,
                250,
                22,
            )
            .unwrap();

            let psbt = plan.fan_out_psbt.expect("one UTXO for three transactions");
            assert_eq!(psbt.unsigned_tx.output.len(), 3);
            assert_eq!(plan.per_branch_funding[0].1.len(), 3);
            assert!(
                plan.per_branch_funding[0]
                    .1
                    .iter()
                    .enumerate()
                    .all(|(vout, input)| input.outpoint.vout == vout as u32
                        && input.outpoint.txid == psbt.unsigned_tx.compute_txid()),
                "each transaction is funded by its own fan-out output"
            );
        }

        #[test_all]
        fn quote_per_node_names_one_list_and_sizes_the_single_utxo_from_it() {
            // A per-node quote funds through its per-transaction list alone, and the
            // single UTXO that fans out into them is sized from that list.
            let (nodes, leaf_id) = root_and_leaf(TreeNodeStatus::Available);
            let quote_of = |shape| {
                quote_unilateral_exit(
                    &nodes,
                    std::slice::from_ref(&leaf_id),
                    UnilateralExitLeafFilter::ProfitableOnly,
                    272,
                    22,
                    294,
                    shape,
                    250,
                    22,
                )
                .unwrap()
            };

            let per_node = quote_of(CpfpFundingShape::PerNode);
            assert_eq!(per_node.per_node_funding.len(), 3);
            assert!(
                per_node.per_branch_funding.is_empty(),
                "each shape names one list to fund"
            );
            let named: u64 = per_node
                .per_node_funding
                .iter()
                .map(|n| n.funding_sat)
                .sum();
            // A single UTXO has to cover the branch plus the fan-out that splits it.
            assert_eq!(
                per_node.single_utxo_funding_sat,
                named + per_node.fanout_fee_sat
            );
            // What a per-node branch asks for is exactly what its children spend: the
            // package fees plus one non-dust change each. It carries no sweep share,
            // because it leaves that change on the funding script rather than sweeping
            // it, and the sweep is paid out of the refunds it does pull.
            let cpfp: u64 = per_node.selected_leaves.iter().map(|l| l.cpfp_cost).sum();
            assert_eq!(named, cpfp + 3 * 294);

            // Per-branch funding chains the change down to one terminal output, so it
            // reserves a single dust limit and the sweep-input headroom that output
            // costs to spend.
            let per_branch = quote_of(CpfpFundingShape::PerBranch);
            assert!(per_branch.per_node_funding.is_empty());
            assert_eq!(
                per_branch.per_branch_funding[0].1,
                per_branch.selected_leaves[0].estimated_cost + 294
            );

            // The quoted fee follows the sweep the shape actually builds. Per node
            // the sweep pulls the refund alone, so the leaf is costed one sweep
            // input; per branch it also pulls the branch's terminal change, and
            // both the fee and what the destination receives differ by that input.
            let sweep_input =
                |extra: Weight| compute_sweep_fee(p2tr_key_path_input_weight() + extra, 22, 250);
            assert_eq!(
                per_node.selected_leaves[0].estimated_cost,
                per_node.selected_leaves[0].cpfp_cost + sweep_input(Weight::ZERO),
                "a per-node leaf is costed one refund input into the sweep"
            );
            assert_eq!(
                per_branch.selected_leaves[0].estimated_cost,
                per_branch.selected_leaves[0].cpfp_cost + sweep_input(Weight::from_wu(272)),
                "a per-branch leaf is costed its terminal change input too"
            );
            // The quoted total adds the fan-out on top of those per-leaf costs, and
            // per node one UTXO has to split into a share per transaction rather
            // than per branch, so a single-leaf exit funded that way quotes HIGHER
            // overall even though its sweep is cheaper.
            assert_eq!(per_branch.fanout_fee_sat, 0, "one branch needs no fan-out");
            assert!(per_node.fanout_fee_sat > 0, "three transactions need one");
            assert_eq!(
                per_node.total_fee_sat,
                per_node.selected_leaves[0].estimated_cost + per_node.fanout_fee_sat
            );
            assert!(
                per_node.total_fee_sat > per_branch.total_fee_sat,
                "the fan-out outweighs the sweep input per node ({} vs {})",
                per_node.total_fee_sat,
                per_branch.total_fee_sat
            );
        }
    }
}
