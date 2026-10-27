use std::collections::{HashMap, HashSet};

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
    utils::{
        fee_ladder::{refund_rung_tx, signed_key_path_vsize},
        transactions::{has_anchor, is_ephemeral_anchor_output},
    },
};

/// Returns a leaf's ancestor chain, root → leaf. The walk is purely structural:
/// it crosses a node whatever its status, because what the exit broadcasts is the
/// pre-signed transaction lineage, not the operator's label for it. An ancestor
/// that is `TransferLocked`, `RenewLocked` or under `Investigation` still holds an
/// unspent output whose `node_tx` every node below it spends. `Err(parent_id)`
/// names the first ancestor missing from `node_map` for the caller to re-fetch.
pub fn walk_unilateral_exit_chain<'a>(
    node_map: &'a HashMap<TreeNodeId, TreeNode>,
    leaf: &'a TreeNode,
) -> Result<Vec<&'a TreeNode>, TreeNodeId> {
    let mut chain = Vec::new();
    let mut visited: HashSet<TreeNodeId> = HashSet::new();
    let mut current = leaf;
    loop {
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

/// How funding UTXOs pay the CPFP children of a unilateral exit: which of a
/// leaf's CPFP parents share one funding UTXO.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CpfpFundingStrategy {
    /// One UTXO per branch. Each child after the first has the change of the one
    /// before it as its funding input, and the sweep spends the change of the
    /// last.
    #[default]
    PerBranch,
    /// One UTXO per fee-bumped transaction. No child has another's output as an
    /// input, so each can be signed at any fee rate without knowing the others.
    /// The sweep spends none of their change, so it stays valid whichever of
    /// them the caller broadcasts.
    PerNode,
}

/// A transaction of the exit that needs a CPFP child: a node's `node_tx` or a
/// leaf's `refund_tx`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpfpParent {
    pub txid: Txid,
    pub weight: Weight,
}

/// CPFP parents funded from one UTXO, ordered root to leaf. The inputs of the
/// first parent's CPFP child are the chain's funding, and the input of each
/// later CPFP child is the change output of the CPFP child before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpfpChain {
    pub parents: Vec<CpfpParent>,
    /// Whether the sweep spends the change of the chain's last CPFP child.
    pub sweep_change: bool,
}

impl CpfpChain {
    /// Each parent with the weight of its CPFP child's funding inputs: `first`
    /// for the first parent, `change` for each later one.
    fn input_weights(
        &self,
        first: Weight,
        change: Weight,
    ) -> impl Iterator<Item = (&CpfpParent, Weight)> + '_ {
        self.parents
            .iter()
            .enumerate()
            .map(move |(i, parent)| (parent, if i == 0 { first } else { change }))
    }

    /// The fees of the chain's CPFP children, for funding of the kind `params`
    /// describes.
    fn cpfp_fee(&self, params: &UnilateralExitLeafCostParams) -> u64 {
        self.input_weights(
            params.initial_cpfp_input_weight,
            params.single_cpfp_input_weight,
        )
        .map(|(parent, input_weight)| {
            compute_cpfp_package_fee(
                parent.weight,
                input_weight,
                params.change_script_len,
                params.fee_rate_sat_per_kw,
            )
        })
        .fold(0u64, u64::saturating_add)
    }

    /// The least value of the one UTXO from which [`build_cpfp_child`] builds every
    /// CPFP child of the chain. The first child has inputs of `first_input_weight`,
    /// and each later one the change of the child before it, of
    /// `change_input_weight`.
    fn floor_sat(
        &self,
        first_input_weight: Weight,
        change_input_weight: Weight,
        change_script_len: usize,
        change_dust_limit: u64,
        fee_rate_sat_per_kw: u64,
    ) -> u64 {
        let children: Vec<CpfpChildFloor> = self
            .input_weights(first_input_weight, change_input_weight)
            .map(|(parent, input_weight)| {
                cpfp_child_floor(
                    parent.weight,
                    input_weight,
                    change_script_len,
                    change_dust_limit,
                    fee_rate_sat_per_kw,
                )
            })
            .collect();
        // Every child pays its fee, and the last one also leaves a change at the
        // dust limit.
        children.split_last().map_or(0, |(last, earlier)| {
            earlier
                .iter()
                .map(|child| child.fee_sat)
                .fold(last.required_sat, u64::saturating_add)
        })
    }

    /// The weight whose fee the chain's CPFP children pay. The first child has
    /// inputs of `first_input_weight`, and each later one the change of the
    /// child before it, of `change_input_weight`.
    fn package_weight(
        &self,
        first_input_weight: Weight,
        change_input_weight: Weight,
        change_script_len: usize,
    ) -> Weight {
        self.input_weights(first_input_weight, change_input_weight)
            .fold(Weight::ZERO, |total, (parent, input_weight)| {
                total + cpfp_package_weight(parent.weight, input_weight, change_script_len)
            })
    }
}

/// A chain and the UTXOs that are the inputs of its first CPFP child.
#[derive(Clone, Debug)]
pub struct CpfpFunding {
    pub chain: CpfpChain,
    pub inputs: Vec<CpfpInput>,
}

impl CpfpFunding {
    fn value_sat(&self) -> u64 {
        self.inputs
            .iter()
            .map(|input| input.witness_utxo.value.to_sat())
            .fold(0u64, u64::saturating_add)
    }

    /// The least total value of the inputs from which [`build_cpfp_child`] builds
    /// every CPFP child of the chain. The first child has all of the inputs, and
    /// every change output has the script and signed weight of the first input.
    /// `None` without inputs.
    fn required_sat(&self, fee_rate_sat_per_kw: u64) -> Option<u64> {
        let first = self.inputs.first()?;
        let all_inputs = self
            .inputs
            .iter()
            .map(|input| input.signed_input_weight)
            .fold(0u64, u64::saturating_add);
        let script = &first.witness_utxo.script_pubkey;
        Some(self.chain.floor_sat(
            Weight::from_wu(all_inputs),
            Weight::from_wu(first.signed_input_weight),
            script.len(),
            script.minimal_non_dust().to_sat(),
            fee_rate_sat_per_kw,
        ))
    }
}

#[derive(Clone, Debug)]
pub struct UnilateralExitPlan {
    pub selected_leaves: Vec<UnilateralExitSelectedLeaf>,
    /// Set when the inputs can't be split over the chains; one output per chain.
    pub fan_out_psbt: Option<psbt::Psbt>,
    /// Every chain of the exit, in the order of the selected leaves, with the
    /// inputs of its first CPFP child.
    pub funding: Vec<CpfpFunding>,
    /// The exit tree, keyed by node id. Every selected leaf's full ancestor chain
    /// is present, so the build resolves offline without re-fetching.
    pub tree_nodes: HashMap<TreeNodeId, TreeNode>,
}

/// What of an exit's tree is on-chain. Resolved before the exit is funded, and
/// everything an exit does follows from it: a node already there is not built
/// again, a refund already there is not built again and its output is what the
/// sweep pulls from, and the builder spends the funding on the CPFP children of
/// the transactions that still get one.
///
/// Ordered by id, so the same chain gives the same state.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ExitChainState {
    pub nodes: Vec<ConfirmedExitNode>,
    pub refunds: Vec<ExitRefund>,
    /// Leaves whose cpfp lineage was taken on-chain by a transaction the exit
    /// cannot continue from.
    pub stopped_leaves: Vec<TreeNodeId>,
    /// Nodes whose own lookup failed. Their state is unknown, not absent.
    pub unverified_nodes: Vec<TreeNodeId>,
    /// Nodes taken to be confirmed on the operators' word because the chain
    /// could not be read. Their spend is invisible, so anything built over them
    /// risks double-spending an output already gone.
    pub unverifiable_confirmed_nodes: Vec<TreeNodeId>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConfirmedExitNode {
    pub node_id: TreeNodeId,
    pub confirmed_by: ExitNodeConfirmation,
    /// The block it is in, where that is known. A relative timelock counts from
    /// this height, so a caller holding it can tell when a child of this node
    /// can go out. Unknown for a node put in a block by a descendant's
    /// confirmation rather than read directly.
    pub block_height: Option<u32>,
}

/// Which of a node's two pre-signed spends took it on-chain. The cpfp one is
/// fee-bumped by a child; the direct one pays its own fee, and a leaf that went
/// out that way is refunded by its `direct_refund_tx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExitNodeConfirmation {
    Cpfp,
    Direct,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ExitRefund {
    pub leaf_id: TreeNodeId,
    pub state: ExitRefundState,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum ExitRefundState {
    /// On-chain with its output still there to sweep. A sweep sitting unconfirmed
    /// in the mempool leaves the refund here, so that sweep is rebuilt rather
    /// than dropped.
    OnChain {
        tx: Transaction,
        vout: u32,
        value: u64,
        block_height: Option<u32>,
    },
    /// Spent by a confirmed transaction: the sweep landed.
    Swept,
}

impl ExitChainState {
    /// Whether this node's transaction is on-chain.
    #[must_use]
    pub fn has_node(&self, node_id: &TreeNodeId) -> bool {
        self.nodes.iter().any(|node| node.node_id == *node_id)
    }

    /// Whether this leaf's refund is on-chain, swept or not: either way it is
    /// not built again.
    #[must_use]
    pub fn has_refund(&self, leaf_id: &TreeNodeId) -> bool {
        self.refunds.iter().any(|refund| refund.leaf_id == *leaf_id)
    }

    /// Whether the scan recorded this leaf's node as confirmed by its direct tx.
    fn went_direct(&self, leaf_id: &TreeNodeId) -> bool {
        self.nodes.iter().any(|node| {
            node.node_id == *leaf_id && node.confirmed_by == ExitNodeConfirmation::Direct
        })
    }

    /// Whether nothing is left of this leaf's exit: its refund was swept, or its
    /// branch was taken by a spend the exit cannot continue from and no refund
    /// surfaced.
    #[must_use]
    pub fn is_finished(&self, leaf_id: &TreeNodeId) -> bool {
        match self.refunds.iter().find(|r| r.leaf_id == *leaf_id) {
            Some(refund) => matches!(refund.state, ExitRefundState::Swept),
            None => self.stopped_leaves.contains(leaf_id),
        }
    }
}

/// Selects which leaves to exit and assigns the funding inputs to their CPFP
/// chains. Never fetches: works offline as long as `tree_nodes` holds each
/// selected leaf's full ancestor chain.
#[allow(clippy::too_many_arguments)]
pub fn plan_unilateral_exit(
    tree_nodes: HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    filter: UnilateralExitLeafFilter,
    inputs: Vec<CpfpInput>,
    funding_strategy: CpfpFundingStrategy,
    fee_rate_sat_per_kw: u64,
    destination_script_len: usize,
    on_chain: &ExitChainState,
) -> Result<UnilateralExitPlan, ServiceError> {
    if leaf_ids.is_empty() {
        return Ok(UnilateralExitPlan {
            selected_leaves: vec![],
            fan_out_psbt: None,
            funding: vec![],
            tree_nodes,
        });
    }

    // Without inputs no CPFP child is built, so the weight is not used.
    let input_weight = Weight::from_wu(inputs.first().map_or(0, |i| i.signed_input_weight));
    let change_dust_limit = inputs.first().map_or(0, |i| {
        i.witness_utxo.script_pubkey.minimal_non_dust().to_sat()
    });
    let params = UnilateralExitLeafCostParams {
        initial_cpfp_input_weight: input_weight,
        single_cpfp_input_weight: input_weight,
        change_script_len: inputs
            .first()
            .map_or(0, |i| i.witness_utxo.script_pubkey.len()),
        change_dust_limit,
        destination_script_len,
        fee_rate_sat_per_kw,
        funding_strategy,
    };

    let selected =
        evaluate_unilateral_exit_leaf_costs(&tree_nodes, leaf_ids, &params, filter, on_chain)?;
    let entries: Vec<FundingEntry> = selected
        .iter()
        .flat_map(|leaf| leaf.funding.iter().cloned())
        .collect();
    let (funding, fan_out_psbt) =
        fund_chains(entries, inputs, fee_rate_sat_per_kw, change_dust_limit)?;

    let plan = UnilateralExitPlan {
        selected_leaves: selected,
        fan_out_psbt,
        funding,
        tree_nodes,
    };
    debug!(
        selected_leaves = plan.selected_leaves.len(),
        chains = plan.funding.len(),
        has_fan_out = plan.fan_out_psbt.is_some(),
        tree_nodes = plan.tree_nodes.len(),
        ?funding_strategy,
        "plan_unilateral_exit: planned"
    );
    Ok(plan)
}

/// A chain of a selected leaf and the least value of the one UTXO that funds it.
#[derive(Clone, Debug)]
pub struct FundingEntry {
    pub leaf_id: TreeNodeId,
    pub chain: CpfpChain,
    /// The fees of the chain's CPFP children plus a last change at the dust
    /// limit. On the leaf's last chain whose change the sweep spends, also the
    /// leaf's share of the sweep fee, which ends up in that change.
    pub amount_sat: u64,
    /// The weight whose fee `amount_sat` pays: the chain's transactions and
    /// their CPFP children, and on that chain also the leaf's share of the
    /// sweep.
    pub weight: Weight,
}

/// Assigns the inputs to the chains. A single chain gets every input. Several
/// chains each get inputs of their own when [`assign_inputs`] finds such a split,
/// and otherwise one output each of a fan-out that has every input.
fn fund_chains(
    entries: Vec<FundingEntry>,
    inputs: Vec<CpfpInput>,
    fee_rate_sat_per_kw: u64,
    change_dust_limit: u64,
) -> Result<(Vec<CpfpFunding>, Option<psbt::Psbt>), ServiceError> {
    if entries.is_empty() {
        return Ok((Vec::new(), None));
    }
    if inputs.is_empty() {
        return Err(ServiceError::InvalidInput(
            "At least one funding input is required".to_string(),
        ));
    }
    if let [entry] = entries.as_slice() {
        // The sweep pays its fee from the value it sweeps, so the floor is what
        // build_cpfp_child needs: the chain's CPFP fees on these inputs, and dust.
        let funding = CpfpFunding {
            chain: entry.chain.clone(),
            inputs,
        };
        let required = funding
            .required_sat(fee_rate_sat_per_kw)
            .unwrap_or_default();
        if funding.value_sat() < required {
            return Err(ServiceError::InsufficientCpfpBudget {
                required_sat: required,
            });
        }
        return Ok((vec![funding], None));
    }
    if let Some(funding) = assign_inputs(&inputs, &entries, fee_rate_sat_per_kw) {
        return Ok((funding, None));
    }

    let amounts: Vec<u64> = entries.iter().map(|entry| entry.amount_sat).collect();
    let surplus_weights: Vec<u64> = amounts
        .iter()
        .map(|amount| amount.saturating_sub(change_dust_limit))
        .collect();
    let (psbt, outputs) = build_fan_out(&inputs, &amounts, &surplus_weights, fee_rate_sat_per_kw)?;
    let funding = entries
        .into_iter()
        .zip(outputs)
        .map(|(entry, output)| CpfpFunding {
            chain: entry.chain,
            inputs: vec![output],
        })
        .collect();
    Ok((funding, Some(psbt)))
}

/// Splits the inputs over the entries' chains so that each chain has inputs of
/// its own and the exit needs no fan-out. Greedy: gives the entry with the
/// largest amount the largest inputs until they cover it, keeping one input back
/// for each entry still without any. Returned in the order of `entries`. `None`
/// when the split leaves a chain short of what [`build_cpfp_child`] needs.
fn assign_inputs(
    inputs: &[CpfpInput],
    entries: &[FundingEntry],
    fee_rate_sat_per_kw: u64,
) -> Option<Vec<CpfpFunding>> {
    if inputs.len() < entries.len() {
        return None;
    }
    let mut largest_first: Vec<&CpfpInput> = inputs.iter().collect();
    largest_first.sort_by(|a, b| {
        b.witness_utxo
            .value
            .cmp(&a.witness_utxo.value)
            .then_with(|| a.outpoint.cmp(&b.outpoint))
    });
    let mut remaining = largest_first.into_iter();
    // The planner orders equal amounts by the weight whose fee they pay, before
    // the leaf id. At a zero fee rate every amount is the dust limit, and the
    // planner then pairs the inputs as it does at other rates, apart from
    // rounding.
    let mut largest_amount_first: Vec<(usize, &FundingEntry)> =
        entries.iter().enumerate().collect();
    largest_amount_first.sort_by(|(_, a), (_, b)| {
        b.amount_sat
            .cmp(&a.amount_sat)
            .then_with(|| b.weight.cmp(&a.weight))
            .then_with(|| a.leaf_id.cmp(&b.leaf_id))
            .then_with(|| {
                let first_txid = |chain: &CpfpChain| chain.parents.first().map(|p| p.txid);
                first_txid(&a.chain).cmp(&first_txid(&b.chain))
            })
    });

    let mut assigned: Vec<(usize, CpfpFunding)> = Vec::with_capacity(entries.len());
    for (position, (index, entry)) in largest_amount_first.into_iter().enumerate() {
        let entries_left_after = entries.len().saturating_sub(position + 1);
        let mut chain_inputs: Vec<CpfpInput> = Vec::new();
        let mut sum: u64 = 0;
        while sum < entry.amount_sat {
            if remaining.len() <= entries_left_after {
                return None;
            }
            let input = remaining.next()?;
            sum = sum.saturating_add(input.witness_utxo.value.to_sat());
            chain_inputs.push(input.clone());
        }
        assigned.push((
            index,
            CpfpFunding {
                chain: entry.chain.clone(),
                inputs: chain_inputs,
            },
        ));
    }
    assigned.sort_by_key(|(index, _)| *index);
    let funding: Vec<CpfpFunding> = assigned.into_iter().map(|(_, funding)| funding).collect();
    funding
        .iter()
        .all(|f| {
            f.required_sat(fee_rate_sat_per_kw)
                .is_some_and(|required| f.value_sat() >= required)
        })
        .then_some(funding)
}

/// A chain-independent unilateral-exit quote: which leaves would exit and the
/// funding they need, sized from the funding kind's weight with no actual UTXOs.
pub struct UnilateralExitQuote {
    pub selected_leaves: Vec<UnilateralExitSelectedLeaf>,
    /// The requested leaves the quote leaves out.
    pub skipped_leaves: Vec<UnilateralExitSkippedLeaf>,
    /// Funding to avoid a fan-out: (leaf id, minimum sats), one entry per
    /// branch, or under per-node funding one per fee-bumped transaction. Only
    /// the transactions that still get a CPFP child are funded.
    pub per_branch_funding: Vec<(TreeNodeId, u64)>,
    pub single_utxo_funding_sat: u64,
    /// CPFP children's fees, paid by the funding UTXOs.
    pub cpfp_fee_sat: u64,
    /// The fan-out's fee, paid by the funding UTXO. Zero for a single branch.
    pub fanout_fee_sat: u64,
    /// The fees paid out of the leaves' value before the sweep: the sum of each
    /// selected leaf's `refund_fee`.
    pub refund_fee_sat: u64,
    /// The sweep's fee, paid out of the value being swept rather than by the
    /// funding.
    pub sweep_fee_sat: u64,
    /// `cpfp_fee_sat + fanout_fee_sat + refund_fee_sat + sweep_fee_sat`.
    pub total_fee_sat: u64,
}

pub struct UnilateralExitSkippedLeaf {
    pub id: TreeNodeId,
    pub value: u64,
    pub reason: UnilateralExitSkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnilateralExitSkipReason {
    /// Its exit costs at least the value of its refund.
    Unprofitable,
    /// It cannot be exited, for the reason given.
    Unexitable(String),
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
    funding_strategy: CpfpFundingStrategy,
    fee_rate_sat_per_kw: u64,
    destination_script_len: usize,
    on_chain: &ExitChainState,
) -> Result<UnilateralExitQuote, ServiceError> {
    let params = UnilateralExitLeafCostParams {
        initial_cpfp_input_weight: Weight::from_wu(funding_input_weight),
        single_cpfp_input_weight: Weight::from_wu(funding_input_weight),
        change_script_len: funding_output_script_len,
        change_dust_limit,
        destination_script_len,
        fee_rate_sat_per_kw,
        funding_strategy,
    };

    let selected =
        evaluate_unilateral_exit_leaf_costs(tree_nodes, leaf_ids, &params, filter, on_chain)?;
    let skipped_leaves = leaf_ids
        .iter()
        .filter(|id| !selected.iter().any(|leaf| leaf.id == **id))
        .filter_map(|id| {
            let leaf = tree_nodes.get(id)?;
            let reason = match exit_transactions(tree_nodes, leaf, on_chain) {
                Ok(_) => UnilateralExitSkipReason::Unprofitable,
                Err(reason) => UnilateralExitSkipReason::Unexitable(reason),
            };
            Some(UnilateralExitSkippedLeaf {
                id: id.clone(),
                value: leaf.value,
                reason,
            })
        })
        .collect();
    if selected.is_empty() {
        return Ok(UnilateralExitQuote {
            selected_leaves: vec![],
            skipped_leaves,
            per_branch_funding: vec![],
            single_utxo_funding_sat: 0,
            cpfp_fee_sat: 0,
            fanout_fee_sat: 0,
            refund_fee_sat: 0,
            sweep_fee_sat: 0,
            total_fee_sat: 0,
        });
    }

    let per_branch_funding: Vec<(TreeNodeId, u64)> = selected
        .iter()
        .flat_map(|leaf| &leaf.funding)
        .map(|entry| (entry.leaf_id.clone(), entry.amount_sat))
        .collect();
    let leaves_total: u64 = per_branch_funding
        .iter()
        .map(|(_, sat)| *sat)
        .fold(0u64, u64::saturating_add);
    let cpfp_fee_sat: u64 = selected
        .iter()
        .map(|l| l.cpfp_cost)
        .fold(0u64, u64::saturating_add);
    let refund_fee_sat: u64 = selected
        .iter()
        .map(|l| l.refund_fee)
        .fold(0u64, u64::saturating_add);
    // Each leaf's sweep share is the marginal cost of adding its input, so the
    // shares telescope to the fee of the one sweep that spends all of them.
    let sweep_fee_sat: u64 = selected
        .iter()
        .map(|l| l.estimated_cost.saturating_sub(l.cpfp_cost))
        .fold(0u64, u64::saturating_add);

    let fanout_fee_sat = if per_branch_funding.len() <= 1 {
        0
    } else {
        fan_out_fee(
            Weight::from_wu(funding_input_weight),
            funding_output_script_len,
            per_branch_funding.len(),
            fee_rate_sat_per_kw,
        )
    };

    Ok(UnilateralExitQuote {
        single_utxo_funding_sat: leaves_total.saturating_add(fanout_fee_sat),
        total_fee_sat: cpfp_fee_sat
            .saturating_add(fanout_fee_sat)
            .saturating_add(refund_fee_sat)
            .saturating_add(sweep_fee_sat),
        selected_leaves: selected,
        skipped_leaves,
        per_branch_funding,
        cpfp_fee_sat,
        fanout_fee_sat,
        refund_fee_sat,
        sweep_fee_sat,
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
    /// The fees paid out of the leaf's value before the sweep: its value less
    /// the value of its refund, where a direct transaction's output is its input
    /// less its fee, plus the fee of the child of a refund without an anchor.
    pub refund_fee: u64,
    /// Its CPFP parents grouped into chains, each with the value of the UTXO that
    /// funds it. The parents are its refund, when a CPFP child pays the refund's
    /// fee, and each node above it that is neither on-chain in the scan's state
    /// nor a CPFP parent of an earlier leaf. Empty when no transaction of the
    /// leaf's exit still gets a CPFP child.
    pub funding: Vec<FundingEntry>,
}

pub struct UnilateralExitLeafCostParams {
    /// Weight of the first CPFP child's inputs in a leaf's chain.
    pub initial_cpfp_input_weight: Weight,
    /// Weight of each subsequent child's single (chained-change) input.
    pub single_cpfp_input_weight: Weight,
    pub change_script_len: usize,
    /// The least value of a change output to the funding script.
    pub change_dust_limit: u64,
    pub destination_script_len: usize,
    pub fee_rate_sat_per_kw: u64,
    /// How the leaf's CPFP parents are grouped into chains.
    pub funding_strategy: CpfpFundingStrategy,
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
    fee_sat(
        fee_rate_sat_per_kw,
        cpfp_package_weight(parent_weight, cpfp_input_weight, change_script_len),
    )
}

/// The weight of a parent and its CPFP child, whose fee the child pays.
fn cpfp_package_weight(
    parent_weight: Weight,
    cpfp_input_weight: Weight,
    change_script_len: usize,
) -> Weight {
    parent_weight
        + cpfp_input_weight
        + anchor_input_weight()
        + output_weight(change_script_len)
        + tx_overhead_weight()
}

/// The fee of a CPFP child and the least value of its funding inputs.
struct CpfpChildFloor {
    fee_sat: u64,
    required_sat: u64,
}

/// The CPFP child of a parent of `parent_weight` whose funding inputs weigh
/// `input_weight` and whose change output has a script of `change_script_len`
/// bytes: its fee, and the least value of its inputs, which is that fee plus a
/// change of `change_dust_limit`.
fn cpfp_child_floor(
    parent_weight: Weight,
    input_weight: Weight,
    change_script_len: usize,
    change_dust_limit: u64,
    fee_rate_sat_per_kw: u64,
) -> CpfpChildFloor {
    let fee_sat = compute_cpfp_package_fee(
        parent_weight,
        input_weight,
        change_script_len,
        fee_rate_sat_per_kw,
    );
    CpfpChildFloor {
        fee_sat,
        required_sat: fee_sat.saturating_add(change_dust_limit),
    }
}

/// The child that pays a refund's fee when the refund has no P2A anchor: it
/// spends the refund's output back to the same key, so the fee comes out of the
/// leaf rather than out of the funding. Unsigned: only the leaf key can sign it.
#[derive(Clone, Debug)]
pub struct AnchorlessRefundChild {
    pub tx: Transaction,
    pub fee_sat: u64,
}

/// Fee for an anchorless refund plus the child that pays for it.
#[must_use]
pub fn anchorless_refund_package_fee(refund_tx: &Transaction, fee_rate_sat_per_kw: u64) -> u64 {
    let value = refund_tx.output.first().map_or(0, |out| out.value.to_sat());
    let child = refund_rung_tx(refund_tx, value);
    let child_weight = Weight::from_vb_unchecked(signed_key_path_vsize(&child));
    fee_sat(fee_rate_sat_per_kw, refund_tx.weight() + child_weight)
}

/// Builds the child that pays `refund_tx`'s fee out of its own output. Errors
/// when what is left after the fee would be dust.
pub fn build_anchorless_refund_child(
    refund_tx: &Transaction,
    fee_rate_sat_per_kw: u64,
) -> Result<AnchorlessRefundChild, ServiceError> {
    let output = refund_tx
        .output
        .first()
        .ok_or_else(|| ServiceError::ValidationError("Refund has no outputs".to_string()))?;
    let value = output.value.to_sat();
    let fee_sat = anchorless_refund_package_fee(refund_tx, fee_rate_sat_per_kw);
    let output_value = value
        .checked_sub(fee_sat)
        .filter(|v| *v >= output.script_pubkey.minimal_non_dust().to_sat())
        .ok_or_else(|| {
            ServiceError::ValidationError(format!(
                "Refund of {value} sats cannot pay its own fee of {fee_sat} sats"
            ))
        })?;
    Ok(AnchorlessRefundChild {
        tx: refund_rung_tx(refund_tx, output_value),
        fee_sat,
    })
}

/// Fee for the sweep. The caller passes the total input weight directly because
/// the sweep mixes P2TR refund inputs and external CPFP-change inputs.
pub fn compute_sweep_fee(
    total_input_weight: Weight,
    destination_script_len: usize,
    fee_rate_sat_per_kw: u64,
) -> u64 {
    fee_sat(
        fee_rate_sat_per_kw,
        sweep_weight(total_input_weight, destination_script_len),
    )
}

fn sweep_weight(total_input_weight: Weight, destination_script_len: usize) -> Weight {
    total_input_weight + output_weight(destination_script_len) + tx_overhead_weight()
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
    /// Keep every requested leaf, even when its exit cost exceeds the value of
    /// its refund.
    All,
    /// Keep only leaves whose refund's value strictly exceeds their marginal
    /// exit cost.
    ProfitableOnly,
}

fn is_unilaterally_exitable(status: TreeNodeStatus) -> bool {
    match status {
        // A direct tx took the leaf's funds on-chain, into an output none of its
        // own transactions spends.
        TreeNodeStatus::WatchtowerExited | TreeNodeStatus::WatchtowerExitRecovered => false,
        // Any other status is the operators' label for the leaf: the chain
        // settles whether its stored transactions still exit it.
        TreeNodeStatus::Creating
        | TreeNodeStatus::Available
        | TreeNodeStatus::FrozenByIssuer
        | TreeNodeStatus::TransferLocked
        | TreeNodeStatus::SplitLocked
        | TreeNodeStatus::Splitted
        | TreeNodeStatus::Aggregated
        | TreeNodeStatus::OnChain
        | TreeNodeStatus::Exited
        | TreeNodeStatus::AggregateLock
        | TreeNodeStatus::Investigation
        | TreeNodeStatus::Lost
        | TreeNodeStatus::Reimbursed
        | TreeNodeStatus::RenewLocked
        | TreeNodeStatus::ParentExited
        | TreeNodeStatus::Consolidated
        | TreeNodeStatus::Returned
        | TreeNodeStatus::Unknown => true,
    }
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

/// How the builder pays the fee of a leaf's refund.
enum RefundFee<'a> {
    /// It builds nothing for the refund: the refund is on-chain, is the direct
    /// refund, which pays its own fee, or the branch is stopped.
    None,
    /// A CPFP child spends the refund's anchor, paid by the funding.
    Cpfp(&'a Transaction),
    /// The refund has no anchor: a child spending its output pays the fee out of
    /// the leaf.
    Anchorless(&'a Transaction),
}

/// The transactions of `leaf`'s exit that the builder still builds a child for,
/// or why the leaf cannot be exited. They are the refund, when it gets a child,
/// and the nodes that the scan did not record as on-chain, which get a CPFP one.
fn exit_transactions<'a>(
    tree_nodes: &'a HashMap<TreeNodeId, TreeNode>,
    leaf: &'a TreeNode,
    on_chain: &ExitChainState,
) -> Result<(RefundFee<'a>, Vec<&'a TreeNode>), String> {
    if !is_unilaterally_exitable(leaf.status) {
        return Err(format!("a {} leaf has no unilateral exit", leaf.status));
    }
    let Some(refund_tx) = &leaf.refund_tx else {
        return Err("no refund transaction".to_string());
    };
    let ancestors = walk_unilateral_exit_chain(tree_nodes, leaf).map_err(|missing| {
        format!("incomplete ancestor chain (parent {missing} missing from the tree map)")
    })?;
    // The builder builds nothing in a stopped branch. It still lists a refund
    // that is on-chain at the leaf's address for the sweep.
    if on_chain.stopped_leaves.contains(&leaf.id) {
        return Ok((RefundFee::None, Vec::new()));
    }
    // A leaf that went out through its direct tx is refunded by its
    // `direct_refund_tx`, which pays its own fee.
    let refund_fee = if on_chain.has_refund(&leaf.id) || on_chain.went_direct(&leaf.id) {
        RefundFee::None
    } else if !has_anchor(refund_tx) {
        RefundFee::Anchorless(refund_tx)
    } else {
        RefundFee::Cpfp(refund_tx)
    };
    let nodes = ancestors
        .into_iter()
        .filter(|node| !on_chain.has_node(&node.id))
        .collect();
    Ok((refund_fee, nodes))
}

/// The value of the refund output that the builder lists for `leaf`. It can
/// differ from `leaf.value`, which the operators report: the output of a direct
/// transaction holds what its input holds less its fee.
fn refund_value(leaf: &TreeNode, on_chain: &ExitChainState) -> u64 {
    for refund in &on_chain.refunds {
        if refund.leaf_id == leaf.id
            && let ExitRefundState::OnChain { value, .. } = &refund.state
        {
            return *value;
        }
    }
    let refund_tx = if on_chain.went_direct(&leaf.id) {
        &leaf.direct_refund_tx
    } else {
        &leaf.refund_tx
    };
    refund_tx
        .as_ref()
        .and_then(|tx| tx.output.first())
        .map_or(leaf.value, |output| output.value.to_sat())
}

/// Selects the leaves to exit, highest value first, and groups each leaf's CPFP
/// parents into chains by `params.funding_strategy`. Greedy: a leaf is kept when
/// the value of its refund exceeds its marginal cost (the fees of its CPFP
/// children, plus the incremental sweep inputs). A shared ancestor is a CPFP
/// parent of only the first leaf reaching it, so order matters; `All` keeps all.
pub fn evaluate_unilateral_exit_leaf_costs(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    params: &UnilateralExitLeafCostParams,
    filter: UnilateralExitLeafFilter,
    on_chain: &ExitChainState,
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
    let mut covered_txids: HashSet<Txid> = HashSet::new();
    // The sweep of the leaves selected so far.
    let mut sweep_input_weight = Weight::ZERO;
    let mut sweep_fee: u64 = 0;

    for (leaf_id, leaf) in &leaves {
        let (refund_fee, nodes) = match exit_transactions(tree_nodes, leaf, on_chain) {
            Ok(transactions) => transactions,
            Err(reason) => {
                report_unexitable(filter, leaf_id, &reason)?;
                continue;
            }
        };

        // A node above several selected leaves is a CPFP parent of the first of
        // them only.
        let mut parents: Vec<CpfpParent> = nodes
            .iter()
            .map(|node| CpfpParent {
                txid: node.node_tx.compute_txid(),
                weight: node.node_tx.weight(),
            })
            .filter(|parent| !covered_txids.contains(&parent.txid))
            .collect();
        let cpfp_refund = match refund_fee {
            RefundFee::Cpfp(refund_tx) => Some(refund_tx),
            RefundFee::None | RefundFee::Anchorless(_) => None,
        };
        if let Some(refund_tx) = cpfp_refund {
            parents.push(CpfpParent {
                txid: refund_tx.compute_txid(),
                weight: refund_tx.weight(),
            });
        }
        let chains: Vec<CpfpChain> = match params.funding_strategy {
            CpfpFundingStrategy::PerBranch if parents.is_empty() => Vec::new(),
            CpfpFundingStrategy::PerBranch => vec![CpfpChain {
                parents,
                sweep_change: true,
            }],
            CpfpFundingStrategy::PerNode => parents
                .into_iter()
                .map(|parent| CpfpChain {
                    parents: vec![parent],
                    sweep_change: false,
                })
                .collect(),
        };
        let cpfp_cost = chains
            .iter()
            .map(|chain| chain.cpfp_fee(params))
            .fold(0u64, u64::saturating_add);

        // What the funding of a chain with `sweep_change` has left after its last
        // CPFP child is an input of the sweep too.
        let swept_changes = chains.iter().filter(|chain| chain.sweep_change).count();
        let leaf_input_weight = p2tr_key_path_input_weight()
            + Weight::from_wu(
                params
                    .single_cpfp_input_weight
                    .to_wu()
                    .saturating_mul(swept_changes as u64),
            );
        let sweep_cost = compute_sweep_fee(
            sweep_input_weight + leaf_input_weight,
            params.destination_script_len,
            params.fee_rate_sat_per_kw,
        )
        .saturating_sub(sweep_fee);
        // The weight whose fee sweep_cost is: the leaf's inputs of the sweep, and
        // for the first selected leaf the rest of the sweep too.
        let sweep_before = if selected.is_empty() {
            Weight::ZERO
        } else {
            sweep_weight(sweep_input_weight, params.destination_script_len)
        };
        let sweep_cost_weight = sweep_weight(
            sweep_input_weight + leaf_input_weight,
            params.destination_script_len,
        ) - sweep_before;

        let total_marginal_cost = cpfp_cost.saturating_add(sweep_cost);

        // The sweep takes the child's output of a refund without an anchor.
        let refund_value = refund_value(leaf, on_chain);
        let child_fee = match refund_fee {
            RefundFee::Anchorless(refund_tx) => {
                match build_anchorless_refund_child(refund_tx, params.fee_rate_sat_per_kw) {
                    Ok(child) => child.fee_sat,
                    Err(_) => {
                        report_unexitable(
                            filter,
                            leaf_id,
                            "its refund cannot pay its own fee at this fee rate",
                        )?;
                        continue;
                    }
                }
            }
            RefundFee::None | RefundFee::Cpfp(_) => 0,
        };
        let swept_value = refund_value.saturating_sub(child_fee);
        if filter == UnilateralExitLeafFilter::All || swept_value > total_marginal_cost {
            // The leaf's share of the sweep fee goes with the last change the
            // sweep spends.
            let sweep_entry = chains.iter().rposition(|chain| chain.sweep_change);
            let funding = chains
                .into_iter()
                .enumerate()
                .map(|(i, chain)| FundingEntry {
                    leaf_id: (*leaf_id).clone(),
                    amount_sat: chain
                        .floor_sat(
                            params.initial_cpfp_input_weight,
                            params.single_cpfp_input_weight,
                            params.change_script_len,
                            params.change_dust_limit,
                            params.fee_rate_sat_per_kw,
                        )
                        .saturating_add(if Some(i) == sweep_entry {
                            sweep_cost
                        } else {
                            0
                        }),
                    weight: chain.package_weight(
                        params.initial_cpfp_input_weight,
                        params.single_cpfp_input_weight,
                        params.change_script_len,
                    ) + if Some(i) == sweep_entry {
                        sweep_cost_weight
                    } else {
                        Weight::ZERO
                    },
                    chain,
                })
                .collect();
            selected.push(UnilateralExitSelectedLeaf {
                id: (*leaf_id).clone(),
                value: leaf.value,
                estimated_cost: total_marginal_cost,
                cpfp_cost,
                refund_fee: leaf.value.saturating_sub(swept_value),
                funding,
            });
            covered_txids.extend(nodes.iter().map(|node| node.node_tx.compute_txid()));
            sweep_input_weight += leaf_input_weight;
            sweep_fee = sweep_fee.saturating_add(sweep_cost);
        }
    }

    Ok(selected)
}

/// An unsigned fan-out PSBT paying `output_values`, in order, to the first
/// input's script, and those outputs as funding inputs. Surplus input value is
/// shared over the outputs in proportion to `surplus_weights`, the remainder to
/// the first. RBF-signaled so an unconfirmed fan-out can be replaced.
fn build_fan_out(
    inputs: &[CpfpInput],
    output_values: &[u64],
    surplus_weights: &[u64],
    fee_rate_sat_per_kw: u64,
) -> Result<(psbt::Psbt, Vec<CpfpInput>), ServiceError> {
    let Some(first_input) = inputs.first() else {
        return Err(ServiceError::ValidationError(
            "fan-out: at least one CPFP input is required".to_string(),
        ));
    };
    if output_values.is_empty() {
        return Err(ServiceError::ValidationError(
            "fan-out: at least one output is required".to_string(),
        ));
    }

    let script_pubkey = first_input.witness_utxo.script_pubkey.clone();
    let signed_input_weight = first_input.signed_input_weight;

    let total_input_value: u64 = inputs
        .iter()
        .map(|i| i.witness_utxo.value.to_sat())
        .fold(0u64, u64::saturating_add);
    let total_input_weight: u64 = inputs
        .iter()
        .map(|i| i.signed_input_weight)
        .fold(0u64, u64::saturating_add);

    let outputs_total: u64 = output_values
        .iter()
        .copied()
        .fold(0u64, u64::saturating_add);

    let fee_no_change = fan_out_fee(
        Weight::from_wu(total_input_weight),
        script_pubkey.len(),
        output_values.len(),
        fee_rate_sat_per_kw,
    );

    if total_input_value < outputs_total.saturating_add(fee_no_change) {
        return Err(ServiceError::InsufficientCpfpBudget {
            required_sat: outputs_total.saturating_add(fee_no_change),
        });
    }

    let surplus = total_input_value
        .saturating_sub(outputs_total)
        .saturating_sub(fee_no_change);
    let mut output_values: Vec<u64> = output_values.to_vec();
    if surplus > 0 {
        let weight_total: u128 = surplus_weights.iter().map(|&w| u128::from(w)).sum();
        let mut distributed: u64 = 0;
        for (value, &weight) in output_values.iter_mut().zip(surplus_weights) {
            // checked_div guards weight_total == 0 (all weights zero): no share is
            // distributed and the whole surplus falls to the first output below.
            let share = u128::from(surplus)
                .saturating_mul(u128::from(weight))
                .checked_div(weight_total)
                .and_then(|s| u64::try_from(s).ok())
                .unwrap_or(0);
            *value = value.saturating_add(share);
            distributed = distributed.saturating_add(share);
        }
        // Never `None`: there is at least one output.
        if let Some(first) = output_values.first_mut() {
            *first = first.saturating_add(surplus.saturating_sub(distributed));
        }
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
    for (psbt_input, cpfp_input) in psbt_unsigned.inputs.iter_mut().zip(inputs) {
        *psbt_input = psbt::Input {
            witness_utxo: Some(cpfp_input.witness_utxo.clone()),
            ..Default::default()
        };
    }

    let outputs: Vec<CpfpInput> = output_values
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
        outputs = outputs.len(),
        total_input_value,
        fee = fee_no_change,
        "build_fan_out"
    );
    Ok((psbt_unsigned, outputs))
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
    use bitcoin::psbt::{Input as PsbtInput, Psbt};

    let (vout, anchor_tx_out) = parent_tx
        .output
        .iter()
        .enumerate()
        .find(|(_, tx_out)| is_ephemeral_anchor_output(tx_out))
        .ok_or(ServiceError::ValidationError(
            "Ephemeral anchor output not found".to_string(),
        ))?;

    let Some(first_funding_input) = funding_inputs.first() else {
        return Err(ServiceError::ValidationError(
            "At least one CPFP input is required for fee bumping".to_string(),
        ));
    };

    let total_input_value: u64 = funding_inputs
        .iter()
        .map(|i| i.witness_utxo.value.to_sat())
        .fold(0u64, u64::saturating_add);
    let change_script_pubkey = first_funding_input.witness_utxo.script_pubkey.clone();
    let first_signed_input_weight = first_funding_input.signed_input_weight;

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
    let floor = cpfp_child_floor(
        parent_tx.weight(),
        Weight::from_wu(input_weight),
        change_script_pubkey.len(),
        change_script_pubkey.minimal_non_dust().to_sat(),
        fee_rate_sat_per_kw,
    );
    if total_input_value < floor.required_sat {
        // The authoritative funding check: computed from the real inputs, this is
        // where a branch the plan sized on one input but funded with several
        // surfaces.
        return Err(ServiceError::InsufficientCpfpBudget {
            required_sat: floor.required_sat,
        });
    }
    let fee_amount = floor.fee_sat;
    let adjusted_output_value = total_input_value.saturating_sub(fee_amount);
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
    // The funding inputs come first and the anchor last, as in `tx_inputs`.
    let prevouts = funding_inputs
        .iter()
        .map(|cpfp_input| &cpfp_input.witness_utxo)
        .chain(std::iter::once(anchor_tx_out));
    for (psbt_input, prevout) in psbt.inputs.iter_mut().zip(prevouts) {
        *psbt_input = PsbtInput {
            witness_utxo: Some(prevout.clone()),
            ..Default::default()
        };
    }

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

        /// The walk is structural, so an ancestor is crossed whatever its status:
        /// every node below it spends its `node_tx`, so dropping one would build a
        /// silently unbroadcastable package. `SplitLocked` is the load-bearing case
        /// (a timelock renewal leaves one permanently above the renewed leaf) and
        /// `Splitted` is the ordinary intermediate node.
        #[test_all]
        fn walks_through_any_ancestor_status() {
            for status in [
                TreeNodeStatus::SplitLocked,
                TreeNodeStatus::Splitted,
                TreeNodeStatus::TransferLocked,
                TreeNodeStatus::RenewLocked,
                TreeNodeStatus::Investigation,
                TreeNodeStatus::Lost,
                TreeNodeStatus::ParentExited,
                TreeNodeStatus::Exited,
                TreeNodeStatus::Unknown,
            ] {
                let root = node(ROOT, None, TreeNodeStatus::Available);
                let mid = node(MID, Some(ROOT), status);
                let leaf = node(LEAF, Some(MID), TreeNodeStatus::Available);
                let map = node_map(&[&root, &mid, &leaf]);

                let chain = walk_unilateral_exit_chain(&map, &leaf).unwrap();

                assert_eq!(
                    chain_ids(&chain),
                    vec![ROOT, MID, LEAF],
                    "a {status} ancestor must not truncate the chain"
                );
            }
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

        /// A chain of the leaf `id` with one 400 WU parent, to be funded with
        /// `amount`.
        fn chain_of(id: &str, amount_sat: u64) -> FundingEntry {
            let parent = CpfpParent {
                txid: Txid::from_byte_array([id.as_bytes()[0]; 32]),
                weight: Weight::from_wu(400),
            };
            FundingEntry {
                leaf_id: TreeNodeId::from_str(id).unwrap(),
                chain: CpfpChain {
                    parents: vec![parent],
                    sweep_change: true,
                },
                amount_sat,
                weight: Weight::from_wu(400),
            }
        }

        fn values(funding: &CpfpFunding) -> Vec<u64> {
            funding
                .inputs
                .iter()
                .map(|i| i.witness_utxo.value.to_sat())
                .collect()
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
            let chains = vec![chain_of("a", 3_330), chain_of("b", 1_330)];
            let got = assign_inputs(&inputs, &chains, 250).expect("should fit");
            assert_eq!(got.len(), 2);
            assert_eq!(got[0].chain, chains[0].chain);
            assert_eq!(values(&got[0]), vec![10_000]);
            assert_eq!(got[1].chain, chains[1].chain);
            assert_eq!(values(&got[1]), vec![5_000]);
        }

        #[test_all]
        fn assign_inputs_returns_chain_order_when_the_first_needs_less() {
            // Chain "a" comes first but needs less; "b" needs more. The greedy pass
            // runs largest amount first (b, a).
            let inputs = vec![cpfp_input(10_000, 0), cpfp_input(5_000, 1)];
            let chains = vec![chain_of("a", 1_330), chain_of("b", 3_330)];
            let got = assign_inputs(&inputs, &chains, 250).expect("should fit");
            // Returned in chain order, which is the order the build reaches them in.
            assert_eq!(got[0].chain, chains[0].chain);
            assert_eq!(got[1].chain, chains[1].chain);
            // The costlier chain still greedily took the larger input.
            assert_eq!(values(&got[1]), vec![10_000]);
            assert_eq!(values(&got[0]), vec![5_000]);
        }

        #[test_all]
        fn assign_inputs_combines_multiple_inputs_per_chain() {
            let inputs = vec![
                cpfp_input(10_000, 0),
                cpfp_input(1_000, 1),
                cpfp_input(1_000, 2),
            ];
            let chains = vec![chain_of("a", 3_330), chain_of("b", 1_830)];
            let got = assign_inputs(&inputs, &chains, 250).expect("should fit");
            assert_eq!(got.len(), 2);
            // b (1_830) needs both small inputs; a takes the 10_000.
            assert_eq!(values(&got[0]), vec![10_000]);
            assert_eq!(values(&got[1]), vec![1_000, 1_000]);
        }

        #[test_all]
        fn assign_inputs_rejects_fewer_inputs_than_chains() {
            let chains = vec![chain_of("a", 3_330), chain_of("b", 1_330)];
            assert!(assign_inputs(&[cpfp_input(10_000, 0)], &chains, 250).is_none());
        }

        #[test_all]
        fn assign_inputs_funding_boundary_is_exact() {
            let chains = vec![chain_of("a", 3_330), chain_of("b", 1_330)];
            let exact = vec![cpfp_input(3_330, 0), cpfp_input(1_330, 1)];
            assert!(assign_inputs(&exact, &chains, 250).is_some());
            let short = vec![cpfp_input(3_330, 0), cpfp_input(1_329, 1)];
            assert!(assign_inputs(&short, &chains, 250).is_none());
        }

        #[test_all]
        fn funding_floor_counts_the_weight_of_every_input() {
            // The build gives the first CPFP child of a chain every input of the
            // chain, so its fee is based on their combined weight, and every input
            // counts at its own weight.
            let leaf = leaf_node_n("a", 1_000_000, 1);
            let leaf_id = leaf.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(leaf_id.clone(), leaf)].into_iter().collect();

            let probe = cpfp_input(0, 0);
            let change_len = probe.witness_utxo.script_pubkey.len();
            let dust = probe.witness_utxo.script_pubkey.minimal_non_dust().to_sat();
            let input_weight = probe.signed_input_weight;
            let cpfp_cost = |first_input_weight: u64, change_weight: u64| {
                evaluate_unilateral_exit_leaf_costs(
                    &nodes,
                    std::slice::from_ref(&leaf_id),
                    &UnilateralExitLeafCostParams {
                        initial_cpfp_input_weight: Weight::from_wu(first_input_weight),
                        single_cpfp_input_weight: Weight::from_wu(change_weight),
                        change_script_len: change_len,
                        change_dust_limit: 0,
                        destination_script_len: change_len,
                        fee_rate_sat_per_kw: 250,
                        funding_strategy: CpfpFundingStrategy::PerBranch,
                    },
                    UnilateralExitLeafFilter::All,
                    &ExitChainState::default(),
                )
                .unwrap()[0]
                    .cpfp_cost
            };
            let chain = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                std::slice::from_ref(&leaf_id),
                &UnilateralExitLeafCostParams {
                    initial_cpfp_input_weight: Weight::from_wu(input_weight),
                    single_cpfp_input_weight: Weight::from_wu(input_weight),
                    change_script_len: change_len,
                    change_dust_limit: 0,
                    destination_script_len: change_len,
                    fee_rate_sat_per_kw: 250,
                    funding_strategy: CpfpFundingStrategy::PerBranch,
                },
                UnilateralExitLeafFilter::All,
                &ExitChainState::default(),
            )
            .unwrap()[0]
                .funding[0]
                .chain
                .clone();
            let funding = |inputs: Vec<CpfpInput>| CpfpFunding {
                chain: chain.clone(),
                inputs,
            };

            let four = funding((0..4).map(|vout| cpfp_input(0, vout)).collect());
            assert_eq!(
                four.required_sat(250),
                Some(cpfp_cost(4 * input_weight, input_weight) + dust)
            );
            assert_eq!(
                funding(vec![cpfp_input(0, 0)]).required_sat(250),
                Some(cpfp_cost(input_weight, input_weight) + dust)
            );
            // An input heavier than the reference kind (a Custom funding kind), and
            // the change outputs that have its script.
            let mut heavy = cpfp_input(0, 0);
            heavy.signed_input_weight = 4 * input_weight;
            assert_eq!(
                funding(vec![heavy]).required_sat(250),
                Some(cpfp_cost(4 * input_weight, 4 * input_weight) + dust)
            );
            assert_eq!(funding(Vec::new()).required_sat(250), None);
        }

        #[test_all]
        fn fan_out_emits_one_output_per_chain_and_is_deterministic() {
            let inputs = vec![cpfp_input(100_000, 0)];
            let (amounts, surplus_weights) = ([3_330, 2_330], [3_000, 2_000]);
            let (psbt, outputs) = build_fan_out(&inputs, &amounts, &surplus_weights, 250).unwrap();
            assert_eq!(psbt.unsigned_tx.output.len(), 2);
            assert_eq!(outputs.len(), 2);
            // 100_000 - 141 fee - 5_660 base is split by cost (3:2), remainder to a.
            assert_eq!(outputs[0].witness_utxo.value.to_sat(), 59_850);
            assert_eq!(outputs[1].witness_utxo.value.to_sat(), 40_009);
            let out_total: u64 = psbt
                .unsigned_tx
                .output
                .iter()
                .map(|o| o.value.to_sat())
                .sum();
            assert_eq!(out_total, 100_000 - 141);
            assert_eq!(outputs[0].outpoint.vout, 0);
            assert_eq!(outputs[1].outpoint.vout, 1);
            assert_eq!(
                psbt.unsigned_tx.input[0].sequence,
                Sequence::ENABLE_RBF_NO_LOCKTIME
            );
            let (psbt2, _) = build_fan_out(&inputs, &amounts, &surplus_weights, 250).unwrap();
            assert_eq!(
                psbt.unsigned_tx.compute_txid(),
                psbt2.unsigned_tx.compute_txid()
            );
        }

        #[test_all]
        fn fan_out_funding_boundary_is_exact() {
            // base 5_660 (two chains at cost + 330 dust) + 141 fan-out fee = 5_801.
            let (amounts, surplus_weights) = ([3_330, 2_330], [3_000, 2_000]);
            assert!(
                build_fan_out(&[cpfp_input(5_801, 0)], &amounts, &surplus_weights, 250).is_ok()
            );
            assert!(
                build_fan_out(&[cpfp_input(5_800, 0)], &amounts, &surplus_weights, 250).is_err()
            );
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
            n.refund_tx = Some(refund_paying(value, 0));
            n
        }

        /// A leaf whose node and refund txs are unique to `nonce`, so independent
        /// leaves don't collide on a shared txid (which the ancestor walk would
        /// treat as an already-covered ancestor).
        fn leaf_node_n(id: &str, value: u64, nonce: u32) -> TreeNode {
            let mut n = crate::tree::tests::create_test_tree_node(id, value);
            n.node_tx = anchor_tx_n(nonce);
            n.refund_tx = Some(refund_paying(value, nonce + 1_000));
            n
        }

        fn cost_params() -> UnilateralExitLeafCostParams {
            UnilateralExitLeafCostParams {
                initial_cpfp_input_weight: Weight::from_wu(272),
                single_cpfp_input_weight: Weight::from_wu(272),
                change_script_len: 22,
                change_dust_limit: 0,
                destination_script_len: 22,
                fee_rate_sat_per_kw: 250,
                funding_strategy: CpfpFundingStrategy::PerBranch,
            }
        }

        /// An on-chain state naming the given nodes and refunds, for the cases
        /// where only which of them are there matters.
        fn on_chain_state(nodes: &[TreeNodeId], refunds: &[TreeNodeId]) -> ExitChainState {
            ExitChainState {
                nodes: nodes
                    .iter()
                    .map(|node_id| ConfirmedExitNode {
                        node_id: node_id.clone(),
                        confirmed_by: ExitNodeConfirmation::Cpfp,
                        block_height: None,
                    })
                    .collect(),
                refunds: refunds
                    .iter()
                    .map(|leaf_id| ExitRefund {
                        leaf_id: leaf_id.clone(),
                        state: ExitRefundState::Swept,
                    })
                    .collect(),
                ..Default::default()
            }
        }

        /// A root with one leaf per value below it: the nodes, the root's id and
        /// the leaves' ids.
        fn root_with_leaves(
            values: &[u64],
        ) -> (HashMap<TreeNodeId, TreeNode>, TreeNodeId, Vec<TreeNodeId>) {
            let root = leaf_node_n("root", values.iter().sum(), 1);
            let root_id = root.id.clone();
            let mut leaf_ids = Vec::new();
            let mut nodes: HashMap<TreeNodeId, TreeNode> = HashMap::new();
            for (vout, value) in (0u32..).zip(values) {
                let mut leaf = leaf_node_n(&format!("leaf-{vout}"), *value, 2 + vout);
                leaf.parent_node_id = Some(root_id.clone());
                leaf.vout = vout;
                leaf_ids.push(leaf.id.clone());
                nodes.insert(leaf.id.clone(), leaf);
            }
            nodes.insert(root_id.clone(), root);
            (nodes, root_id, leaf_ids)
        }

        /// The fee of a direct transaction, which comes out of its input.
        const DIRECT_FEE: u64 = 955;

        /// Gives `leaf` its direct tx and its direct refund.
        fn set_direct_txs(leaf: &mut TreeNode) {
            leaf.direct_tx = Some(tx_paying(leaf.value - DIRECT_FEE, 100));
            leaf.direct_refund_tx = Some(tx_paying(leaf.value - 2 * DIRECT_FEE, 101));
        }

        fn refund_on_chain(leaf_id: &TreeNodeId, value: u64) -> ExitRefund {
            ExitRefund {
                leaf_id: leaf_id.clone(),
                state: ExitRefundState::OnChain {
                    tx: tx_paying(value, 9),
                    vout: 0,
                    value,
                    block_height: None,
                },
            }
        }

        /// A settled step is not counted, rather than counted and taken off
        /// afterwards, so the first step still to be built is the one funded
        /// directly and is costed on the weight of that funding. Subtracting after
        /// leaves it costed as a chained child, which on a branch funded by several
        /// UTXOs is cheaper than the truth.
        #[test_all]
        fn a_settled_ancestor_leaves_the_next_step_funded_directly() {
            let mut root = leaf_node_n("root", 1_000_000, 1);
            root.parent_node_id = None;
            let mut leaf = leaf_node_n("leaf", 1_000_000, 2);
            leaf.parent_node_id = Some(root.id.clone());
            let leaf_id = leaf.id.clone();
            let root_id = root.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(root_id.clone(), root), (leaf_id.clone(), leaf)]
                    .into_iter()
                    .collect();

            // The funding child is much heavier than a chained one, so which step
            // is taken to be funded directly is visible in the price.
            let params = UnilateralExitLeafCostParams {
                initial_cpfp_input_weight: Weight::from_wu(4 * 272),
                single_cpfp_input_weight: Weight::from_wu(272),
                ..cost_params()
            };
            let cost_of = |on_chain: &ExitChainState| {
                evaluate_unilateral_exit_leaf_costs(
                    &nodes,
                    std::slice::from_ref(&leaf_id),
                    &params,
                    UnilateralExitLeafFilter::All,
                    on_chain,
                )
                .unwrap()[0]
                    .cpfp_cost
            };

            let fresh = cost_of(&ExitChainState::default());
            let root_on_chain = cost_of(&on_chain_state(std::slice::from_ref(&root_id), &[]));
            assert!(
                root_on_chain < fresh,
                "a settled root costs less: {root_on_chain} vs {fresh}"
            );

            // What is left is the leaf and its refund, the leaf now being the step
            // the funding pays for directly.
            let leaf_only = {
                let mut only = nodes.clone();
                only.remove(&root_id);
                let mut leaf = only.remove(&leaf_id).unwrap();
                leaf.parent_node_id = None;
                only.insert(leaf_id.clone(), leaf);
                evaluate_unilateral_exit_leaf_costs(
                    &only,
                    std::slice::from_ref(&leaf_id),
                    &params,
                    UnilateralExitLeafFilter::All,
                    &ExitChainState::default(),
                )
                .unwrap()[0]
                    .cpfp_cost
            };
            assert_eq!(
                root_on_chain, leaf_only,
                "the step below a settled one is priced as the funded step it is"
            );
        }

        /// A leaf the chain has finished with builds nothing, so it is not asked to
        /// be funded for it. It keeps its branch: the build walks the branches to
        /// collect refunds, and dropping it would leave this leaf's out of the sweep.
        #[test_all]
        fn a_finished_leaf_needs_no_funding_but_keeps_its_branch() {
            let node = leaf_node_n("leaf", 1_000_000, 1);
            let leaf_id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(leaf_id.clone(), node)].into_iter().collect();
            // Its own node tx is on-chain too: a leaf still waiting to broadcast
            // has work left whatever its refund has done.
            let on_chain = on_chain_state(
                std::slice::from_ref(&leaf_id),
                std::slice::from_ref(&leaf_id),
            );

            let selected = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                std::slice::from_ref(&leaf_id),
                &cost_params(),
                UnilateralExitLeafFilter::All,
                &on_chain,
            )
            .unwrap();
            assert_eq!(
                selected[0].cpfp_cost, 0,
                "nothing left to build costs nothing"
            );

            let plan = plan_unilateral_exit(
                nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::ProfitableOnly,
                vec![cpfp_input(1_000, 0)],
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &on_chain,
            )
            .unwrap();
            assert_eq!(
                plan.selected_leaves.len(),
                1,
                "the leaf is kept so its refund reaches the sweep"
            );
            assert!(plan.funding.is_empty(), "and it takes no funding");
        }

        #[test_all]
        fn a_leaf_confirmed_by_its_direct_tx_needs_no_funding() {
            let (mut nodes, root_id, leaf_ids) = root_with_leaves(&[1_000_000]);
            set_direct_txs(nodes.get_mut(&leaf_ids[0]).unwrap());
            let mut on_chain = on_chain_state(&[root_id, leaf_ids[0].clone()], &[]);
            on_chain.nodes[1].confirmed_by = ExitNodeConfirmation::Direct;

            let selected = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                &leaf_ids,
                &cost_params(),
                UnilateralExitLeafFilter::All,
                &on_chain,
            )
            .unwrap();

            assert_eq!(selected[0].cpfp_cost, 0);
            assert!(selected[0].funding.is_empty());
        }

        #[test_all]
        fn a_leaf_on_a_stopped_branch_needs_no_funding() {
            let (nodes, root_id, leaf_ids) = root_with_leaves(&[1_000_000]);
            let mut on_chain = on_chain_state(&[root_id], &[]);
            on_chain.refunds = vec![refund_on_chain(&leaf_ids[0], 1_000_000)];
            on_chain.stopped_leaves.clone_from(&leaf_ids);

            let selected = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                &leaf_ids,
                &cost_params(),
                UnilateralExitLeafFilter::All,
                &on_chain,
            )
            .unwrap();

            assert_eq!(selected[0].cpfp_cost, 0);
            assert!(selected[0].funding.is_empty());
        }

        #[test_all]
        fn a_leaf_is_kept_when_its_refund_holds_more_than_its_exit_costs() {
            #[derive(Clone, Copy)]
            enum Refund {
                /// In a block, its fee paid out of the leaf.
                InABlock,
                /// The direct refund, still to be broadcast.
                Direct,
                /// Still to be broadcast with a CPFP child. It holds less than the
                /// value the operators report for the leaf.
                WithChild,
            }
            // The second leaf's refund holds `refund`, which is less than the
            // leaf's value.
            let evaluate = |refund: u64, kind: Refund, filter| {
                let value = refund
                    + match kind {
                        Refund::InABlock => DIRECT_FEE,
                        Refund::Direct => 2 * DIRECT_FEE,
                        Refund::WithChild => 1_000,
                    };
                let (mut nodes, root_id, leaf_ids) = root_with_leaves(&[1_000_000, value]);
                let mut on_chain =
                    on_chain_state(&[root_id, leaf_ids[0].clone(), leaf_ids[1].clone()], &[]);
                on_chain.refunds = vec![refund_on_chain(&leaf_ids[0], 1_000_000)];
                let leaf = nodes.get_mut(&leaf_ids[1]).unwrap();
                match kind {
                    Refund::InABlock => {
                        on_chain.refunds.push(refund_on_chain(&leaf_ids[1], refund));
                    }
                    Refund::Direct => {
                        set_direct_txs(leaf);
                        on_chain.nodes[2].confirmed_by = ExitNodeConfirmation::Direct;
                    }
                    Refund::WithChild => leaf.refund_tx = Some(refund_paying(refund, 50)),
                }
                evaluate_unilateral_exit_leaf_costs(
                    &nodes,
                    &leaf_ids,
                    &cost_params(),
                    filter,
                    &on_chain,
                )
                .unwrap()
            };

            for kind in [Refund::InABlock, Refund::Direct, Refund::WithChild] {
                let cost = evaluate(1_000, kind, UnilateralExitLeafFilter::All)[1].estimated_cost;
                let at = evaluate(cost, kind, UnilateralExitLeafFilter::ProfitableOnly);
                assert_eq!(at.len(), 1, "a refund worth its exit's cost is left out");
                let above = evaluate(cost + 1, kind, UnilateralExitLeafFilter::ProfitableOnly);
                assert_eq!(above.len(), 2);
            }
        }

        #[test_all]
        fn the_sweep_fee_is_based_on_the_sweeps_inputs() {
            let (nodes, root_id, leaf_ids) = root_with_leaves(&[1_000_000, 500_000]);
            let mut on_chain = on_chain_state(&[root_id, leaf_ids[1].clone()], &[]);
            on_chain.refunds = vec![refund_on_chain(&leaf_ids[1], 500_000)];

            let quote = quote_unilateral_exit(
                &nodes,
                &leaf_ids,
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                DUST,
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &on_chain,
            )
            .unwrap();

            // Two refunds, and the change of the child built for the first.
            let inputs = p2tr_key_path_input_weight() * 2 + Weight::from_wu(272);
            assert_eq!(quote.selected_leaves.len(), 2);
            assert_eq!(quote.sweep_fee_sat, compute_sweep_fee(inputs, 22, 250));
        }

        #[test_all]
        fn a_refund_in_a_block_is_charged_for_no_change_input() {
            // The leaf holds more than the fee of a sweep of its refund alone (990)
            // and less than the fee of a sweep with a change input as well (1,670).
            let (nodes, root_id, leaf_ids) = root_with_leaves(&[1_500]);
            let mut on_chain = on_chain_state(&[root_id, leaf_ids[0].clone()], &[]);
            on_chain.refunds = vec![refund_on_chain(&leaf_ids[0], 1_500)];

            let quote = quote_unilateral_exit(
                &nodes,
                &leaf_ids,
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                DUST,
                CpfpFundingStrategy::PerBranch,
                2_500,
                22,
                &on_chain,
            )
            .unwrap();

            assert_eq!(quote.selected_leaves.len(), 1);
            assert_eq!(quote.sweep_fee_sat, 990);
        }

        #[test_all]
        fn a_quote_reports_the_fees_of_direct_transactions() {
            let quote_of = |node: TreeNode, on_chain: &ExitChainState| {
                let leaf_id = node.id.clone();
                let nodes: HashMap<TreeNodeId, TreeNode> =
                    [(leaf_id.clone(), node)].into_iter().collect();
                quote_unilateral_exit(
                    &nodes,
                    &[leaf_id],
                    UnilateralExitLeafFilter::All,
                    272,
                    22,
                    DUST,
                    CpfpFundingStrategy::PerBranch,
                    250,
                    22,
                    on_chain,
                )
                .unwrap()
            };

            let nothing_on_chain =
                quote_of(leaf_node_n("leaf", 100_000, 1), &ExitChainState::default());
            assert_eq!(nothing_on_chain.refund_fee_sat, 0);

            let node = leaf_node_n("leaf", 100_000, 1);
            let mut on_chain = on_chain_state(std::slice::from_ref(&node.id), &[]);
            on_chain.refunds = vec![ExitRefund {
                leaf_id: node.id.clone(),
                state: ExitRefundState::OnChain {
                    tx: tx_paying(99_045, 2),
                    vout: 0,
                    value: 99_045,
                    block_height: None,
                },
            }];
            let direct_refund_on_chain = quote_of(node, &on_chain);
            assert_eq!(direct_refund_on_chain.refund_fee_sat, 955);
            assert_eq!(direct_refund_on_chain.cpfp_fee_sat, 0);
            assert!(direct_refund_on_chain.per_branch_funding.is_empty());

            let mut node = leaf_node_n("leaf", 100_000, 1);
            node.direct_tx = Some(tx_paying(99_045, 2));
            node.direct_refund_tx = Some(tx_paying(98_090, 3));
            let mut on_chain = on_chain_state(std::slice::from_ref(&node.id), &[]);
            on_chain.nodes[0].confirmed_by = ExitNodeConfirmation::Direct;
            let direct_tx_on_chain = quote_of(node, &on_chain);
            assert_eq!(direct_tx_on_chain.refund_fee_sat, 1_910);
            assert_eq!(direct_tx_on_chain.cpfp_fee_sat, 0);
            assert!(direct_tx_on_chain.per_branch_funding.is_empty());
        }

        #[test_all]
        fn the_total_fee_of_a_quote_includes_the_refund_fee() {
            let ordinary = leaf_node_n("ordinary", 100_000, 1);
            let refunded = leaf_node_n("refunded", 100_000, 3);
            let on_chain = ExitChainState {
                nodes: vec![ConfirmedExitNode {
                    node_id: refunded.id.clone(),
                    confirmed_by: ExitNodeConfirmation::Cpfp,
                    block_height: None,
                }],
                refunds: vec![ExitRefund {
                    leaf_id: refunded.id.clone(),
                    state: ExitRefundState::OnChain {
                        tx: tx_paying(99_045, 5),
                        vout: 0,
                        value: 99_045,
                        block_height: None,
                    },
                }],
                ..Default::default()
            };
            let ids = [ordinary.id.clone(), refunded.id.clone()];
            let nodes: HashMap<TreeNodeId, TreeNode> = [ordinary, refunded]
                .into_iter()
                .map(|node| (node.id.clone(), node))
                .collect();

            let quote = quote_unilateral_exit(
                &nodes,
                &ids,
                UnilateralExitLeafFilter::All,
                272,
                22,
                DUST,
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &on_chain,
            )
            .unwrap();

            assert_eq!(quote.refund_fee_sat, 955);
            assert!(quote.cpfp_fee_sat > 0);
            assert_eq!(
                quote.total_fee_sat,
                quote.cpfp_fee_sat + quote.fanout_fee_sat + 955 + quote.sweep_fee_sat
            );
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
                &ExitChainState::default(),
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
                &ExitChainState::default(),
            )
            .unwrap();
            assert!(sel.is_empty());
        }

        /// Selection never consults the status. Gating on the operators' label used
        /// to strand a `TransferLocked` or `Lost` leaf whose pre-signed refund was
        /// still perfectly broadcastable, and an already-`Exited` one has to stay
        /// selectable too so re-running an exit part-way out picks it back up.
        #[test_all]
        fn selects_leaf_in_any_status() {
            for status in [
                TreeNodeStatus::Available,
                TreeNodeStatus::TransferLocked,
                TreeNodeStatus::SplitLocked,
                TreeNodeStatus::Splitted,
                TreeNodeStatus::RenewLocked,
                TreeNodeStatus::Investigation,
                TreeNodeStatus::Lost,
                TreeNodeStatus::OnChain,
                TreeNodeStatus::Exited,
                TreeNodeStatus::Aggregated,
                TreeNodeStatus::Reimbursed,
                TreeNodeStatus::ParentExited,
                TreeNodeStatus::Unknown,
            ] {
                let mut node = leaf_node("leaf", 1_000_000);
                node.status = status;
                let id = node.id.clone();
                let nodes: HashMap<TreeNodeId, TreeNode> =
                    [(id.clone(), node)].into_iter().collect();

                let sel = evaluate_unilateral_exit_leaf_costs(
                    &nodes,
                    std::slice::from_ref(&id),
                    &cost_params(),
                    UnilateralExitLeafFilter::ProfitableOnly,
                    &ExitChainState::default(),
                )
                .unwrap();
                assert_eq!(sel.len(), 1, "a {status} leaf must stay exitable");
            }
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
                &ExitChainState::default(),
            )
            .unwrap()[0]
                .estimated_cost;
            assert_eq!(cost, 526);

            let at = leaf_node("leaf", cost);
            let at_id = at.id.clone();
            let at_nodes: HashMap<TreeNodeId, TreeNode> =
                [(at_id.clone(), at)].into_iter().collect();
            assert!(
                evaluate_unilateral_exit_leaf_costs(
                    &at_nodes,
                    &[at_id],
                    &cost_params(),
                    UnilateralExitLeafFilter::ProfitableOnly,
                    &ExitChainState::default(),
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
                &ExitChainState::default(),
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
                &ExitChainState::default(),
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
                    UnilateralExitLeafFilter::All,
                    &ExitChainState::default(),
                )
                .is_err()
            );
            let sel = evaluate_unilateral_exit_leaf_costs(
                &nodes,
                &[id],
                &cost_params(),
                UnilateralExitLeafFilter::ProfitableOnly,
                &ExitChainState::default(),
            )
            .unwrap();
            assert!(sel.is_empty());
        }

        #[test_all]
        fn evaluate_takes_only_unilaterally_exitable_statuses() {
            for status in [
                TreeNodeStatus::WatchtowerExited,
                TreeNodeStatus::WatchtowerExitRecovered,
            ] {
                let mut node = leaf_node("leaf", 1_000_000);
                node.status = status;
                let id = node.id.clone();
                let nodes: HashMap<TreeNodeId, TreeNode> =
                    [(id.clone(), node)].into_iter().collect();

                assert!(
                    evaluate_unilateral_exit_leaf_costs(
                        &nodes,
                        std::slice::from_ref(&id),
                        &cost_params(),
                        UnilateralExitLeafFilter::All,
                        &ExitChainState::default(),
                    )
                    .is_err()
                );
                let sel = evaluate_unilateral_exit_leaf_costs(
                    &nodes,
                    &[id],
                    &cost_params(),
                    UnilateralExitLeafFilter::ProfitableOnly,
                    &ExitChainState::default(),
                )
                .unwrap();
                assert!(sel.is_empty());
            }
        }

        fn without_anchor(tx: &Transaction) -> Transaction {
            let mut tx = tx.clone();
            tx.output.retain(|out| !is_ephemeral_anchor_output(out));
            tx
        }

        /// A leaf whose nodes are on-chain and whose refund pays its own fee has
        /// nothing left for the funding: it is planned and quoted without any.
        #[test_all]
        fn a_leaf_left_with_an_anchorless_refund_needs_no_funding() {
            let mut node = leaf_node_n("leaf", 1_000_000, 1);
            let refund = without_anchor(node.refund_tx.as_ref().unwrap());
            node.refund_tx = Some(refund.clone());
            let leaf_id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> =
                [(leaf_id.clone(), node)].into_iter().collect();
            let on_chain = on_chain_state(std::slice::from_ref(&leaf_id), &[]);

            let quote = quote_unilateral_exit(
                &nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::All,
                272,
                22,
                DUST,
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &on_chain,
            )
            .unwrap();
            assert!(quote.per_branch_funding.is_empty());
            assert_eq!(quote.single_utxo_funding_sat, 0);
            assert_eq!(
                quote.refund_fee_sat,
                anchorless_refund_package_fee(&refund, 250),
                "the leaf still pays its refund's fee"
            );

            let plan = plan_unilateral_exit(
                nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::All,
                Vec::new(),
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &on_chain,
            )
            .unwrap();
            assert_eq!(plan.selected_leaves.len(), 1);
            assert!(plan.funding.is_empty());
        }

        #[test_all]
        fn an_anchorless_refund_costs_the_leaf_not_the_funding() {
            let anchored = leaf_node("leaf", 1_000_000);
            let mut anchorless = anchored.clone();
            let refund = without_anchor(anchored.refund_tx.as_ref().unwrap());
            anchorless.refund_tx = Some(refund.clone());
            let id = anchored.id.clone();
            let cost_of = |node: TreeNode| {
                let nodes: HashMap<TreeNodeId, TreeNode> =
                    [(id.clone(), node)].into_iter().collect();
                evaluate_unilateral_exit_leaf_costs(
                    &nodes,
                    std::slice::from_ref(&id),
                    &cost_params(),
                    UnilateralExitLeafFilter::All,
                    &ExitChainState::default(),
                )
                .unwrap()
                .remove(0)
            };
            let with_anchor = cost_of(anchored.clone());
            let without = cost_of(anchorless);

            let refund_cpfp = compute_cpfp_package_fee(
                anchored.refund_tx.as_ref().unwrap().weight(),
                Weight::from_wu(272),
                22,
                250,
            );
            assert_eq!(without.cpfp_cost, with_anchor.cpfp_cost - refund_cpfp);
            assert_eq!(with_anchor.refund_fee, 0);
            assert_eq!(
                without.refund_fee,
                anchorless_refund_package_fee(&refund, 250)
            );
            // The sweep still takes back what the funding of the node's child
            // has left.
            assert_eq!(without.funding.len(), 1);
            assert_eq!(
                without.estimated_cost - without.cpfp_cost,
                with_anchor.estimated_cost - with_anchor.cpfp_cost
            );
        }

        #[test_all]
        fn a_leaf_that_cannot_pay_its_anchorless_refund_is_unexitable() {
            let mut node = leaf_node("leaf", 400);
            node.refund_tx = Some(without_anchor(node.refund_tx.as_ref().unwrap()));
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();
            assert!(
                evaluate_unilateral_exit_leaf_costs(
                    &nodes,
                    std::slice::from_ref(&id),
                    &cost_params(),
                    UnilateralExitLeafFilter::All,
                    &ExitChainState::default(),
                )
                .is_err()
            );
        }

        const DUST: u64 = 330;

        #[test_all]
        fn quote_reports_the_leaves_it_leaves_out() {
            let worth_it = leaf_node_n("worth-it", 1_000_000, 1);
            let too_small = leaf_node_n("too-small", 400, 2);
            let mut no_refund = leaf_node_n("no-refund", 1_000_000, 3);
            no_refund.refund_tx = None;
            let ids = [
                worth_it.id.clone(),
                too_small.id.clone(),
                no_refund.id.clone(),
            ];
            let nodes: HashMap<TreeNodeId, TreeNode> = [worth_it, too_small, no_refund]
                .into_iter()
                .map(|node| (node.id.clone(), node))
                .collect();

            let quote = quote_unilateral_exit(
                &nodes,
                &ids,
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                DUST,
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
            )
            .unwrap();

            assert_eq!(quote.selected_leaves.len(), 1);
            assert_eq!(quote.selected_leaves[0].id, ids[0]);
            let skipped: Vec<(TreeNodeId, u64, UnilateralExitSkipReason)> = quote
                .skipped_leaves
                .into_iter()
                .map(|leaf| (leaf.id, leaf.value, leaf.reason))
                .collect();
            assert_eq!(
                skipped,
                vec![
                    (ids[1].clone(), 400, UnilateralExitSkipReason::Unprofitable),
                    (
                        ids[2].clone(),
                        1_000_000,
                        UnilateralExitSkipReason::Unexitable("no refund transaction".to_string())
                    ),
                ]
            );
        }

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
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
            )
            .unwrap();
            assert_eq!(quote.selected_leaves.len(), 1);
            let est = quote.selected_leaves[0].estimated_cost;
            assert_eq!(est, 526);

            assert_eq!(quote.fanout_fee_sat, 0);
            assert_eq!(quote.per_branch_funding.len(), 1);
            assert_eq!(quote.per_branch_funding[0].1, est + DUST);
            assert_eq!(quote.single_utxo_funding_sat, est + DUST);
            assert_eq!(quote.total_fee_sat, est);
            assert_eq!(quote.cpfp_fee_sat, quote.selected_leaves[0].cpfp_cost);
            assert_eq!(quote.sweep_fee_sat, est - quote.cpfp_fee_sat);
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
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
            )
            .unwrap();

            // a is the first selected, so it carries the initial CPFP input and a
            // full sweep input (526); b's sweep cost is only the incremental input
            // (485). The fan-out fee (141) is charged once over both branches.
            let est: Vec<u64> = quote
                .selected_leaves
                .iter()
                .map(|l| l.estimated_cost)
                .collect();
            assert_eq!(est, vec![526, 485]);
            assert_eq!(quote.fanout_fee_sat, 141);
            let funding: Vec<u64> = quote.per_branch_funding.iter().map(|(_, s)| *s).collect();
            assert_eq!(funding, vec![526 + DUST, 485 + DUST]);
            assert_eq!(quote.total_fee_sat, 526 + 485 + 141);
            assert_eq!(
                quote.single_utxo_funding_sat,
                (526 + DUST) + (485 + DUST) + 141
            );

            // The per-leaf sweep shares are marginal, so they telescope to the fee
            // of the single sweep that spends both leaves' inputs.
            let per_leaf_input_weight = p2tr_key_path_input_weight() + Weight::from_wu(272);
            assert_eq!(
                quote.sweep_fee_sat,
                compute_sweep_fee(per_leaf_input_weight * 2, 22, 250)
            );
            assert_eq!(
                quote.cpfp_fee_sat + quote.fanout_fee_sat + quote.sweep_fee_sat,
                quote.total_fee_sat
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
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
            )
            .unwrap();
            let funding: Vec<u64> = quote.per_branch_funding.iter().map(|(_, s)| *s).collect();
            assert_eq!(funding, vec![820, 779]);

            let fund_at = |a_sat: u64, b_sat: u64| {
                plan_unilateral_exit(
                    nodes.clone(),
                    &[ida.clone(), idb.clone()],
                    UnilateralExitLeafFilter::ProfitableOnly,
                    vec![cpfp_input(a_sat, 0), cpfp_input(b_sat, 1)],
                    CpfpFundingStrategy::PerBranch,
                    250,
                    22,
                    &ExitChainState::default(),
                )
            };

            // Funded at exactly the quote: a clean one-UTXO-per-branch plan, no fan-out.
            let plan = fund_at(820, 779).unwrap();
            assert!(plan.fan_out_psbt.is_none());
            assert_eq!(plan.funding.len(), 2);
            assert!(plan.funding.iter().all(|f| f.inputs.len() == 1));

            // One sat short on either branch and the exit can no longer be funded.
            assert!(fund_at(819, 779).is_err());
            assert!(fund_at(820, 778).is_err());
        }

        #[test_all]
        fn plan_leaves_under_one_root_funded_at_quote_amounts_needs_no_fan_out() {
            let (nodes, _, leaf_ids) = root_with_leaves(&[1_000_000, 1_000_000]);
            let dust = test_script().minimal_non_dust().to_sat();
            let quote = quote_unilateral_exit(
                &nodes,
                &leaf_ids,
                UnilateralExitLeafFilter::ProfitableOnly,
                272,
                22,
                dust,
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
            )
            .unwrap();
            // The first amount includes the fee of the root's child.
            let funding: Vec<u64> = quote.per_branch_funding.iter().map(|(_, s)| *s).collect();
            assert_eq!(funding, vec![995, 779]);

            let fund_at = |a_sat: u64, b_sat: u64| {
                plan_unilateral_exit(
                    nodes.clone(),
                    &leaf_ids,
                    UnilateralExitLeafFilter::ProfitableOnly,
                    vec![cpfp_input(a_sat, 0), cpfp_input(b_sat, 1)],
                    CpfpFundingStrategy::PerBranch,
                    250,
                    22,
                    &ExitChainState::default(),
                )
            };

            let plan = fund_at(995, 779).unwrap();
            assert!(plan.fan_out_psbt.is_none());
            assert!(plan.funding.iter().all(|f| f.inputs.len() == 1));

            assert!(fund_at(994, 779).is_err());
            assert!(fund_at(995, 778).is_err());
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
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
            )
            .unwrap();
            // Two per-branch amounts (820 + 779) plus one fan-out fee (141).
            assert_eq!(quote.single_utxo_funding_sat, 1_740);

            let fund_one = |sat: u64| {
                plan_unilateral_exit(
                    nodes.clone(),
                    &[ida.clone(), idb.clone()],
                    UnilateralExitLeafFilter::ProfitableOnly,
                    vec![cpfp_input(sat, 0)],
                    CpfpFundingStrategy::PerBranch,
                    250,
                    22,
                    &ExitChainState::default(),
                )
            };

            // One UTXO can't fund two branches directly, so the plan fans out. At the
            // exact quote there is no surplus, so each branch output is its quoted amount.
            let plan = fund_one(quote.single_utxo_funding_sat).unwrap();
            assert!(plan.fan_out_psbt.is_some());
            let branch_outputs: Vec<u64> = plan
                .funding
                .iter()
                .map(|f| f.inputs[0].witness_utxo.value.to_sat())
                .collect();
            assert_eq!(branch_outputs, vec![820, 779]);

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
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
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
                    CpfpFundingStrategy::PerBranch,
                    250,
                    22,
                    &ExitChainState::default(),
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
            assert_eq!(plan.funding.len(), 1);
            match fund(floor - 1) {
                Err(ServiceError::InsufficientCpfpBudget { required_sat }) => {
                    assert_eq!(required_sat, floor);
                }
                other => panic!("expected InsufficientCpfpBudget, got {other:?}"),
            }
        }

        #[test_all]
        fn plan_falls_back_to_fan_out_when_a_branch_input_is_too_heavy() {
            // assign_inputs matches by amount and would give each of two chains one
            // UTXO. One UTXO is far heavier than the reference inputs[0]
            // (a Custom funding kind), so its own first-child fee exceeds its value:
            // assign_inputs rejects the split and the plan fans out, rather than
            // passing a plan build_cpfp_child would then fail.
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
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
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
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
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
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
            )
            .unwrap();
            assert!(plan.fan_out_psbt.is_some());
            assert_eq!(plan.funding.len(), 3);
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

        /// A leaf's refund: an output that holds `value`, then the anchor.
        fn refund_paying(value: u64, nonce: u32) -> Transaction {
            let mut tx = anchor_tx_n(nonce);
            tx.output.insert(
                0,
                TxOut {
                    value: Amount::from_sat(value),
                    script_pubkey: ScriptBuf::new(),
                },
            );
            tx
        }

        fn tx_paying(value: u64, nonce: u32) -> Transaction {
            Transaction {
                version: Version::non_standard(3),
                lock_time: LockTime::from_height(nonce).unwrap(),
                input: Vec::new(),
                output: vec![TxOut {
                    value: Amount::from_sat(value),
                    script_pubkey: ScriptBuf::new(),
                }],
            }
        }

        #[test_all]
        fn a_node_the_scan_did_not_record_is_charged_whatever_its_status() {
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
                    &ExitChainState::default(),
                )
                .unwrap()[0]
                    .estimated_cost
            };

            // Root, leaf and refund each get a child (175 each), and the sweep
            // costs 167. The builder builds the root's child whatever status the
            // operators report for the root.
            let cost_all = cost_of(&chain(TreeNodeStatus::Available));
            let cost_onchain = cost_of(&chain(TreeNodeStatus::OnChain));
            assert_eq!(cost_all, 692);
            assert_eq!(cost_onchain, 692);
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
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
            )
            .unwrap();
            assert!(quote.selected_leaves.is_empty());
            assert_eq!(quote.per_branch_funding.len(), 0);
            assert_eq!(quote.single_utxo_funding_sat, 0);
            assert_eq!(quote.fanout_fee_sat, 0);
            assert_eq!(quote.total_fee_sat, 0);
        }

        fn refunded_leaves() -> (
            HashMap<TreeNodeId, TreeNode>,
            Vec<TreeNodeId>,
            ExitChainState,
        ) {
            let leaves = [leaf_node_n("a", 100_000, 1), leaf_node_n("b", 100_000, 2)];
            let ids: Vec<TreeNodeId> = leaves.iter().map(|leaf| leaf.id.clone()).collect();
            let mut on_chain = on_chain_state(&ids, &[]);
            on_chain.refunds = leaves
                .iter()
                .map(|leaf| ExitRefund {
                    leaf_id: leaf.id.clone(),
                    state: ExitRefundState::OnChain {
                        tx: leaf.refund_tx.clone().unwrap(),
                        vout: 0,
                        value: leaf.value,
                        block_height: None,
                    },
                })
                .collect();
            let nodes = leaves
                .into_iter()
                .map(|leaf| (leaf.id.clone(), leaf))
                .collect();
            (nodes, ids, on_chain)
        }

        #[test_all]
        fn leaves_with_only_the_sweep_left_need_no_funding() {
            let (nodes, ids, on_chain) = refunded_leaves();

            let quote = quote_unilateral_exit(
                &nodes,
                &ids,
                UnilateralExitLeafFilter::All,
                272,
                22,
                DUST,
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &on_chain,
            )
            .unwrap();
            assert_eq!(quote.selected_leaves.len(), 2);
            assert!(quote.per_branch_funding.is_empty());
            assert_eq!(quote.fanout_fee_sat, 0);
            assert_eq!(quote.single_utxo_funding_sat, 0);

            let plan = plan_unilateral_exit(
                nodes,
                &ids,
                UnilateralExitLeafFilter::All,
                Vec::new(),
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &on_chain,
            )
            .unwrap();
            assert_eq!(plan.selected_leaves.len(), 2);
            assert!(plan.fan_out_psbt.is_none());
        }

        #[test_all]
        fn a_leaf_with_steps_left_needs_funding_to_plan() {
            let node = leaf_node("leaf", 1_000_000);
            let id = node.id.clone();
            let nodes: HashMap<TreeNodeId, TreeNode> = [(id.clone(), node)].into_iter().collect();

            let result = plan_unilateral_exit(
                nodes,
                &[id],
                UnilateralExitLeafFilter::All,
                Vec::new(),
                CpfpFundingStrategy::PerBranch,
                250,
                22,
                &ExitChainState::default(),
            );

            assert!(matches!(result, Err(ServiceError::InvalidInput(_))));
        }
    }

    mod per_node {
        use super::*;
        use std::str::FromStr;

        const LOW: u64 = 250; // 1 sat/vB
        const HIGH: u64 = 38_250; // 153 sat/vB

        fn id(s: &str) -> TreeNodeId {
            TreeNodeId::from_str(s).unwrap()
        }

        fn p2wpkh() -> ScriptBuf {
            let secp = Secp256k1::new();
            let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
            let pk = PublicKey::from_secret_key(&secp, &sk);
            Address::p2wpkh(&CompressedPublicKey(pk), bitcoin::Network::Testnet).script_pubkey()
        }

        /// A v3 tx with a P2A anchor, its txid unique to `nonce`. The input's
        /// witness sets its weight one unit at a time without touching the txid.
        fn anchored_tx(nonce: u32, witness_len: usize) -> Transaction {
            Transaction {
                version: Version::non_standard(3),
                lock_time: LockTime::from_height(nonce).unwrap(),
                input: vec![TxIn {
                    witness: Witness::from_slice(&[vec![0u8; witness_len]]),
                    ..Default::default()
                }],
                output: vec![TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
                }],
            }
        }

        fn node(name: &str, parent: Option<&str>, node_tx: Transaction) -> TreeNode {
            let mut node = crate::tree::tests::create_test_tree_node(name, 1_000_000);
            node.parent_node_id = parent.map(id);
            node.node_tx = node_tx;
            node
        }

        fn leaf(name: &str, parent: Option<&str>, nonce: u32) -> TreeNode {
            let mut leaf = node(name, parent, anchored_tx(nonce, 0));
            leaf.refund_tx = Some(anchored_tx(nonce + 1_000, 0));
            leaf
        }

        fn tree(nodes: Vec<TreeNode>) -> HashMap<TreeNodeId, TreeNode> {
            nodes.into_iter().map(|n| (n.id.clone(), n)).collect()
        }

        fn input(value: u64, vout: u32) -> CpfpInput {
            CpfpInput {
                outpoint: OutPoint {
                    txid: Txid::from_byte_array([7u8; 32]),
                    vout,
                },
                witness_utxo: TxOut {
                    value: Amount::from_sat(value),
                    script_pubkey: p2wpkh(),
                },
                signed_input_weight: 272,
            }
        }

        fn selected_all(
            nodes: &HashMap<TreeNodeId, TreeNode>,
            leaves: &[TreeNodeId],
            on_chain: &ExitChainState,
        ) -> Vec<UnilateralExitSelectedLeaf> {
            let params = UnilateralExitLeafCostParams {
                initial_cpfp_input_weight: Weight::from_wu(272),
                single_cpfp_input_weight: Weight::from_wu(272),
                change_script_len: 22,
                change_dust_limit: 0,
                destination_script_len: 22,
                fee_rate_sat_per_kw: LOW,
                funding_strategy: CpfpFundingStrategy::PerNode,
            };
            evaluate_unilateral_exit_leaf_costs(
                nodes,
                leaves,
                &params,
                UnilateralExitLeafFilter::All,
                on_chain,
            )
            .unwrap()
        }

        fn bumped(
            nodes: &HashMap<TreeNodeId, TreeNode>,
            leaves: &[TreeNodeId],
            on_chain: &ExitChainState,
        ) -> Vec<Txid> {
            selected_all(nodes, leaves, on_chain)
                .iter()
                .flat_map(|leaf| &leaf.funding)
                .flat_map(|entry| &entry.chain.parents)
                .map(|parent| parent.txid)
                .collect()
        }

        /// What a one-input child of `tx` needs at `rate`: the floor
        /// `build_cpfp_child` enforces.
        fn needed(tx: &Transaction, rate: u64) -> u64 {
            compute_cpfp_package_fee(tx.weight(), Weight::from_wu(272), 22, rate)
                + p2wpkh().minimal_non_dust().to_sat()
        }

        fn plan(
            nodes: HashMap<TreeNodeId, TreeNode>,
            leaves: &[TreeNodeId],
            inputs: Vec<CpfpInput>,
            rate: u64,
        ) -> Result<UnilateralExitPlan, ServiceError> {
            plan_with(nodes, leaves, inputs, CpfpFundingStrategy::PerNode, rate)
        }

        fn plan_with(
            nodes: HashMap<TreeNodeId, TreeNode>,
            leaves: &[TreeNodeId],
            inputs: Vec<CpfpInput>,
            strategy: CpfpFundingStrategy,
            rate: u64,
        ) -> Result<UnilateralExitPlan, ServiceError> {
            plan_unilateral_exit(
                nodes,
                leaves,
                UnilateralExitLeafFilter::All,
                inputs,
                strategy,
                rate,
                22,
                &ExitChainState::default(),
            )
        }

        /// One input per entry of the quote at `rate`, holding the entry's amount.
        fn funded_for(
            nodes: &HashMap<TreeNodeId, TreeNode>,
            leaves: &[TreeNodeId],
            strategy: CpfpFundingStrategy,
            rate: u64,
        ) -> Vec<CpfpInput> {
            quote_unilateral_exit(
                nodes,
                leaves,
                UnilateralExitLeafFilter::All,
                272,
                22,
                p2wpkh().minimal_non_dust().to_sat(),
                strategy,
                rate,
                22,
                &ExitChainState::default(),
            )
            .unwrap()
            .per_branch_funding
            .iter()
            .zip(0..)
            .map(|((_, sat), vout)| input(*sat, vout))
            .collect()
        }

        /// Which input funds which transaction, as outpoint vouts by txid.
        fn pairing(plan: &UnilateralExitPlan) -> HashMap<Txid, u32> {
            plan.funding
                .iter()
                .flat_map(|f| f.chain.parents.iter().zip(&f.inputs))
                .map(|(parent, input)| (parent.txid, input.outpoint.vout))
                .collect()
        }

        #[test_all]
        fn names_every_transaction_the_build_bumps_in_order() {
            let root = node("root", None, anchored_tx(1, 0));
            let leaf = leaf("leaf", Some("root"), 2);
            let expected = vec![
                root.node_tx.compute_txid(),
                leaf.node_tx.compute_txid(),
                leaf.refund_tx.as_ref().unwrap().compute_txid(),
            ];
            let nodes = tree(vec![root, leaf]);

            assert_eq!(
                bumped(&nodes, &[id("leaf")], &ExitChainState::default()),
                expected,
                "root first, then the leaf's node, then its refund"
            );
        }

        #[test_all]
        fn names_a_shared_ancestor_once() {
            let root = node("root", None, anchored_tx(1, 0));
            let root_txid = root.node_tx.compute_txid();
            let nodes = tree(vec![
                root,
                leaf("leaf-a", Some("root"), 2),
                leaf("leaf-b", Some("root"), 3),
            ]);
            let txids = bumped(
                &nodes,
                &[id("leaf-a"), id("leaf-b")],
                &ExitChainState::default(),
            );

            assert_eq!(txids.len(), 5, "one root, two leaf nodes, two refunds");
            assert_eq!(
                txids.iter().filter(|txid| **txid == root_txid).count(),
                1,
                "the shared root is bumped once, by the first branch"
            );
        }

        #[test_all]
        fn leaves_out_what_the_chain_already_has() {
            let root = node("root", None, anchored_tx(1, 0));
            let leaf = leaf("leaf", Some("root"), 2);
            let leaf_txid = leaf.node_tx.compute_txid();
            let nodes = tree(vec![root, leaf]);
            let root_on_chain = ExitChainState {
                nodes: vec![ConfirmedExitNode {
                    node_id: id("root"),
                    confirmed_by: ExitNodeConfirmation::Cpfp,
                    block_height: Some(100),
                }],
                refunds: vec![ExitRefund {
                    leaf_id: id("leaf"),
                    state: ExitRefundState::Swept,
                }],
                ..Default::default()
            };

            assert_eq!(
                bumped(&nodes, &[id("leaf")], &root_on_chain),
                vec![leaf_txid],
                "only the leaf's node is left to bump"
            );
        }

        #[test_all]
        fn leaves_out_a_stopped_branch_and_a_direct_refund() {
            let mut went_direct = leaf("went-direct", None, 1);
            went_direct.direct_tx = Some(anchored_tx(50, 0));
            went_direct.direct_refund_tx = Some(anchored_tx(51, 0));
            let nodes = tree(vec![went_direct, leaf("stopped", None, 2)]);
            let on_chain = ExitChainState {
                nodes: vec![ConfirmedExitNode {
                    node_id: id("went-direct"),
                    confirmed_by: ExitNodeConfirmation::Direct,
                    block_height: Some(100),
                }],
                stopped_leaves: vec![id("stopped")],
                ..Default::default()
            };

            assert!(
                bumped(&nodes, &[id("went-direct"), id("stopped")], &on_chain).is_empty(),
                "a direct refund pays its own fee and a stopped branch builds nothing"
            );
        }

        #[test_all]
        fn quote_names_one_entry_per_bumped_transaction() {
            let root = node("root", None, anchored_tx(1, 0));
            let leaf = leaf("leaf", Some("root"), 2);
            let amounts = [
                needed(&root.node_tx, LOW),
                needed(&leaf.node_tx, LOW),
                needed(leaf.refund_tx.as_ref().unwrap(), LOW),
            ];
            let nodes = tree(vec![root, leaf]);
            let dust = p2wpkh().minimal_non_dust().to_sat();
            let quote_of = |strategy| {
                quote_unilateral_exit(
                    &nodes,
                    &[id("leaf")],
                    UnilateralExitLeafFilter::All,
                    272,
                    22,
                    dust,
                    strategy,
                    LOW,
                    22,
                    &ExitChainState::default(),
                )
                .unwrap()
            };
            let per_node = quote_of(CpfpFundingStrategy::PerNode);

            assert_eq!(
                per_node
                    .per_branch_funding
                    .iter()
                    .map(|(leaf_id, sat)| (leaf_id.to_string(), *sat))
                    .collect::<Vec<_>>(),
                amounts
                    .iter()
                    .map(|sat| ("leaf".to_string(), *sat))
                    .collect::<Vec<_>>(),
                "one entry per transaction, all of the one leaf, each its child's floor"
            );
            assert_eq!(
                per_node.fanout_fee_sat,
                fan_out_fee(Weight::from_wu(272), 22, 3, LOW),
                "the fan-out splits into one output per transaction"
            );
            assert_eq!(
                per_node.single_utxo_funding_sat,
                amounts.iter().sum::<u64>() + per_node.fanout_fee_sat
            );

            // The CPFP fees are the same either way; the sweep is one input per
            // leaf cheaper, the change it no longer pulls.
            let per_branch = quote_of(CpfpFundingStrategy::PerBranch);
            assert_eq!(per_node.cpfp_fee_sat, per_branch.cpfp_fee_sat);
            assert_eq!(
                per_node.sweep_fee_sat,
                compute_sweep_fee(p2tr_key_path_input_weight(), 22, LOW)
            );
            assert_eq!(
                per_branch.sweep_fee_sat,
                compute_sweep_fee(p2tr_key_path_input_weight() + Weight::from_wu(272), 22, LOW)
            );
            assert_eq!(per_branch.fanout_fee_sat, 0, "one branch needs no fan-out");
        }

        #[test_all]
        fn plan_pairs_the_largest_input_with_the_heaviest_transaction() {
            let mut light = leaf("leaf", None, 1);
            light.node_tx = anchored_tx(1, 0);
            light.refund_tx = Some(anchored_tx(1_001, 400));
            let node_txid = light.node_tx.compute_txid();
            let refund_txid = light.refund_tx.as_ref().unwrap().compute_txid();
            let nodes = tree(vec![light]);

            let plan = plan(
                nodes,
                &[id("leaf")],
                vec![input(5_000, 0), input(9_000, 1)],
                LOW,
            )
            .unwrap();

            assert!(plan.fan_out_psbt.is_none());
            let paired = pairing(&plan);
            assert_eq!(
                paired[&refund_txid], 1,
                "the heavier refund takes the larger input"
            );
            assert_eq!(paired[&node_txid], 0);
        }

        /// The property per-node funding exists for: a caller funds each
        /// transaction once, at the amounts quoted for the highest fee rate it will
        /// use, then builds the exit once per fee rate, and every build has to
        /// spend the same UTXO for the same transaction or the children it signed
        /// for different rates do not replace each other.
        ///
        /// The two node transactions here are two weight units apart. At 1 sat/vB
        /// that rounds to the same fee, so they need the same amount; at 153 sat/vB
        /// the heavier one needs more. A tie broken by txid would therefore order
        /// them differently at the two rates and swap their inputs.
        #[test_all]
        fn plan_keeps_each_input_on_its_transaction_at_every_fee_rate() {
            // Find a weight where the two tie at the low rate and the lighter one
            // has the smaller txid, so a tie broken by txid puts it first.
            let (light, heavy) = (0..400)
                .flat_map(|witness| (0..20u32).map(move |nonce| (witness, nonce)))
                .map(|(witness, nonce)| {
                    (
                        anchored_tx(10 + nonce, witness),
                        anchored_tx(2_000 + nonce, witness + 2),
                    )
                })
                .find(|(light, heavy)| {
                    needed(light, LOW) == needed(heavy, LOW)
                        && needed(light, HIGH) < needed(heavy, HIGH)
                        && light.compute_txid() < heavy.compute_txid()
                })
                .expect("a pair that ties at the low rate and not the high one");

            let mut leaf = leaf("leaf", Some("root"), 1);
            leaf.node_tx = heavy.clone();
            let refund_txid = leaf.refund_tx.as_ref().unwrap().compute_txid();
            let nodes = tree(vec![node("root", None, light.clone()), leaf]);
            let quoted = quote_unilateral_exit(
                &nodes,
                &[id("leaf")],
                UnilateralExitLeafFilter::All,
                272,
                22,
                p2wpkh().minimal_non_dust().to_sat(),
                CpfpFundingStrategy::PerNode,
                HIGH,
                22,
                &ExitChainState::default(),
            )
            .unwrap();
            let funding: Vec<CpfpInput> = quoted
                .per_branch_funding
                .iter()
                .zip(0..)
                .map(|((_, sat), vout)| input(*sat, vout))
                .collect();

            for rate in [LOW, 2_500, 12_500, HIGH] {
                let plan = plan(nodes.clone(), &[id("leaf")], funding.clone(), rate).unwrap();
                assert!(
                    plan.fan_out_psbt.is_none(),
                    "funded for {HIGH}, so {rate} needs no fan-out"
                );
                let paired = pairing(&plan);
                assert_eq!(
                    paired[&light.compute_txid()],
                    0,
                    "at {rate} sat/kw the lighter transaction keeps its own input"
                );
                assert_eq!(paired[&heavy.compute_txid()], 1);
                assert_eq!(paired[&refund_txid], 2);
            }
        }

        /// At a zero fee rate every amount is the dust limit, so under either
        /// strategy the entries of a light and a heavy leaf tie. A tie broken by
        /// leaf id would give the light leaf, whose id sorts first, the inputs of
        /// the heavy one.
        #[test_all]
        fn plan_keeps_each_input_on_its_leaf_at_a_zero_fee_rate() {
            let mut heavy = leaf("b", None, 2);
            heavy.node_tx = anchored_tx(2, 400);
            heavy.refund_tx = Some(anchored_tx(1_002, 400));
            let nodes = tree(vec![leaf("a", None, 1), heavy]);
            let leaves = [id("a"), id("b")];

            for strategy in [CpfpFundingStrategy::PerBranch, CpfpFundingStrategy::PerNode] {
                let funding = funded_for(&nodes, &leaves, strategy, HIGH);
                let at = |rate| {
                    plan_with(nodes.clone(), &leaves, funding.clone(), strategy, rate).unwrap()
                };
                let at_zero = at(0);
                assert!(at_zero.fan_out_psbt.is_none(), "{strategy:?}");
                assert_eq!(pairing(&at_zero), pairing(&at(HIGH)), "{strategy:?}");
            }
        }

        #[test_all]
        fn plan_ignores_the_order_inputs_arrive_in() {
            let nodes = tree(vec![
                node("root", None, anchored_tx(1, 0)),
                leaf("leaf", Some("root"), 2),
            ]);
            let inputs = vec![input(7_000, 0), input(9_000, 1), input(8_000, 2)];
            let mut reversed = inputs.clone();
            reversed.reverse();

            let forward = plan(nodes.clone(), &[id("leaf")], inputs, LOW).unwrap();
            let backward = plan(nodes, &[id("leaf")], reversed, LOW).unwrap();
            assert_eq!(pairing(&forward), pairing(&backward));
        }

        #[test_all]
        fn plan_leaves_the_smallest_surplus_input_out() {
            let nodes = tree(vec![leaf("leaf", None, 1)]);
            let plan = plan(
                nodes,
                &[id("leaf")],
                vec![input(400, 0), input(9_000, 1), input(8_000, 2)],
                LOW,
            )
            .unwrap();

            assert!(plan.fan_out_psbt.is_none());
            let mut used: Vec<u32> = pairing(&plan).into_values().collect();
            used.sort_unstable();
            assert_eq!(used, vec![1, 2], "the 400 sat input is the one left over");
        }

        #[test_all]
        fn plan_funding_boundary_is_exact() {
            let leaf = leaf("leaf", None, 1);
            let refund_txid = leaf.refund_tx.as_ref().unwrap().compute_txid();
            let nodes = tree(vec![leaf]);
            let dust = p2wpkh().minimal_non_dust().to_sat();
            let quoted = quote_unilateral_exit(
                &nodes,
                &[id("leaf")],
                UnilateralExitLeafFilter::All,
                272,
                22,
                dust,
                CpfpFundingStrategy::PerNode,
                LOW,
                22,
                &ExitChainState::default(),
            )
            .unwrap();
            let [(_, node_sat), (_, refund_sat)] = quoted.per_branch_funding[..] else {
                panic!("one entry for the node and one for the refund");
            };
            let funded = |node_sat, refund_sat| {
                plan(
                    nodes.clone(),
                    &[id("leaf")],
                    vec![input(node_sat, 0), input(refund_sat, 1)],
                    LOW,
                )
            };

            let exact = funded(node_sat, refund_sat).unwrap();
            assert!(
                exact.fan_out_psbt.is_none(),
                "the quoted amounts are enough"
            );
            for f in &exact.funding {
                let txid = f.chain.parents[0].txid;
                let leaf = &exact.tree_nodes[&id("leaf")];
                let parent = if txid == refund_txid {
                    leaf.refund_tx.clone().unwrap()
                } else {
                    leaf.node_tx.clone()
                };
                let child = build_cpfp_child(&parent, &f.inputs, LOW).unwrap();
                assert_eq!(
                    child.change_input.witness_utxo.value.to_sat(),
                    dust,
                    "funded at the quoted amount, the change is exactly dust"
                );
            }

            // One sat short on the larger: no split covers, so the two are fanned
            // out instead, and together they are too little for that too.
            let short = funded(node_sat, refund_sat - 1);
            assert!(
                matches!(short, Err(ServiceError::InsufficientCpfpBudget { .. })),
                "got {short:?}"
            );
        }

        #[test_all]
        fn plan_fans_out_one_output_per_bumped_transaction() {
            let nodes = tree(vec![
                node("root", None, anchored_tx(1, 0)),
                leaf("leaf", Some("root"), 2),
            ]);
            let plan = plan(nodes, &[id("leaf")], vec![input(100_000, 0)], LOW).unwrap();

            let fan_out = plan
                .fan_out_psbt
                .as_ref()
                .expect("one input for three transactions");
            assert_eq!(fan_out.unsigned_tx.output.len(), 3);
            let fan_out_txid = fan_out.unsigned_tx.compute_txid();
            let mut vouts: Vec<u32> = plan
                .funding
                .iter()
                .flat_map(|f| &f.inputs)
                .map(|input| {
                    assert_eq!(input.outpoint.txid, fan_out_txid);
                    input.outpoint.vout
                })
                .collect();
            vouts.sort_unstable();
            assert_eq!(
                vouts,
                vec![0, 1, 2],
                "each transaction gets an output of its own"
            );
            assert_eq!(plan.funding.len(), 3);
        }

        #[test_all]
        fn plan_checks_each_input_against_its_own_weight() {
            let leaf = leaf("leaf", None, 1);
            let needs =
                needed(&leaf.node_tx, LOW).max(needed(leaf.refund_tx.as_ref().unwrap(), LOW));
            let nodes = tree(vec![leaf]);
            // Exactly what a P2WPKH input needs, but this one signs heavier, so it
            // cannot pay whichever transaction it is paired with. The other input
            // is ample, so the fan-out that takes their place can be built.
            let mut heavy = input(needs, 1);
            heavy.signed_input_weight = 600;

            let plan = plan(nodes, &[id("leaf")], vec![input(50_000, 0), heavy], LOW).unwrap();
            assert!(
                plan.fan_out_psbt.is_some(),
                "an input short of its own child's floor is fanned out, not built to fail"
            );
        }

        #[test_all]
        fn plan_needs_no_funding_when_nothing_is_left_to_bump() {
            let nodes = tree(vec![leaf("leaf", None, 1)]);
            let done = ExitChainState {
                nodes: vec![ConfirmedExitNode {
                    node_id: id("leaf"),
                    confirmed_by: ExitNodeConfirmation::Cpfp,
                    block_height: Some(100),
                }],
                refunds: vec![ExitRefund {
                    leaf_id: id("leaf"),
                    state: ExitRefundState::Swept,
                }],
                ..Default::default()
            };
            let plan = plan_unilateral_exit(
                nodes,
                &[id("leaf")],
                UnilateralExitLeafFilter::All,
                Vec::new(),
                CpfpFundingStrategy::PerNode,
                LOW,
                22,
                &done,
            )
            .unwrap();

            assert!(plan.funding.is_empty());
            assert_eq!(plan.selected_leaves.len(), 1, "the leaf keeps its branch");
        }
    }
}
