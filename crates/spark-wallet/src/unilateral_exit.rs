use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use bitcoin::{Address, Amount, OutPoint, ScriptBuf, Transaction, TxOut, Txid};
use spark::{
    services::{
        CpfpChild, CpfpFundingShape, CpfpInput, ServiceError, UnilateralExitNodeFunding,
        UnilateralExitPlan, build_cpfp_child, csv_timelock, walk_unilateral_exit_chain,
    },
    tree::{LeafPedigree, TreeNode, TreeNodeId, TreeNodeStatus},
    utils::transactions::is_ephemeral_anchor_output,
};
use tracing::{debug, trace, warn};

use crate::SparkWalletError;

/// Which leaves to unilaterally exit.
#[derive(Clone, Debug)]
pub enum ExitLeafSelection {
    /// Exit every available leaf whose value exceeds its marginal exit cost.
    Auto,
    /// Exit exactly these leaves, regardless of profitability.
    Specific(Vec<TreeNodeId>),
}

/// Everything needed to unilaterally exit the wallet's leaves with the
/// operators unreachable: each leaf paired with its ancestor chain.
#[derive(Clone, Debug)]
pub struct ExitStateExport {
    pub pedigrees: Vec<LeafPedigree>,
}

/// How a restored exit state landed in the tree store.
#[derive(Clone, Copy, Debug)]
pub struct ExitStateImport {
    /// Leaves written, with or without their chain.
    pub imported_leaves: usize,
    /// Leaves dropped because they do not record this wallet as their owner.
    pub skipped_foreign_leaves: usize,
    /// Leaves dropped because the entry disagrees with a node the wallet already
    /// holds, on a field fixed for that node's lifetime, so none of the entry
    /// could be trusted. Nothing was written for these, and the wallet is left
    /// without the leaf.
    pub skipped_conflicting_leaves: usize,
    /// Leaves the wallet holds whose incoming chain was not used: it does not
    /// link the leaf to a root, the stored chain already backs an exit, or the
    /// leaf was named more than once. The leaf itself is in the store either way.
    pub skipped_chains: usize,
}

/// A prepared unilateral exit: the chain-independent plan plus per-leaf refund
/// addresses. Feed to [`next_chain_queries`], then [`build_unilateral_exit`].
#[derive(Clone, Debug)]
pub struct PreparedUnilateralExit {
    pub plan: UnilateralExitPlan,
    /// Every refund variant pays the same leaf key, so this one P2TR address
    /// recognizes an on-chain refund of any variant, and is where the sweep pulls.
    pub leaf_refund_addresses: HashMap<TreeNodeId, Address>,
}

/// The exit's on-chain state, resolved from chain [`Observation`]s by
/// [`interpret_chain`]; empty drives a fresh cpfp exit.
///
/// The pre-signed txs only continue along the cpfp `node_tx` chain (every child
/// and the cpfp refund spend the parent's `node_tx` output), so a node taken
/// on-chain by any non-cpfp tx cannot be continued.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedExitState {
    /// Absent means emit the plan's fresh fan-out.
    pub fan_out: Option<ConfirmedFanOut>,
    /// A node absent from the map is driven: emit its `node_tx` with a fresh child.
    pub nodes: HashMap<TreeNodeId, NodeState>,
    /// A leaf absent from the map has its cpfp `refund_tx` driven fresh.
    pub refunds: HashMap<TreeNodeId, RefundState>,
    /// Leaves whose cpfp lineage was taken on-chain by an uncontinuable tx; the
    /// branch drives nothing and is absent from the built set.
    pub stopped: HashSet<TreeNodeId>,
    /// Supplied funding inputs already confirmed spent (e.g. by a prior run's
    /// child); the build drops these and funds from tracked change plus the rest.
    pub spent_funding: HashSet<OutPoint>,
}

/// How a node was resolved on-chain (absent = driven via cpfp).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeState {
    /// Confirmed via the cpfp `node_tx`. `change` is that node's CPFP-child change
    /// funding the next driven node on resume; `None` when unneeded or unresolved.
    ConfirmedCpfp { change: Option<ConfirmedOutput> },
    /// Confirmed via the self-fee `direct_tx`. Only ever a leaf: an intermediate's
    /// children spend its cpfp output, which a direct spend never creates.
    ConfirmedDirect,
}

/// How a leaf's refund was resolved on-chain (absent = drive its cpfp `refund_tx`).
#[derive(Clone, Debug)]
pub(crate) enum RefundState {
    /// A refund is already on-chain (any variant): adopt its output for the sweep.
    Adopted(ConfirmedRefund),
    /// Leaf went out via `direct_tx`; drive the self-fee `direct_refund_tx` as-is
    /// (pays its own fee, no CPFP child).
    DriveDirect,
    /// Refund confirmed and already swept: nothing to drive or sweep.
    Swept,
}

/// An already-confirmed fan-out adopted in place of building a fresh one. A
/// branch takes as many of its outputs as it has funding inputs: one under
/// per-branch funding, one per fee-bumped transaction under per-node funding.
#[derive(Clone, Debug)]
pub(crate) struct ConfirmedFanOut {
    pub tx: Transaction,
    pub branch_outputs: HashMap<TreeNodeId, Vec<ConfirmedOutput>>,
}

/// An output already sitting on-chain, adopted instead of a freshly-built one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmedOutput {
    pub outpoint: bitcoin::OutPoint,
    pub value: u64,
}

/// A leaf refund already on-chain (any variant), adopted for the sweep.
#[derive(Clone, Debug)]
pub(crate) struct ConfirmedRefund {
    pub tx: Transaction,
    pub outpoint: bitcoin::OutPoint,
    pub value: u64,
}

/// One unilateral-exit transaction and, when it still needs fee-bumping, the
/// unsigned CPFP child that pays its fee.
#[derive(Clone, Debug)]
pub struct ExitTx {
    pub kind: ExitTxKind,
    /// The tree node this tx belongs to (leaf id for a refund); `None` for the fan-out.
    pub node_id: Option<TreeNodeId>,
    pub txid: bitcoin::Txid,
    /// Broadcast unless `status` marks it already-on-chain. The pre-signed exit tx
    /// for Node/Refund, the unsigned fan-out for FanOut.
    pub base_tx: Transaction,
    /// The unsigned PSBT the caller signs; `None` when nothing needs signing (an
    /// adopted fan-out, an already-confirmed step, or a self-fee `direct` tx).
    pub to_sign: Option<bitcoin::Psbt>,
    /// Relative CSV timelock (blocks) that must mature before `base_tx` confirms.
    pub csv_timelock_blocks: Option<u32>,
    /// Txids this tx spends from, so it must be broadcast after them.
    pub depends_on: Vec<bitcoin::Txid>,
    pub status: ExitTxStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitTxKind {
    FanOut,
    Node,
    Refund,
}

/// A built exit tx's on-chain state, resolved from the chain observations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitTxStatus {
    /// On-chain and confirmed (or an adopted, already-confirmed output).
    Confirmed,
    Unconfirmed,
    /// A chain lookup this tx depended on failed, so its state is unknown.
    Unverified,
}

/// One selected leaf's exit transactions, ordered root to leaf and ending with
/// the leaf's refund tx.
#[derive(Clone, Debug)]
pub struct ExitBranch {
    pub leaf_id: TreeNodeId,
    pub txs: Vec<ExitTx>,
}

/// A built unilateral exit: the unsigned transactions plus the sweep inputs. The
/// caller signs each `to_sign` and sweeps `refund_outputs` + `cpfp_change_inputs`
/// via [`SparkWallet::create_refund_sweep_transaction`](crate::SparkWallet::create_refund_sweep_transaction).
#[derive(Clone, Debug)]
pub struct UnilateralExitBuild {
    /// Present only when the funding needed splitting across branches.
    pub fan_out: Option<ExitTx>,
    pub branches: Vec<ExitBranch>,
    /// Every leaf's refund output to sweep (adopted on-chain or freshly driven).
    pub refund_outputs: Vec<RefundOutput>,
    /// Terminal CPFP-change outputs to fold into the sweep (only branches whose
    /// refund child was built fresh).
    pub cpfp_change_inputs: Vec<CpfpChangeInput>,
    pub recoverable_value_sat: u64,
    /// CPFP-package fees of the txs built plus a fresh fan-out's fee; excludes the
    /// sweep fee (the caller adds that).
    pub total_fee_sat: u64,
}

/// A refund output sitting on-chain after a unilateral exit.
#[derive(Clone, Debug)]
pub struct RefundOutput {
    pub outpoint: bitcoin::OutPoint,
    pub leaf_id: TreeNodeId,
    pub value: u64,
}

/// A caller-controlled CPFP-change output (the terminal change of a leaf's CPFP
/// chain) that the sweep absorbs alongside the refund outputs.
#[derive(Clone, Debug)]
pub struct CpfpChangeInput {
    pub outpoint: bitcoin::OutPoint,
    pub witness_utxo: bitcoin::TxOut,
    pub signed_input_weight: u64,
}

/// One on-chain lookup a unilateral exit needs. The caller performs the I/O and
/// reports back an [`Observation`]; `bitcoin` types only, so the wallet owns no
/// chain client.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ChainQuery {
    /// Is this output spent, and by which (confirmed?) transaction?
    Outspend(OutPoint),
    Transaction(Txid),
    /// Scan this leaf's refund address for its refund output of any variant,
    /// spent or not, so a swept refund is recognized as well as an unspent one.
    RefundAddress {
        leaf_id: TreeNodeId,
        address: Address,
    },
}

/// The result of performing a [`ChainQuery`]. `Unavailable` means the lookup
/// failed; the affected tx is then treated as unverified, not confirmed or absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChainResult {
    /// `None` if unspent.
    Spend(Option<SpendInfo>),
    Transaction(Transaction),
    /// Every output ever paid to the address, spent or not.
    AddressUtxos(Vec<AddressUtxo>),
    Unavailable,
}

/// The transaction spending a queried output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpendInfo {
    pub spender_txid: Txid,
    pub confirmed: bool,
}

/// An output found at a refund address, spent or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressUtxo {
    pub txid: Txid,
    pub vout: u32,
    pub value: u64,
    pub confirmed: bool,
}

/// A performed [`ChainQuery`] paired with its [`ChainResult`].
#[derive(Clone, Debug)]
pub struct Observation {
    pub query: ChainQuery,
    pub result: ChainResult,
}

/// The chain lookups a unilateral exit still needs, given what has been observed.
/// Empty means fully resolved: call [`build_unilateral_exit`] with the same args.
/// Drive it in a loop — perform the queries, append [`Observation`]s, call again;
/// each call re-derives from scratch, so it is order-independent and idempotent.
pub fn next_chain_queries(
    prepared: &PreparedUnilateralExit,
    observed: &[Observation],
) -> Result<Vec<ChainQuery>, SparkWalletError> {
    let mut pending = interpret_chain(prepared, observed)?.pending;
    let mut seen: HashSet<ChainQuery> = HashSet::new();
    pending.retain(|query| seen.insert(query.clone()));
    trace!(
        pending = pending.len(),
        observed = observed.len(),
        "next_chain_queries"
    );
    Ok(pending)
}

/// The outcome of interpreting the observations: resolved state plus the lookups
/// still needed while the walk is incomplete.
struct ChainInterpretation {
    resolved: ResolvedExitState,
    pending: Vec<ChainQuery>,
    unverified: HashSet<TreeNodeId>,
    fan_out_unverified: bool,
}

/// An index over the observations for O(1) lookup by query, built once per
/// [`interpret_chain`] pass instead of scanning the growing observation list on
/// every lookup. The first observation of a query wins, matching the prior scan.
struct ObservedIndex<'a> {
    by_query: HashMap<&'a ChainQuery, &'a ChainResult>,
}

impl<'a> ObservedIndex<'a> {
    fn new(observed: &'a [Observation]) -> Self {
        let mut by_query = HashMap::with_capacity(observed.len());
        for obs in observed {
            by_query.entry(&obs.query).or_insert(&obs.result);
        }
        Self { by_query }
    }

    fn get(&self, query: &ChainQuery) -> Option<&'a ChainResult> {
        self.by_query.get(query).copied()
    }
}

/// Resolves the exit's on-chain state from `observed`, emitting the lookups still
/// needed. Pure in `(prepared, observed)`. The walk follows the confirmed spender
/// down each branch (a non-`node_tx` spend breaks it); each leaf's refund is
/// recovered independently by an address scan, so it survives a broken branch.
fn interpret_chain(
    prepared: &PreparedUnilateralExit,
    observed: &[Observation],
) -> Result<ChainInterpretation, SparkWalletError> {
    let plan = &prepared.plan;
    let node_map = &plan.tree_nodes;
    let index = ObservedIndex::new(observed);
    let observed = &index;

    let mut pending: Vec<ChainQuery> = Vec::new();
    let mut unverified: HashSet<TreeNodeId> = HashSet::new();

    let (fan_out, fan_out_unverified) = interpret_fan_out(plan, observed, &mut pending)?;

    let mut nodes: HashMap<TreeNodeId, NodeState> = HashMap::new();
    let mut refunds: HashMap<TreeNodeId, RefundState> = HashMap::new();
    let mut stopped: HashSet<TreeNodeId> = HashSet::new();
    let mut needs_change: HashSet<TreeNodeId> = HashSet::new();
    let mut unverifiable_confirmed: HashSet<TreeNodeId> = HashSet::new();
    for (leaf_id, _) in &plan.per_branch_funding {
        walk_branch(
            node_map,
            leaf_id,
            observed,
            &mut nodes,
            &mut refunds,
            &mut stopped,
            &mut needs_change,
            &mut unverified,
            &mut unverifiable_confirmed,
            &mut pending,
        );
    }

    // Only chained funding needs a confirmed child's change resolved: under
    // per-node funding the next child has a UTXO of its own, so there is nothing
    // to carry forward and nothing a rebuild could double-spend.
    match plan.funding_shape {
        CpfpFundingShape::PerBranch => resolve_confirmed_changes(
            node_map,
            plan,
            &mut nodes,
            &needs_change,
            observed,
            &mut unverifiable_confirmed,
            &mut pending,
        ),
        CpfpFundingShape::PerNode => {}
    }

    flag_unverifiable_confirmation_branches(
        node_map,
        plan,
        &unverifiable_confirmed,
        &mut unverified,
    );

    // Runs per leaf independently of the walk; an adopted refund overrides it.
    for (leaf_id, address) in &prepared.leaf_refund_addresses {
        interpret_refund(
            leaf_id,
            address,
            observed,
            &mut refunds,
            &mut unverified,
            &mut pending,
        );
    }

    // Drop supplied inputs a prior run's CPFP child already spent. Only a confirmed
    // spend counts: an unconfirmed spender is our own replaceable child (rebuilt via
    // RBF on resume). Gated to tracked-change branches; skipped under a fan-out.
    let mut spent_funding: HashSet<OutPoint> = HashSet::new();
    if plan.fan_out_psbt.is_none() {
        for (leaf_id, funding) in &plan.per_branch_funding {
            if !branch_has_tracked_change(node_map, leaf_id, &nodes) {
                continue;
            }
            for input in funding {
                let query = ChainQuery::Outspend(input.outpoint);
                match observed.get(&query) {
                    Some(ChainResult::Spend(Some(info))) if info.confirmed => {
                        spent_funding.insert(input.outpoint);
                    }
                    None => pending.push(query),
                    _ => {}
                }
            }
        }
    }

    // A per-node UTXO sits on the caller's own funding script for weeks and looks
    // like any other coin to their wallet. One assigned to a still driven step and
    // spent by anything but that step's own child cannot fund it any more, and the
    // step's child cannot be resized around it, so the exit refuses and names the
    // outpoint. A confirmed spend by the step's own child means the step confirmed,
    // so the check waits for the walk to finish classifying: run a round early, a
    // step the walk has not reached yet would read as driven and its own child's
    // spend as a conflict. A step the walk could not verify is left alone too,
    // and so is everything below it, which the walk never reached.
    if plan.funding_shape == CpfpFundingShape::PerNode
        && pending.is_empty()
        && let Some(spent) = per_node_funding_conflict(
            node_map,
            plan,
            &resolved_fan_out_outpoints(plan, fan_out.as_ref()),
            &nodes,
            &refunds,
            &stopped,
            &mut unverified,
            observed,
            &mut pending,
        )
    {
        return Err(SparkWalletError::ServiceError(
            ServiceError::FundingUtxoConflict {
                txid: spent.txid.to_string(),
                vout: spent.vout,
            },
        ));
    }

    trace!(
        fan_out_resolved = fan_out.is_some(),
        resolved_nodes = nodes.len(),
        resolved_refunds = refunds.len(),
        pending = pending.len(),
        unverified = unverified.len(),
        fan_out_unverified,
        "interpret_chain: exit state"
    );
    Ok(ChainInterpretation {
        resolved: ResolvedExitState {
            fan_out,
            nodes,
            refunds,
            stopped,
            spent_funding,
        },
        pending,
        unverified,
        fan_out_unverified,
    })
}

/// Maps each node to the funding script of the first branch (plan order) that
/// reaches it — the branch that drives it — i.e. the script its CPFP change pays.
fn node_funding_scripts(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    plan: &UnilateralExitPlan,
) -> HashMap<TreeNodeId, ScriptBuf> {
    let mut map: HashMap<TreeNodeId, ScriptBuf> = HashMap::new();
    for (leaf_id, funding) in &plan.per_branch_funding {
        let Some(f0) = funding.first() else {
            continue;
        };
        let Some(leaf) = node_map.get(leaf_id) else {
            continue;
        };
        let Ok(chain) = walk_unilateral_exit_chain(node_map, leaf) else {
            continue;
        };
        for node in chain {
            map.entry(node.id.clone())
                .or_insert_with(|| f0.witness_utxo.script_pubkey.clone());
        }
    }
    map
}

/// Whether `leaf_id`'s chain has a node with tracked CPFP change
/// (`ConfirmedCpfp { change: Some }`) — only then are supplied inputs checked.
fn branch_has_tracked_change(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    leaf_id: &TreeNodeId,
    nodes: &HashMap<TreeNodeId, NodeState>,
) -> bool {
    let Some(leaf) = node_map.get(leaf_id) else {
        return false;
    };
    let Ok(chain) = walk_unilateral_exit_chain(node_map, leaf) else {
        return false;
    };
    chain.iter().any(|n| {
        matches!(
            nodes.get(&n.id),
            Some(NodeState::ConfirmedCpfp { change: Some(_) })
        )
    })
}

/// Marks a branch's driven txs unverified when one of its nodes is confirmed but
/// its on-chain spend can't be verified: either confirmed via the operator-OnChain
/// fallback (the node's own chain lookup was unavailable), or chain-confirmed with
/// its CPFP child's spend or body lookup unavailable, so its change is unresolved.
/// In both cases the confirmed child's spend is invisible to `spent_funding`, so a
/// re-supplied input the child already spent wouldn't be dropped and the next driven
/// child would double-spend; flagging (not the `Unconfirmed` the build would
/// otherwise emit) tells the caller not to broadcast until a later run confirms it on
/// a healthy chain. Chain-verified confirmations with a resolved (or safely absent)
/// change are left alone. Only `Unconfirmed` txs are upgraded, so confirmed nodes
/// keep their state. Under per-node funding no child reuses another's input, but
/// the flag is still the only sign the confirmation below was a guess.
fn flag_unverifiable_confirmation_branches(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    plan: &UnilateralExitPlan,
    unverifiable_confirmed: &HashSet<TreeNodeId>,
    unverified: &mut HashSet<TreeNodeId>,
) {
    for (leaf_id, _) in &plan.per_branch_funding {
        let Some(leaf) = node_map.get(leaf_id) else {
            continue;
        };
        let Ok(chain) = walk_unilateral_exit_chain(node_map, leaf) else {
            continue;
        };
        if chain.iter().any(|n| unverifiable_confirmed.contains(&n.id)) {
            for n in &chain {
                unverified.insert(n.id.clone());
            }
        }
    }
}

/// Resolves each `needs_change` node's on-chain CPFP-child change (the output
/// paying its funding script), driving the two lookups through `pending`. An absent
/// lookup leaves the change `None` to retry; an `Unavailable` one adds the node to
/// `unverifiable_confirmed` so its branch is flagged unverified: the confirmed child
/// already spent the supplied input, which a rebuild must not reuse until the change
/// can be verified on a healthy chain.
fn resolve_confirmed_changes(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    plan: &UnilateralExitPlan,
    nodes: &mut HashMap<TreeNodeId, NodeState>,
    needs_change: &HashSet<TreeNodeId>,
    observed: &ObservedIndex<'_>,
    unverifiable_confirmed: &mut HashSet<TreeNodeId>,
    pending: &mut Vec<ChainQuery>,
) {
    let scripts = node_funding_scripts(node_map, plan);
    for node_id in needs_change {
        let Some(NodeState::ConfirmedCpfp { change }) = nodes.get_mut(node_id) else {
            continue;
        };
        if change.is_some() {
            continue;
        }
        let Some(node) = node_map.get(node_id) else {
            continue;
        };
        // The CPFP child spends the node_tx's ephemeral anchor.
        let Some(anchor_vout) = node
            .node_tx
            .output
            .iter()
            .position(is_ephemeral_anchor_output)
            .and_then(|v| u32::try_from(v).ok())
        else {
            continue;
        };
        let anchor_outpoint = OutPoint {
            txid: node.node_tx.compute_txid(),
            vout: anchor_vout,
        };
        let spend_query = ChainQuery::Outspend(anchor_outpoint);
        let Some(spend) = observed.get(&spend_query) else {
            pending.push(spend_query);
            continue;
        };
        let child_txid = match spend {
            ChainResult::Spend(Some(info)) if info.confirmed => info.spender_txid,
            // The node is confirmed, so a CPFP child may already have spent the
            // supplied funding, but the anchor's spend can't be verified. Flag the
            // branch so a rebuild that reuses the input isn't broadcast.
            ChainResult::Unavailable => {
                unverifiable_confirmed.insert(node_id.clone());
                continue;
            }
            _ => continue,
        };
        let tx_query = ChainQuery::Transaction(child_txid);
        let Some(tx_result) = observed.get(&tx_query) else {
            pending.push(tx_query);
            continue;
        };
        let child_tx = match tx_result {
            ChainResult::Transaction(child_tx) => child_tx,
            // The confirmed child's body is unavailable, so its change output can't
            // be resolved; flag the branch (a rebuild would reuse the input the
            // child already spent).
            ChainResult::Unavailable => {
                unverifiable_confirmed.insert(node_id.clone());
                continue;
            }
            _ => continue,
        };
        let Some(script) = scripts.get(node_id) else {
            continue;
        };
        if let Some((vout, out)) = child_tx
            .output
            .iter()
            .enumerate()
            .find(|(_, o)| &o.script_pubkey == script)
            && let Ok(vout) = u32::try_from(vout)
        {
            *change = Some(ConfirmedOutput {
                outpoint: OutPoint {
                    txid: child_txid,
                    vout,
                },
                value: out.value.to_sat(),
            });
        }
    }
}

/// Resolves the fan-out. A confirmed fan-out is recognized structurally (one
/// output per branch to the funding script), not by txid, so a prior fan-out at
/// any fee rate is adopted; a differently-shaped spender is a `FundingUtxoConflict`.
fn interpret_fan_out(
    plan: &UnilateralExitPlan,
    observed: &ObservedIndex<'_>,
    pending: &mut Vec<ChainQuery>,
) -> Result<(Option<ConfirmedFanOut>, bool), SparkWalletError> {
    let Some(fan_out_psbt) = &plan.fan_out_psbt else {
        return Ok((None, false));
    };
    let Some(funding_outpoint) = fan_out_psbt
        .unsigned_tx
        .input
        .first()
        .map(|i| i.previous_output)
    else {
        return Ok((None, true));
    };
    let Some(funding_script) = fan_out_psbt
        .inputs
        .first()
        .and_then(|i| i.witness_utxo.as_ref())
        .map(|o| o.script_pubkey.clone())
    else {
        return Ok((None, true));
    };
    let conflict = || {
        SparkWalletError::ServiceError(ServiceError::FundingUtxoConflict {
            txid: funding_outpoint.txid.to_string(),
            vout: funding_outpoint.vout,
        })
    };

    let spend_query = ChainQuery::Outspend(funding_outpoint);
    let Some(result) = observed.get(&spend_query) else {
        pending.push(spend_query);
        return Ok((None, false));
    };
    let spender = match result {
        ChainResult::Unavailable => return Ok((None, true)),
        ChainResult::Spend(Some(info)) if info.confirmed => info.spender_txid,
        // Unspent, or spent only by an unconfirmed tx: no fan-out to adopt yet.
        _ => return Ok((None, false)),
    };

    let tx_query = ChainQuery::Transaction(spender);
    let Some(result) = observed.get(&tx_query) else {
        pending.push(tx_query);
        return Ok((None, false));
    };
    let tx = match result {
        ChainResult::Transaction(tx) => tx.clone(),
        ChainResult::Unavailable => return Ok((None, true)),
        _ => return Ok((None, false)),
    };
    // A fan-out pays the funding script once per branch, or once per fee-bumped
    // transaction under per-node funding, in plan order. A spender paying it fewer
    // is not a fan-out of this plan at any fee rate.
    let paid_to_funding: Vec<ConfirmedOutput> = tx
        .output
        .iter()
        .enumerate()
        .filter(|(_, o)| o.script_pubkey == funding_script)
        .filter_map(|(vout, o)| {
            u32::try_from(vout).ok().map(|vout| ConfirmedOutput {
                outpoint: OutPoint {
                    txid: spender,
                    vout,
                },
                value: o.value.to_sat(),
            })
        })
        .collect();
    // Per-branch funding takes the first output per branch from a wider one too.
    // Per-node funding rejects a wider one: its outputs are matched to
    // transactions by position, so adopting a fan-out built for a longer list would
    // slide every transaction onto the output before it. Refusing asks the caller
    // for fresh funding, the same recovery a fan-out too small for a higher fee
    // rate already asks for.
    let planned_outputs = fan_out_psbt.unsigned_tx.output.len();
    let fits = match plan.funding_shape {
        CpfpFundingShape::PerBranch => paid_to_funding.len() >= planned_outputs,
        CpfpFundingShape::PerNode => paid_to_funding.len() == planned_outputs,
    };
    if !fits {
        return Err(conflict());
    }
    // Each branch takes as many as it has funding inputs: one under per-branch
    // funding, one per fee-bumped transaction under per-node funding.
    let mut adopted = paid_to_funding.into_iter();
    let mut branch_outputs: HashMap<TreeNodeId, Vec<ConfirmedOutput>> =
        HashMap::with_capacity(plan.per_branch_funding.len());
    for (leaf_id, funding) in &plan.per_branch_funding {
        branch_outputs.insert(
            leaf_id.clone(),
            adopted.by_ref().take(funding.len()).collect(),
        );
    }
    // A branch named twice would leave both sharing one entry, and so spending one
    // fan-out output twice.
    if branch_outputs.len() < plan.per_branch_funding.len() {
        return Err(conflict());
    }
    Ok((Some(ConfirmedFanOut { tx, branch_outputs }), false))
}

/// Whether the build would still fee-bump `target`: a node the chain has not
/// resolved, or a refund not adopted, swept or driven direct.
fn step_driven(
    target: &UnilateralExitNodeFunding,
    leaf_id: &TreeNodeId,
    nodes: &HashMap<TreeNodeId, NodeState>,
    refunds: &HashMap<TreeNodeId, RefundState>,
) -> bool {
    if target.refund {
        !refunds.contains_key(leaf_id)
    } else {
        !nodes.contains_key(&target.node_id)
    }
}

/// The real outpoints funding a per-node exit: the supplied UTXOs, or the
/// adopted fan-out's outputs once one confirmed. Empty while a fresh fan-out is
/// still to broadcast, when nothing on-chain backs the funding yet.
fn resolved_fan_out_outpoints(
    plan: &UnilateralExitPlan,
    fan_out: Option<&ConfirmedFanOut>,
) -> HashMap<TreeNodeId, Vec<OutPoint>> {
    match (&plan.fan_out_psbt, fan_out) {
        (None, _) => plan
            .per_branch_funding
            .iter()
            .map(|(id, funding)| (id.clone(), funding.iter().map(|f| f.outpoint).collect()))
            .collect(),
        (Some(_), Some(confirmed)) => confirmed
            .branch_outputs
            .iter()
            .map(|(id, outputs)| (id.clone(), outputs.iter().map(|o| o.outpoint).collect()))
            .collect(),
        (Some(_), None) => HashMap::new(),
    }
}

/// The first per-node funding outpoint confirmed spent away from a step the
/// build would still drive, driving the lookups through `pending`. An
/// unconfirmed spender is the step's own replaceable child. A lookup that fails
/// leaves the step unverified: its child is still built, but flagged, since the
/// UTXO it spends may be gone. A branch the walk stopped in at a node it could
/// not verify is not judged below that node: the walk classified nothing there,
/// so a step whose own child confirmed would read as driven.
#[allow(clippy::too_many_arguments)]
fn per_node_funding_conflict(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    plan: &UnilateralExitPlan,
    outpoints: &HashMap<TreeNodeId, Vec<OutPoint>>,
    nodes: &HashMap<TreeNodeId, NodeState>,
    refunds: &HashMap<TreeNodeId, RefundState>,
    stopped: &HashSet<TreeNodeId>,
    unverified: &mut HashSet<TreeNodeId>,
    observed: &ObservedIndex<'_>,
    pending: &mut Vec<ChainQuery>,
) -> Option<OutPoint> {
    // The walk's own marks, before a failed funding lookup here adds to them: one
    // on a shared ancestor must not silence the other branches below it.
    let walk_unverified = unverified.clone();
    for (leaf_id, named) in &plan.per_node_funding {
        if stopped.contains(leaf_id) {
            continue;
        }
        let Some(branch_outpoints) = outpoints.get(leaf_id) else {
            continue;
        };
        let walk_stopped = node_map
            .get(leaf_id)
            .and_then(|leaf| walk_unilateral_exit_chain(node_map, leaf).ok())
            .and_then(|chain| chain.into_iter().find(|n| !nodes.contains_key(&n.id)))
            .is_some_and(|first_driven| walk_unverified.contains(&first_driven.id));
        if walk_stopped {
            continue;
        }
        for (target, outpoint) in named.iter().zip(branch_outpoints) {
            let step_id = if target.refund {
                leaf_id
            } else {
                &target.node_id
            };
            if unverified.contains(step_id) || !step_driven(target, leaf_id, nodes, refunds) {
                continue;
            }
            let query = ChainQuery::Outspend(*outpoint);
            match observed.get(&query) {
                Some(ChainResult::Spend(Some(info))) if info.confirmed => {
                    return Some(*outpoint);
                }
                Some(ChainResult::Unavailable) => {
                    unverified.insert(step_id.clone());
                }
                None => pending.push(query),
                _ => {}
            }
        }
    }
    None
}

/// Follows the confirmed spender from the deposit down one branch, classifying
/// each node into `nodes`/`refunds`. Stops (emitting the next lookup into
/// `pending`) at the first output whose spender is not yet observed.
#[allow(clippy::too_many_arguments)]
fn walk_branch(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    leaf_id: &TreeNodeId,
    observed: &ObservedIndex<'_>,
    nodes: &mut HashMap<TreeNodeId, NodeState>,
    refunds: &mut HashMap<TreeNodeId, RefundState>,
    stopped: &mut HashSet<TreeNodeId>,
    // Confirmed cpfp nodes whose CPFP change is resolved afterwards.
    needs_change: &mut HashSet<TreeNodeId>,
    unverified: &mut HashSet<TreeNodeId>,
    // Confirmed nodes whose on-chain spend `spent_funding` can't see (here: the
    // operator-OnChain fallback, when the chain lookup was unavailable).
    unverifiable_confirmed: &mut HashSet<TreeNodeId>,
    pending: &mut Vec<ChainQuery>,
) {
    let Some(leaf) = node_map.get(leaf_id) else {
        return;
    };
    let Ok(chain_nodes) = walk_unilateral_exit_chain(node_map, leaf) else {
        return;
    };
    let Some(root) = chain_nodes.first() else {
        return;
    };
    let Some(deposit_outpoint) = root.node_tx.input.first().map(|i| i.previous_output) else {
        return;
    };

    // Confirmed parent's node_tx txid; `None` at the root (spends the deposit).
    let mut prev_confirmed_txid: Option<Txid> = None;
    let mut prev_confirmed_id: Option<TreeNodeId> = None;
    for node in &chain_nodes {
        let is_leaf = &node.id == leaf_id;
        let live_outpoint = match prev_confirmed_txid {
            Some(txid) => OutPoint {
                txid,
                vout: node.vout,
            },
            None => deposit_outpoint,
        };
        let query = ChainQuery::Outspend(live_outpoint);
        let Some(result) = observed.get(&query) else {
            trace!(%leaf_id, node = %node.id, %live_outpoint, "walk: awaiting outspend");
            pending.push(query);
            return;
        };
        let info = match result {
            ChainResult::Spend(Some(info)) => *info,
            // Unspent: frontier, driven fresh. Its confirmed parent funds it from
            // its on-chain CPFP change.
            ChainResult::Spend(None) => {
                trace!(%leaf_id, node = %node.id, "walk: frontier reached (output unspent), driving fresh");
                if let Some(id) = prev_confirmed_id {
                    needs_change.insert(id);
                }
                return;
            }
            ChainResult::Unavailable => {
                // Chain lookup failed. Fall back to operator status: OnChain =>
                // assume cpfp-confirmed (skip the child, continue) so we don't
                // double-spend an already-spent anchor. It can't tell cpfp from a
                // direct/foreign confirmation; a wrong guess surfaces on the next
                // lookup, and any refund is still adopted independently.
                if node.status == TreeNodeStatus::OnChain {
                    warn!(
                        %leaf_id, node = %node.id,
                        "walk: chain lookup failed, operators report OnChain; assuming cpfp-confirmed"
                    );
                    nodes.insert(node.id.clone(), NodeState::ConfirmedCpfp { change: None });
                    unverifiable_confirmed.insert(node.id.clone());
                    if is_leaf {
                        needs_change.insert(node.id.clone());
                        return;
                    }
                    prev_confirmed_txid = Some(node.node_tx.compute_txid());
                    prev_confirmed_id = Some(node.id.clone());
                    continue;
                }
                trace!(%leaf_id, node = %node.id, "walk: lookup unavailable, node unverified");
                unverified.insert(node.id.clone());
                if let Some(id) = prev_confirmed_id {
                    needs_change.insert(id);
                }
                return;
            }
            _ => return,
        };
        let node_txid = node.node_tx.compute_txid();
        let direct_txid = node.direct_tx.as_ref().map(Transaction::compute_txid);

        if info.spender_txid == node_txid {
            // An unconfirmed (mempool) node_tx is the frontier: drive from here so
            // the child is (re)built.
            if !info.confirmed {
                trace!(%leaf_id, node = %node.id, "walk: node_tx in mempool (unconfirmed), frontier");
                if let Some(id) = prev_confirmed_id {
                    needs_change.insert(id);
                }
                return;
            }
            trace!(%leaf_id, node = %node.id, is_leaf, "walk: confirmed via cpfp node_tx");
            nodes.insert(node.id.clone(), NodeState::ConfirmedCpfp { change: None });
            if is_leaf {
                // The refund's CPFP child is funded from this leaf's own change.
                needs_change.insert(node.id.clone());
                return;
            }
            prev_confirmed_txid = Some(node_txid);
            prev_confirmed_id = Some(node.id.clone());
        } else if is_leaf && direct_txid == Some(info.spender_txid) {
            // A leaf is terminal, so its own direct spend is recoverable via the
            // direct refund, if held.
            if node.direct_refund_tx.is_some() {
                trace!(%leaf_id, node = %node.id, "walk: leaf went direct, driving direct refund");
                nodes.insert(node.id.clone(), NodeState::ConfirmedDirect);
                refunds.insert(leaf_id.clone(), RefundState::DriveDirect);
            } else {
                trace!(%leaf_id, node = %node.id, "walk: leaf went direct but no direct refund held; branch stopped");
                stopped.insert(leaf_id.clone());
            }
            return;
        } else {
            // A foreign/renewed tx, or an intermediate's own direct split whose
            // children can't continue (they spend the cpfp output it never makes).
            trace!(%leaf_id, node = %node.id, spender = %info.spender_txid, "walk: cpfp lineage taken by an uncontinuable tx, branch stopped");
            stopped.insert(leaf_id.clone());
            return;
        }
    }
}

/// Resolves a leaf's refund from its address. The address scan returns every
/// output paid to it, spent or not, so its one refund output is found even after
/// a sweep spends it. The refund's own [`ChainQuery::Outspend`] then separates the
/// three post-broadcast states:
///
/// - unspent: [`RefundState::Adopted`], swept by the build,
/// - spent by a confirmed tx: [`RefundState::Swept`], nothing left to do,
/// - spent by an unconfirmed tx: still [`RefundState::Adopted`], so a sweep sitting
///   in the mempool is rebuilt and handed back to rebroadcast rather than dropped.
///
/// No confirmed output means the refund was never broadcast: left unresolved to
/// drive fresh.
fn interpret_refund(
    leaf_id: &TreeNodeId,
    address: &Address,
    observed: &ObservedIndex<'_>,
    refunds: &mut HashMap<TreeNodeId, RefundState>,
    unverified: &mut HashSet<TreeNodeId>,
    pending: &mut Vec<ChainQuery>,
) {
    let scan_query = ChainQuery::RefundAddress {
        leaf_id: leaf_id.clone(),
        address: address.clone(),
    };
    let Some(result) = observed.get(&scan_query) else {
        pending.push(scan_query);
        return;
    };
    let txos = match result {
        ChainResult::AddressUtxos(txos) => txos,
        ChainResult::Unavailable => {
            unverified.insert(leaf_id.clone());
            return;
        }
        _ => return,
    };
    // The refund address receives exactly one output (the landed variant); no
    // confirmed one means the refund is not on-chain yet.
    let Some(txo) = txos.iter().find(|t| t.confirmed) else {
        return;
    };
    let refund_outpoint = OutPoint {
        txid: txo.txid,
        vout: txo.vout,
    };

    let outspend_query = ChainQuery::Outspend(refund_outpoint);
    let Some(spend) = observed.get(&outspend_query) else {
        pending.push(outspend_query);
        return;
    };
    match spend {
        // Spent by a confirmed tx: the sweep landed, nothing to drive or sweep.
        ChainResult::Spend(Some(info)) if info.confirmed => {
            trace!(%leaf_id, txid = %txo.txid, "interpret_chain: refund swept");
            refunds.insert(leaf_id.clone(), RefundState::Swept);
            return;
        }
        ChainResult::Unavailable => {
            unverified.insert(leaf_id.clone());
            return;
        }
        // Unspent, or spent only by an unconfirmed sweep: adopt so the sweep is
        // (re)built.
        _ => {}
    }

    let tx_query = ChainQuery::Transaction(txo.txid);
    let Some(result) = observed.get(&tx_query) else {
        pending.push(tx_query);
        return;
    };
    let tx = match result {
        ChainResult::Transaction(tx) => tx.clone(),
        ChainResult::Unavailable => {
            unverified.insert(leaf_id.clone());
            return;
        }
        _ => return,
    };
    trace!(%leaf_id, txid = %txo.txid, value = txo.value, "interpret_chain: adopting on-chain refund");
    refunds.insert(
        leaf_id.clone(),
        RefundState::Adopted(ConfirmedRefund {
            tx,
            outpoint: refund_outpoint,
            value: txo.value,
        }),
    );
}

/// Builds a complete unilateral exit from a `prepared` quote and the `observed`
/// chain state (drive it with [`next_chain_queries`] first; no observations builds
/// a fresh full exit). Each not-yet-confirmed tx gets an unsigned CPFP child that
/// pays its fee; confirmed nodes and adopted refunds are emitted without one.
pub fn build_unilateral_exit(
    prepared: &PreparedUnilateralExit,
    observed: &[Observation],
    fee_rate_sat_per_kw: u64,
) -> Result<UnilateralExitBuild, SparkWalletError> {
    let interpretation = interpret_chain(prepared, observed)?;
    let mut build = build_exit(
        &prepared.plan,
        &interpretation.resolved,
        fee_rate_sat_per_kw,
    )?;
    flag_unverified_txs(&mut build, &interpretation);
    Ok(build)
}

/// Upgrades `Unconfirmed` to `Unverified` for txs whose chain lookup failed (the
/// build is chain-blind, so this is applied afterward).
fn flag_unverified_txs(build: &mut UnilateralExitBuild, interpretation: &ChainInterpretation) {
    if let Some(fan_out) = &mut build.fan_out
        && interpretation.fan_out_unverified
        && fan_out.status == ExitTxStatus::Unconfirmed
    {
        fan_out.status = ExitTxStatus::Unverified;
    }
    for tx in build.branches.iter_mut().flat_map(|b| b.txs.iter_mut()) {
        if tx.status == ExitTxStatus::Unconfirmed
            && let Some(id) = &tx.node_id
            && interpretation.unverified.contains(id)
        {
            tx.status = ExitTxStatus::Unverified;
        }
    }
}

/// Assembles the unsigned transactions from a `plan` and a `resolved` on-chain
/// state, chain-independently. See [`build_unilateral_exit`].
pub(crate) fn build_exit(
    plan: &UnilateralExitPlan,
    resolved: &ResolvedExitState,
    fee_rate_sat_per_kw: u64,
) -> Result<UnilateralExitBuild, SparkWalletError> {
    let node_map = &plan.tree_nodes;

    let (fan_out, per_branch_funding) = resolve_fan_out_funding(plan, resolved)?;
    let fan_out_txid = fan_out.as_ref().map(|f| f.txid);
    let node_funding = per_node_funding_by_txid(plan, &per_branch_funding)?;

    // A shared ancestor is bumped once, by the first branch that reaches it.
    let mut emitted: HashSet<Txid> = HashSet::new();
    let mut branches = Vec::with_capacity(per_branch_funding.len());
    let mut refund_outputs: Vec<RefundOutput> = Vec::new();
    let mut cpfp_change_inputs: Vec<CpfpChangeInput> = Vec::new();
    let mut cpfp_fee_sat: u64 = 0;

    for (leaf_id, branch_funding) in &per_branch_funding {
        let leaf = node_map.get(leaf_id).ok_or_else(|| {
            SparkWalletError::Generic(format!("Leaf {leaf_id} missing from exit plan"))
        })?;
        let chain = walk_unilateral_exit_chain(node_map, leaf).map_err(|missing| {
            SparkWalletError::Generic(format!(
                "Incomplete ancestor chain for leaf {leaf_id}: parent {missing} missing"
            ))
        })?;

        let stopped = resolved.stopped.contains(leaf_id);
        if stopped {
            warn!(
                %leaf_id,
                "unilateral exit: branch STOPPED. Its cpfp lineage was taken on-chain by a \
                 transaction this SDK cannot continue (a foreign or timelock-renewed tx, or an \
                 intermediate node's own self-fee direct split). The branch drives no \
                 transactions. If a refund surfaces at the leaf's refund address it is still \
                 swept; otherwise these funds are not recoverable via unilateral exit (they \
                 were spent to, or reclaimed by, another owner)."
            );
        }
        let branch_funding_script = branch_funding
            .first()
            .map(|f| f.witness_utxo.script_pubkey.clone());
        let branch_funding_weight = branch_funding.first().map(|f| f.signed_input_weight);
        let usable_supplied: Vec<CpfpInput> = branch_funding
            .iter()
            .filter(|f| !resolved.spent_funding.contains(&f.outpoint))
            .cloned()
            .collect();
        let mut funding = usable_supplied.clone();
        let mut txs: Vec<ExitTx> = Vec::new();
        let mut first_in_branch = true;
        // Tracked so dependencies survive skipped shared ancestors.
        let mut prev_txid: Option<Txid> = None;

        let mut leaf_node_txid: Option<Txid> = None;

        // A stopped branch drives no nodes; only an adopted refund is swept below.
        if !stopped {
            for node in chain {
                let node_state = resolved.nodes.get(&node.id);
                let base_tx = if node_state == Some(&NodeState::ConfirmedDirect) {
                    node.direct_tx.clone().ok_or_else(|| {
                        SparkWalletError::Generic(format!(
                            "Node {} resolved as direct but has no direct_tx",
                            node.id
                        ))
                    })?
                } else {
                    node.node_tx.clone()
                };
                let node_txid = base_tx.compute_txid();
                let parent_txid = prev_txid.replace(node_txid);
                if &node.id == leaf_id {
                    leaf_node_txid = Some(node_txid);
                }

                if emitted.insert(node_txid) {
                    let mut depends_on = Vec::new();
                    if let Some(p) = parent_txid {
                        depends_on.push(p);
                    }
                    // Chained funding reaches the fan-out through the branch's first
                    // child and carries its change from there. Per-node funding
                    // spends a fan-out output at every step, so each driven step
                    // waits on it in its own right.
                    let waits_on_fan_out = match plan.funding_shape {
                        CpfpFundingShape::PerBranch => first_in_branch,
                        CpfpFundingShape::PerNode => node_state.is_none(),
                    };
                    if waits_on_fan_out && let Some(fo) = fan_out_txid {
                        depends_on.push(fo);
                    }

                    let to_sign = match node_state {
                        Some(NodeState::ConfirmedCpfp { change: Some(c) }) => {
                            if plan.funding_shape == CpfpFundingShape::PerBranch
                                && let (Some(script), Some(weight)) =
                                    (&branch_funding_script, branch_funding_weight)
                            {
                                let mut combined = vec![CpfpInput {
                                    outpoint: c.outpoint,
                                    witness_utxo: TxOut {
                                        value: Amount::from_sat(c.value),
                                        script_pubkey: script.clone(),
                                    },
                                    signed_input_weight: weight,
                                }];
                                // Add still-unspent supplied inputs only for directly-
                                // supplied funding (filtering the tracked change to
                                // avoid a duplicate). Under a fan-out the branch's
                                // output was consumed to produce `c`, so keep only `c`.
                                if plan.fan_out_psbt.is_none() {
                                    combined.extend(
                                        usable_supplied
                                            .iter()
                                            .filter(|f| f.outpoint != c.outpoint)
                                            .cloned(),
                                    );
                                }
                                funding = combined;
                            }
                            None
                        }
                        Some(_) => None,
                        None => {
                            let child_funding =
                                child_inputs(plan, &funding, &node_funding, node_txid, &node.id)?;
                            let CpfpChild {
                                psbt,
                                change_input,
                                fee_sat,
                            } = build_cpfp_child(
                                &node.node_tx,
                                &child_funding,
                                fee_rate_sat_per_kw,
                            )?;
                            cpfp_fee_sat = cpfp_fee_sat.saturating_add(fee_sat);
                            // Chained funding hands the child's change to the next
                            // one; a per-node child's change stays where it lands.
                            match plan.funding_shape {
                                CpfpFundingShape::PerBranch => funding = vec![change_input],
                                CpfpFundingShape::PerNode => {}
                            }
                            Some(psbt)
                        }
                    };
                    let status = match node_state {
                        Some(_) => ExitTxStatus::Confirmed,
                        None => ExitTxStatus::Unconfirmed,
                    };
                    txs.push(ExitTx {
                        kind: ExitTxKind::Node,
                        node_id: Some(node.id.clone()),
                        txid: node_txid,
                        csv_timelock_blocks: csv_timelock(&base_tx),
                        base_tx,
                        to_sign,
                        depends_on,
                        status,
                    });
                    first_in_branch = false;
                }
            }
        }

        // Resolved independently of the node walk, so an on-chain refund is
        // adopted even on a stopped branch.
        match resolved.refunds.get(leaf_id) {
            Some(RefundState::Adopted(adopted)) => {
                refund_outputs.push(RefundOutput {
                    outpoint: adopted.outpoint,
                    leaf_id: leaf_id.clone(),
                    value: adopted.value,
                });
                txs.push(ExitTx {
                    kind: ExitTxKind::Refund,
                    node_id: Some(leaf_id.clone()),
                    txid: adopted.outpoint.txid,
                    csv_timelock_blocks: csv_timelock(&adopted.tx),
                    base_tx: adopted.tx.clone(),
                    to_sign: None,
                    depends_on: vec![],
                    status: ExitTxStatus::Confirmed,
                });
            }
            Some(RefundState::DriveDirect) => {
                let direct_refund = leaf.direct_refund_tx.clone().ok_or_else(|| {
                    SparkWalletError::Generic(format!(
                        "Leaf {leaf_id} went direct but has no direct_refund_tx"
                    ))
                })?;
                let refund_txid = direct_refund.compute_txid();
                let refund_value = refund_output_value(&direct_refund, leaf_id)?;
                let refund_csv = csv_timelock(&direct_refund);
                refund_outputs.push(RefundOutput {
                    outpoint: OutPoint {
                        txid: refund_txid,
                        vout: 0,
                    },
                    leaf_id: leaf_id.clone(),
                    value: refund_value,
                });
                txs.push(ExitTx {
                    kind: ExitTxKind::Refund,
                    node_id: Some(leaf_id.clone()),
                    txid: refund_txid,
                    base_tx: direct_refund,
                    to_sign: None,
                    csv_timelock_blocks: refund_csv,
                    depends_on: leaf_node_txid.into_iter().collect(),
                    status: ExitTxStatus::Unconfirmed,
                });
            }
            Some(RefundState::Swept) => {}
            // Drive the cpfp refund with a fresh child; skipped on a stopped branch.
            None if !stopped => {
                let refund_tx = leaf.refund_tx.clone().ok_or_else(|| {
                    SparkWalletError::Generic(format!(
                        "Leaf {leaf_id} cannot be exited: no refund transaction"
                    ))
                })?;
                let refund_txid = refund_tx.compute_txid();
                let refund_value = refund_output_value(&refund_tx, leaf_id)?;
                let refund_csv = csv_timelock(&refund_tx);
                let refund_funding =
                    child_inputs(plan, &funding, &node_funding, refund_txid, leaf_id)?;
                let child = build_cpfp_child(&refund_tx, &refund_funding, fee_rate_sat_per_kw)?;
                cpfp_fee_sat = cpfp_fee_sat.saturating_add(child.fee_sat);
                refund_outputs.push(RefundOutput {
                    outpoint: OutPoint {
                        txid: refund_txid,
                        vout: 0,
                    },
                    leaf_id: leaf_id.clone(),
                    value: refund_value,
                });
                // Chained funding ends in one change output per branch, which the
                // sweep absorbs. Per-node funding instead leaves every child's change
                // on the caller's own funding script, so the sweep pulls refunds only.
                let mut refund_depends_on: Vec<Txid> = leaf_node_txid.into_iter().collect();
                match plan.funding_shape {
                    CpfpFundingShape::PerBranch => {
                        cpfp_change_inputs.push(CpfpChangeInput {
                            outpoint: child.change_input.outpoint,
                            witness_utxo: child.change_input.witness_utxo.clone(),
                            signed_input_weight: child.change_input.signed_input_weight,
                        });
                    }
                    // The refund child spends a fan-out output of its own.
                    CpfpFundingShape::PerNode => {
                        if let Some(fo) = fan_out_txid {
                            refund_depends_on.push(fo);
                        }
                    }
                }
                txs.push(ExitTx {
                    kind: ExitTxKind::Refund,
                    node_id: Some(leaf_id.clone()),
                    txid: refund_txid,
                    base_tx: refund_tx,
                    to_sign: Some(child.psbt),
                    csv_timelock_blocks: refund_csv,
                    depends_on: refund_depends_on,
                    status: ExitTxStatus::Unconfirmed,
                });
            }
            None => {}
        }

        branches.push(ExitBranch {
            leaf_id: leaf_id.clone(),
            txs,
        });
    }

    let recoverable_value_sat = plan
        .selected_leaves
        .iter()
        .map(|l| l.value)
        .fold(0u64, u64::saturating_add);
    let total_fee_sat = cpfp_fee_sat.saturating_add(fresh_fan_out_fee(plan, fan_out.as_ref()));

    debug!(
        has_fan_out = fan_out.is_some(),
        branches = branches.len(),
        refund_outputs = refund_outputs.len(),
        cpfp_change_inputs = cpfp_change_inputs.len(),
        recoverable_value_sat,
        total_fee_sat,
        "build_unilateral_exit: assembled"
    );
    Ok(UnilateralExitBuild {
        fan_out,
        branches,
        refund_outputs,
        cpfp_change_inputs,
        recoverable_value_sat,
        total_fee_sat,
    })
}

/// The UTXO fee-bumping each transaction, keyed by that transaction's txid.
/// Empty unless the plan funds per node. Keying on the txid rather than on a
/// position keeps a step the chain reports confirmed, which the build skips,
/// from shifting every UTXO after it onto the wrong transaction. Two named
/// transactions sharing a txid are refused: their children would spend one UTXO,
/// conflict, and leave whichever loses unbumpable on every rerun.
fn per_node_funding_by_txid(
    plan: &UnilateralExitPlan,
    per_branch_funding: &[(TreeNodeId, Vec<CpfpInput>)],
) -> Result<HashMap<Txid, CpfpInput>, SparkWalletError> {
    let mut by_txid = HashMap::new();
    if plan.funding_shape != CpfpFundingShape::PerNode {
        return Ok(by_txid);
    }
    let named_by_leaf: HashMap<&TreeNodeId, &[UnilateralExitNodeFunding]> = plan
        .per_node_funding
        .iter()
        .map(|(id, named)| (id, named.as_slice()))
        .collect();
    for (leaf_id, funding) in per_branch_funding {
        let Some(named) = named_by_leaf.get(leaf_id) else {
            continue;
        };
        if named.len() != funding.len() {
            return Err(SparkWalletError::Generic(format!(
                "Branch {leaf_id} names {} transactions but carries {} funding inputs",
                named.len(),
                funding.len()
            )));
        }
        for (target, input) in named.iter().zip(funding) {
            if by_txid.insert(target.txid, input.clone()).is_some() {
                return Err(SparkWalletError::ValidationError(format!(
                    "Two exit transactions share txid {}; the tree cannot be exited per node",
                    target.txid
                )));
            }
        }
    }
    Ok(by_txid)
}

/// The inputs a CPFP child for `txid` spends: the branch's chained funding, or
/// the one UTXO named for that transaction.
fn child_inputs<'a>(
    plan: &UnilateralExitPlan,
    chained: &'a [CpfpInput],
    by_txid: &HashMap<Txid, CpfpInput>,
    txid: Txid,
    node_id: &TreeNodeId,
) -> Result<Cow<'a, [CpfpInput]>, SparkWalletError> {
    match plan.funding_shape {
        CpfpFundingShape::PerBranch => Ok(Cow::Borrowed(chained)),
        CpfpFundingShape::PerNode => {
            Ok(Cow::Owned(vec![node_funding_for(by_txid, txid, node_id)?]))
        }
    }
}

/// The UTXO funding `txid`'s CPFP child. A per-node plan names every transaction
/// its branches could bump, so an absent one means the plan and the tree it is
/// built against have diverged, and the caller has to quote the exit again.
fn node_funding_for(
    by_txid: &HashMap<Txid, CpfpInput>,
    txid: Txid,
    node_id: &TreeNodeId,
) -> Result<CpfpInput, SparkWalletError> {
    by_txid.get(&txid).cloned().ok_or_else(|| {
        SparkWalletError::ValidationError(format!(
            "Node {node_id} still needs fee-bumping but the plan funds no transaction {txid}"
        ))
    })
}

/// The fee a freshly-broadcast fan-out pays (its inputs minus its outputs). Zero
/// when there is no fan-out or it was adopted already-confirmed (fee paid).
fn fresh_fan_out_fee(plan: &UnilateralExitPlan, fan_out: Option<&ExitTx>) -> u64 {
    let (Some(psbt), Some(fan_out)) = (&plan.fan_out_psbt, fan_out) else {
        return 0;
    };
    // A fan-out with nothing to sign was adopted from a confirmed one.
    if fan_out.to_sign.is_none() {
        return 0;
    }
    let in_value: u64 = psbt
        .inputs
        .iter()
        .filter_map(|i| i.witness_utxo.as_ref())
        .map(|o| o.value.to_sat())
        .fold(0u64, u64::saturating_add);
    let out_value: u64 = fan_out
        .base_tx
        .output
        .iter()
        .map(|o| o.value.to_sat())
        .fold(0u64, u64::saturating_add);
    in_value.saturating_sub(out_value)
}

/// The value of a refund tx's swept output (vout 0).
fn refund_output_value(
    refund_tx: &Transaction,
    leaf_id: &TreeNodeId,
) -> Result<u64, SparkWalletError> {
    Ok(refund_tx
        .output
        .first()
        .ok_or_else(|| {
            SparkWalletError::Generic(format!("refund tx for leaf {leaf_id} has no outputs"))
        })?
        .value
        .to_sat())
}

/// The CPFP inputs funding each branch's first child, keyed by leaf id (the
/// shape of [`UnilateralExitPlan::per_branch_funding`]).
type BranchFunding = Vec<(TreeNodeId, Vec<CpfpInput>)>;

/// Resolves the fan-out step and the per-branch funding it feeds. A confirmed
/// fan-out replaces each branch's first input with its real output; a fresh one
/// is returned unsigned to broadcast first; no fan-out assigns funding directly.
fn resolve_fan_out_funding(
    plan: &UnilateralExitPlan,
    resolved: &ResolvedExitState,
) -> Result<(Option<ExitTx>, BranchFunding), SparkWalletError> {
    let Some(fan_out_psbt) = &plan.fan_out_psbt else {
        return Ok((None, plan.per_branch_funding.clone()));
    };

    let Some(confirmed) = &resolved.fan_out else {
        let fan_out = ExitTx {
            kind: ExitTxKind::FanOut,
            node_id: None,
            txid: fan_out_psbt.unsigned_tx.compute_txid(),
            base_tx: fan_out_psbt.unsigned_tx.clone(),
            to_sign: Some(fan_out_psbt.clone()),
            csv_timelock_blocks: None,
            depends_on: vec![],
            status: ExitTxStatus::Unconfirmed,
        };
        return Ok((Some(fan_out), plan.per_branch_funding.clone()));
    };

    // Adopt the confirmed fan-out's real outputs. Each is fixed at the fee it was
    // built with, so it must still cover what the branch spends from it.
    let leaf_by_id: HashMap<&TreeNodeId, _> =
        plan.selected_leaves.iter().map(|l| (&l.id, l)).collect();
    let named_by_leaf: HashMap<&TreeNodeId, &[UnilateralExitNodeFunding]> = plan
        .per_node_funding
        .iter()
        .map(|(id, named)| (id, named.as_slice()))
        .collect();
    let mut per_branch = plan.per_branch_funding.clone();
    for (leaf_id, funding) in &mut per_branch {
        let adopted = confirmed.branch_outputs.get(leaf_id).ok_or_else(|| {
            SparkWalletError::Generic(format!(
                "adopted fan-out is missing an output for branch {leaf_id}"
            ))
        })?;
        match plan.funding_shape {
            CpfpFundingShape::PerBranch => {
                // The fan-out funds each branch with exactly one output.
                let (Some(first), Some(adopted)) = (funding.first_mut(), adopted.first()) else {
                    continue;
                };
                // Dust from the branch's own funding script, not the plan's
                // change_dust_limit.
                let dust = first.witness_utxo.script_pubkey.minimal_non_dust().to_sat();
                // Gate on the physical CPFP floor (cpfp_cost), not the quote's
                // estimated_cost: the sweep is paid from the swept value, not this
                // output, so its sweep-fee headroom must not reject a higher-rate
                // resume the CPFP fees can afford.
                let required = leaf_by_id
                    .get(leaf_id)
                    .map_or(dust, |leaf| leaf.cpfp_cost.saturating_add(dust));
                if adopted.value < required {
                    return Err(SparkWalletError::ServiceError(
                        ServiceError::InsufficientCpfpBudget {
                            required_sat: required,
                        },
                    ));
                }
                first.outpoint = adopted.outpoint;
                first.witness_utxo.value = Amount::from_sat(adopted.value);
                funding.truncate(1);
            }
            CpfpFundingShape::PerNode => {
                // One output per transaction, gated on what that transaction's own
                // CPFP child costs rather than on the branch as a whole. A step the
                // chain already resolved builds no child, so its output, spent or
                // priced for an older rate, is adopted without the gate: holding a
                // dead output to a fresh rate would refuse a resume the still
                // driven steps can afford.
                let named = named_by_leaf.get(leaf_id).copied().unwrap_or_default();
                if adopted.len() != funding.len() || named.len() != funding.len() {
                    return Err(SparkWalletError::Generic(format!(
                        "adopted fan-out gives branch {leaf_id} {} outputs for {} inputs \
                         funding {} transactions",
                        adopted.len(),
                        funding.len(),
                        named.len()
                    )));
                }
                let stopped = resolved.stopped.contains(leaf_id);
                for ((input, adopted), target) in funding.iter_mut().zip(adopted).zip(named) {
                    let driven = !stopped
                        && step_driven(target, leaf_id, &resolved.nodes, &resolved.refunds);
                    if driven && adopted.value < target.funding_sat {
                        return Err(SparkWalletError::ServiceError(
                            ServiceError::InsufficientCpfpBudget {
                                required_sat: target.funding_sat,
                            },
                        ));
                    }
                    input.outpoint = adopted.outpoint;
                    input.witness_utxo.value = Amount::from_sat(adopted.value);
                }
            }
        }
    }

    let fan_out = ExitTx {
        kind: ExitTxKind::FanOut,
        node_id: None,
        txid: confirmed.tx.compute_txid(),
        base_tx: confirmed.tx.clone(),
        to_sign: None,
        csv_timelock_blocks: None,
        depends_on: vec![],
        status: ExitTxStatus::Confirmed,
    };
    Ok((Some(fan_out), per_branch))
}

#[cfg(test)]
fn to_node_map(nodes: Vec<TreeNode>) -> HashMap<TreeNodeId, TreeNode> {
    nodes.into_iter().map(|n| (n.id.clone(), n)).collect()
}

#[cfg(test)]
mod exit_build_tests {
    use super::*;
    use bitcoin::{
        CompressedPublicKey, ScriptBuf, TxOut, Weight,
        absolute::LockTime,
        hashes::Hash,
        key::Secp256k1,
        secp256k1::{PublicKey, SecretKey},
        transaction::Version,
    };
    use spark::{
        Identifier,
        services::{
            UnilateralExitLeafFilter, UnilateralExitSelectedLeaf, compute_cpfp_package_fee,
            plan_unilateral_exit, quote_unilateral_exit,
        },
        tree::{SigningKeyshare, TreeNodeStatus},
    };
    use std::str::FromStr;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const FEE_RATE: u64 = 250;
    const TEST_PUBKEY: &str = "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443";

    fn anchor_tx(nonce: u32) -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::from_height(nonce).unwrap(),
            input: vec![],
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
            }],
        }
    }

    fn node(
        id: &str,
        parent: Option<&str>,
        node_tx: Transaction,
        refund_tx: Option<Transaction>,
    ) -> TreeNode {
        let pk = PublicKey::from_str(TEST_PUBKEY).unwrap();
        TreeNode {
            id: TreeNodeId::from_str(id).unwrap(),
            tree_id: "test".to_string(),
            value: 100_000,
            parent_node_id: parent.map(|p| TreeNodeId::from_str(p).unwrap()),
            node_tx,
            refund_tx,
            direct_tx: None,
            direct_refund_tx: None,
            direct_from_cpfp_refund_tx: None,
            vout: 0,
            verifying_public_key: pk,
            owner_identity_public_key: Some(pk),
            signing_keyshare: SigningKeyshare {
                public_key: pk,
                owner_identifiers: vec![Identifier::try_from(1u16).unwrap()],
                threshold: 2,
            },
            status: TreeNodeStatus::Available,
        }
    }

    fn funding(value: u64) -> CpfpInput {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);
        let script_pubkey =
            Address::p2wpkh(&CompressedPublicKey(pk), bitcoin::Network::Testnet).script_pubkey();
        CpfpInput {
            outpoint: OutPoint {
                txid: Txid::from_byte_array([7u8; 32]),
                vout: 0,
            },
            witness_utxo: TxOut {
                value: Amount::from_sat(value),
                script_pubkey,
            },
            signed_input_weight: 272,
        }
    }

    fn single_leaf_plan() -> UnilateralExitPlan {
        let root = node("root", None, anchor_tx(1), None);
        let leaf = node("leaf", Some("root"), anchor_tx(2), Some(anchor_tx(3)));
        plan_of(root, leaf)
    }

    fn direct_leaf_plan() -> UnilateralExitPlan {
        let root = node("root", None, anchor_tx(1), None);
        let mut leaf = node("leaf", Some("root"), anchor_tx(2), Some(anchor_tx(3)));
        leaf.direct_tx = Some(anchor_tx(4));
        leaf.direct_refund_tx = Some(anchor_tx(5));
        plan_of(root, leaf)
    }

    fn plan_of(root: TreeNode, leaf: TreeNode) -> UnilateralExitPlan {
        UnilateralExitPlan {
            funding_shape: CpfpFundingShape::PerBranch,
            per_node_funding: vec![],
            selected_leaves: vec![UnilateralExitSelectedLeaf {
                id: leaf.id.clone(),
                value: 100_000,
                estimated_cost: 2_000,
                cpfp_cost: 2_000,
            }],
            fan_out_psbt: None,
            per_branch_funding: vec![(leaf.id.clone(), vec![funding(100_000)])],
            tree_nodes: to_node_map(vec![root, leaf]),
        }
    }

    fn id(s: &str) -> TreeNodeId {
        TreeNodeId::from_str(s).unwrap()
    }

    #[test]
    fn build_fresh_drives_node_and_refund() {
        let build =
            build_exit(&single_leaf_plan(), &ResolvedExitState::default(), FEE_RATE).unwrap();

        assert!(
            build.fan_out.is_none(),
            "single-input plan needs no fan-out"
        );
        assert_eq!(build.branches.len(), 1);
        let txs = &build.branches[0].txs;
        assert_eq!(txs.len(), 3);
        assert!(
            txs.iter().all(|t| t.to_sign.is_some()),
            "every driven tx carries a CPFP child to sign"
        );
        let refund = txs.last().unwrap();
        assert_eq!(refund.kind, ExitTxKind::Refund);
        assert_eq!(build.refund_outputs.len(), 1);
        assert_eq!(build.refund_outputs[0].outpoint.txid, refund.txid);
        assert_eq!(build.refund_outputs[0].outpoint.vout, 0);
        assert_eq!(build.cpfp_change_inputs.len(), 1);
    }

    #[test]
    fn build_adopts_confirmed_refund() {
        let adopted_outpoint = OutPoint {
            txid: Txid::from_byte_array([0x42; 32]),
            vout: 0,
        };
        let resolved = ResolvedExitState {
            refunds: [(
                id("leaf"),
                RefundState::Adopted(ConfirmedRefund {
                    tx: anchor_tx(9),
                    outpoint: adopted_outpoint,
                    value: 55_000,
                }),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let build = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        let refund = build.branches[0].txs.last().unwrap();
        assert_eq!(refund.kind, ExitTxKind::Refund);
        assert!(
            refund.to_sign.is_none(),
            "an adopted refund needs no CPFP child"
        );
        assert_eq!(build.refund_outputs.len(), 1);
        assert_eq!(build.refund_outputs[0].outpoint, adopted_outpoint);
        assert_eq!(build.refund_outputs[0].value, 55_000);
        assert!(
            build.cpfp_change_inputs.is_empty(),
            "no refund child was built, so no terminal change feeds the sweep"
        );
    }

    #[test]
    fn build_skips_confirmed_node() {
        let resolved = ResolvedExitState {
            nodes: [(id("root"), NodeState::ConfirmedCpfp { change: None })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let build = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        let txs = &build.branches[0].txs;
        let root = &txs[0];
        assert_eq!(root.node_id.as_ref(), Some(&id("root")));
        assert!(root.to_sign.is_none(), "a confirmed node carries no child");
        assert_eq!(root.status, ExitTxStatus::Confirmed);
        assert!(
            txs.iter().skip(1).all(|t| t.to_sign.is_some()),
            "nodes below the confirmed one are still driven"
        );
    }

    #[test]
    fn build_threads_confirmed_change_into_next_child() {
        let change_outpoint = OutPoint {
            txid: Txid::from_byte_array([0x55; 32]),
            vout: 0,
        };
        let resolved = ResolvedExitState {
            nodes: [(
                id("root"),
                NodeState::ConfirmedCpfp {
                    change: Some(ConfirmedOutput {
                        outpoint: change_outpoint,
                        value: 90_000,
                    }),
                },
            )]
            .into_iter()
            .collect(),
            spent_funding: [funding(100_000).outpoint].into_iter().collect(),
            ..Default::default()
        };
        let build = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        let txs = &build.branches[0].txs;

        assert!(
            txs[0].to_sign.is_none(),
            "the confirmed root carries no child"
        );
        let leaf_child = txs[1]
            .to_sign
            .as_ref()
            .expect("the leaf node below the confirmed root is driven");
        assert!(
            leaf_child
                .unsigned_tx
                .input
                .iter()
                .any(|i| i.previous_output == change_outpoint),
            "the driven child must spend the confirmed node's on-chain change"
        );
        let original_funding = funding(100_000).outpoint;
        assert!(
            !leaf_child
                .unsigned_tx
                .input
                .iter()
                .any(|i| i.previous_output == original_funding),
            "the driven child must not reuse the already-spent original funding UTXO"
        );
    }

    #[test]
    fn build_combines_confirmed_change_with_unspent_supplied() {
        let change_outpoint = OutPoint {
            txid: Txid::from_byte_array([0x55; 32]),
            vout: 0,
        };
        let resolved = ResolvedExitState {
            nodes: [(
                id("root"),
                NodeState::ConfirmedCpfp {
                    change: Some(ConfirmedOutput {
                        outpoint: change_outpoint,
                        value: 90_000,
                    }),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let build = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        let leaf_child = build.branches[0].txs[1]
            .to_sign
            .as_ref()
            .expect("the leaf node below the confirmed root is driven");
        let spends = |o: OutPoint| {
            leaf_child
                .unsigned_tx
                .input
                .iter()
                .any(|i| i.previous_output == o)
        };
        assert!(
            spends(change_outpoint),
            "the driven child spends the tracked on-chain change"
        );
        assert!(
            spends(funding(100_000).outpoint),
            "and additively spends the still-unspent supplied UTXO"
        );
    }

    #[test]
    fn build_fanout_resume_does_not_readd_consumed_output() {
        let root = node("root", None, anchor_tx(1), None);
        let leaf = node("leaf", Some("root"), anchor_tx(2), Some(anchor_tx(3)));
        let branch_output = funding(100_000);
        let fan_out_tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                previous_output: branch_output.outpoint,
                ..Default::default()
            }],
            output: vec![branch_output.witness_utxo.clone()],
        };
        let fan_out_psbt = bitcoin::Psbt::from_unsigned_tx(fan_out_tx).unwrap();
        let plan = UnilateralExitPlan {
            funding_shape: CpfpFundingShape::PerBranch,
            per_node_funding: vec![],
            selected_leaves: vec![UnilateralExitSelectedLeaf {
                id: leaf.id.clone(),
                value: 100_000,
                estimated_cost: 2_000,
                cpfp_cost: 2_000,
            }],
            fan_out_psbt: Some(fan_out_psbt),
            per_branch_funding: vec![(leaf.id.clone(), vec![branch_output.clone()])],
            tree_nodes: to_node_map(vec![root, leaf]),
        };
        let change_outpoint = OutPoint {
            txid: Txid::from_byte_array([0x55; 32]),
            vout: 0,
        };
        let resolved = ResolvedExitState {
            nodes: [(
                id("root"),
                NodeState::ConfirmedCpfp {
                    change: Some(ConfirmedOutput {
                        outpoint: change_outpoint,
                        value: 90_000,
                    }),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let build = build_exit(&plan, &resolved, FEE_RATE).unwrap();
        let leaf_child = build.branches[0].txs[1]
            .to_sign
            .as_ref()
            .expect("the leaf below the confirmed root is driven");
        let spends = |o: OutPoint| {
            leaf_child
                .unsigned_tx
                .input
                .iter()
                .any(|i| i.previous_output == o)
        };
        assert!(spends(change_outpoint), "funds from the tracked change");
        assert!(
            !spends(branch_output.outpoint),
            "must not re-add the already-consumed fan-out output"
        );
    }

    #[test]
    fn build_shared_confirmed_change_is_not_double_spent() {
        let mid_change = OutPoint {
            txid: Txid::from_byte_array([0x66; 32]),
            vout: 0,
        };
        let resolved = ResolvedExitState {
            nodes: [
                (id("root"), NodeState::ConfirmedCpfp { change: None }),
                (
                    id("mid"),
                    NodeState::ConfirmedCpfp {
                        change: Some(ConfirmedOutput {
                            outpoint: mid_change,
                            value: 80_000,
                        }),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let build = build_exit(&shared_ancestor_plan(), &resolved, FEE_RATE).unwrap();

        let spends_mid_change = |branch: &ExitBranch| {
            branch.txs.iter().any(|t| {
                t.to_sign.as_ref().is_some_and(|c| {
                    c.unsigned_tx
                        .input
                        .iter()
                        .any(|i| i.previous_output == mid_change)
                })
            })
        };
        let count = build
            .branches
            .iter()
            .filter(|b| spends_mid_change(b))
            .count();
        assert_eq!(
            count, 1,
            "exactly one branch consumes the shared confirmed change"
        );
    }

    #[test]
    fn build_drives_direct_refund() {
        let resolved = ResolvedExitState {
            nodes: [
                (id("root"), NodeState::ConfirmedCpfp { change: None }),
                (id("leaf"), NodeState::ConfirmedDirect),
            ]
            .into_iter()
            .collect(),
            refunds: [(id("leaf"), RefundState::DriveDirect)]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let build = build_exit(&direct_leaf_plan(), &resolved, FEE_RATE).unwrap();
        let txs = &build.branches[0].txs;
        assert_eq!(txs.len(), 3, "root, leaf (direct), refund (direct)");
        let leaf_tx = &txs[1];
        assert_eq!(leaf_tx.node_id.as_ref(), Some(&id("leaf")));
        assert_eq!(leaf_tx.txid, anchor_tx(4).compute_txid());
        assert!(leaf_tx.to_sign.is_none(), "a direct node pays its own fee");
        let refund = &txs[2];
        assert_eq!(refund.kind, ExitTxKind::Refund);
        assert_eq!(refund.txid, anchor_tx(5).compute_txid());
        assert!(refund.to_sign.is_none(), "a direct refund pays its own fee");
        assert_eq!(build.refund_outputs.len(), 1);
        assert_eq!(build.refund_outputs[0].outpoint.txid, refund.txid);
        assert!(
            build.cpfp_change_inputs.is_empty(),
            "no cpfp child was built, so no terminal change feeds the sweep"
        );
    }

    #[test]
    fn build_emits_nothing_for_stopped_branch() {
        let resolved = ResolvedExitState {
            stopped: [id("leaf")].into_iter().collect(),
            ..Default::default()
        };
        let build = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        assert!(
            build.branches[0].txs.is_empty(),
            "a stopped branch emits no transactions"
        );
        assert!(
            build.refund_outputs.is_empty(),
            "a stopped branch yields no refund to sweep"
        );
        assert!(build.cpfp_change_inputs.is_empty());
    }

    #[test]
    fn build_stopped_branch_still_adopts_surfaced_refund() {
        let adopted_outpoint = OutPoint {
            txid: Txid::from_byte_array([0x43; 32]),
            vout: 0,
        };
        let resolved = ResolvedExitState {
            stopped: [id("leaf")].into_iter().collect(),
            refunds: [(
                id("leaf"),
                RefundState::Adopted(ConfirmedRefund {
                    tx: anchor_tx(9),
                    outpoint: adopted_outpoint,
                    value: 40_000,
                }),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let build = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        assert_eq!(build.refund_outputs.len(), 1);
        assert_eq!(build.refund_outputs[0].outpoint, adopted_outpoint);
    }

    #[test]
    fn build_omits_swept_leaf_refund() {
        let resolved = ResolvedExitState {
            nodes: [
                (id("root"), NodeState::ConfirmedCpfp { change: None }),
                (id("leaf"), NodeState::ConfirmedCpfp { change: None }),
            ]
            .into_iter()
            .collect(),
            refunds: [(id("leaf"), RefundState::Swept)].into_iter().collect(),
            ..Default::default()
        };
        let build = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        assert!(
            build.refund_outputs.is_empty(),
            "a swept leaf yields no refund output to sweep"
        );
        assert!(
            !build.branches[0]
                .txs
                .iter()
                .any(|t| t.kind == ExitTxKind::Refund),
            "a swept leaf emits no refund tx"
        );
        assert!(build.cpfp_change_inputs.is_empty());
    }

    fn shared_ancestor_plan() -> UnilateralExitPlan {
        let root = node("root", None, anchor_tx(1), None);
        let mid = node("mid", Some("root"), anchor_tx(2), None);
        let leaf_a = node("leafA", Some("mid"), anchor_tx(3), Some(anchor_tx(4)));
        let leaf_b = node("leafB", Some("mid"), anchor_tx(5), Some(anchor_tx(6)));
        UnilateralExitPlan {
            funding_shape: CpfpFundingShape::PerBranch,
            per_node_funding: vec![],
            selected_leaves: vec![
                UnilateralExitSelectedLeaf {
                    id: leaf_a.id.clone(),
                    value: 100_000,
                    estimated_cost: 2_000,
                    cpfp_cost: 2_000,
                },
                UnilateralExitSelectedLeaf {
                    id: leaf_b.id.clone(),
                    value: 100_000,
                    estimated_cost: 2_000,
                    cpfp_cost: 2_000,
                },
            ],
            fan_out_psbt: None,
            per_branch_funding: vec![
                (leaf_a.id.clone(), vec![funding(100_000)]),
                (leaf_b.id.clone(), vec![funding(100_000)]),
            ],
            tree_nodes: to_node_map(vec![root, mid, leaf_a, leaf_b]),
        }
    }

    #[test]
    fn build_dedups_shared_ancestors_and_threads_dependencies() {
        let plan = shared_ancestor_plan();
        let mid_txid = anchor_tx(2).compute_txid();
        let build = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE).unwrap();

        assert_eq!(build.branches.len(), 2);
        let all_txs: Vec<&ExitTx> = build.branches.iter().flat_map(|b| b.txs.iter()).collect();

        for shared in ["root", "mid"] {
            let count = all_txs
                .iter()
                .filter(|t| t.node_id.as_ref() == Some(&id(shared)))
                .count();
            assert_eq!(
                count, 1,
                "shared ancestor {shared} must appear exactly once"
            );
        }

        let second = &build.branches[1];
        let leaf_node = second
            .txs
            .iter()
            .find(|t| t.kind == ExitTxKind::Node)
            .expect("the second branch emits its own leaf node");
        assert!(
            leaf_node.depends_on.contains(&mid_txid),
            "the second branch's leaf must depend on the shared ancestor mid"
        );

        assert_eq!(build.refund_outputs.len(), 2);
    }

    /// Two branches sharing root and mid, funded by a single fan-out that pays
    /// each branch one output.
    fn shared_ancestor_plan_with_fan_out() -> UnilateralExitPlan {
        let root = node("root", None, anchor_tx(1), None);
        let mid = node("mid", Some("root"), anchor_tx(2), None);
        let leaf_a = node("leafA", Some("mid"), anchor_tx(3), Some(anchor_tx(4)));
        let leaf_b = node("leafB", Some("mid"), anchor_tx(5), Some(anchor_tx(6)));

        let mut fund_a = funding(100_000);
        fund_a.outpoint.vout = 0;
        let mut fund_b = funding(100_000);
        fund_b.outpoint.vout = 1;

        let fan_out_tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                previous_output: OutPoint {
                    txid: Txid::from_byte_array([0x99; 32]),
                    vout: 0,
                },
                ..Default::default()
            }],
            output: vec![fund_a.witness_utxo.clone(), fund_b.witness_utxo.clone()],
        };
        let fan_out_psbt = bitcoin::Psbt::from_unsigned_tx(fan_out_tx).unwrap();

        UnilateralExitPlan {
            funding_shape: CpfpFundingShape::PerBranch,
            per_node_funding: vec![],
            selected_leaves: vec![
                UnilateralExitSelectedLeaf {
                    id: leaf_a.id.clone(),
                    value: 100_000,
                    estimated_cost: 2_000,
                    cpfp_cost: 2_000,
                },
                UnilateralExitSelectedLeaf {
                    id: leaf_b.id.clone(),
                    value: 100_000,
                    estimated_cost: 2_000,
                    cpfp_cost: 2_000,
                },
            ],
            fan_out_psbt: Some(fan_out_psbt),
            per_branch_funding: vec![
                (leaf_a.id.clone(), vec![fund_a]),
                (leaf_b.id.clone(), vec![fund_b]),
            ],
            tree_nodes: to_node_map(vec![root, mid, leaf_a, leaf_b]),
        }
    }

    #[test]
    fn build_fanout_shared_ancestor_threads_fanout_dependency() {
        let plan = shared_ancestor_plan_with_fan_out();
        let fan_out_txid = plan
            .fan_out_psbt
            .as_ref()
            .unwrap()
            .unsigned_tx
            .compute_txid();
        let build = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE).unwrap();

        // The first branch drives root first, so its root depends on the fan-out.
        let first_root = build.branches[0]
            .txs
            .iter()
            .find(|t| t.node_id.as_ref() == Some(&id("root")))
            .expect("the first branch emits root");
        assert!(
            first_root.depends_on.contains(&fan_out_txid),
            "the first branch's first driven node depends on the fan-out"
        );

        // The second branch shares root and mid (already emitted), so its own leaf
        // is its first driven node. Its CPFP child spends the fan-out's per-branch
        // output, so it must depend on the fan-out too.
        let second_first = build.branches[1]
            .txs
            .iter()
            .find(|t| t.kind == ExitTxKind::Node)
            .expect("the second branch emits its own leaf node");
        assert!(
            second_first.depends_on.contains(&fan_out_txid),
            "the second branch's first driven node must depend on the fan-out"
        );
    }

    #[test]
    fn flag_unverified_preserves_confirmed_downstream_tx() {
        // Both txs are in the unverified set, but only the unconfirmed one is
        // upgraded: a confirmed child on the same branch keeps its status.
        let node_tx = |nonce| ExitTx {
            kind: ExitTxKind::Node,
            node_id: Some(id(if nonce == 1 { "mid" } else { "leaf" })),
            txid: anchor_tx(nonce).compute_txid(),
            base_tx: anchor_tx(nonce),
            to_sign: None,
            csv_timelock_blocks: None,
            depends_on: vec![],
            status: if nonce == 1 {
                ExitTxStatus::Confirmed
            } else {
                ExitTxStatus::Unconfirmed
            },
        };
        let mut build = UnilateralExitBuild {
            fan_out: None,
            branches: vec![ExitBranch {
                leaf_id: id("leaf"),
                txs: vec![node_tx(1), node_tx(2)],
            }],
            refund_outputs: vec![],
            cpfp_change_inputs: vec![],
            recoverable_value_sat: 0,
            total_fee_sat: 0,
        };
        let interpretation = ChainInterpretation {
            resolved: ResolvedExitState::default(),
            pending: vec![],
            unverified: [id("mid"), id("leaf")].into_iter().collect(),
            fan_out_unverified: false,
        };
        flag_unverified_txs(&mut build, &interpretation);

        let txs = &build.branches[0].txs;
        assert_eq!(
            txs[0].status,
            ExitTxStatus::Confirmed,
            "a confirmed downstream tx is not downgraded"
        );
        assert_eq!(
            txs[1].status,
            ExitTxStatus::Unverified,
            "the unconfirmed driven tx is upgraded to unverified"
        );
    }

    fn psbt_fee(psbt: &bitcoin::Psbt) -> u64 {
        let ins: u64 = psbt
            .inputs
            .iter()
            .filter_map(|i| i.witness_utxo.as_ref())
            .map(|o| o.value.to_sat())
            .fold(0u64, u64::saturating_add);
        let outs: u64 = psbt
            .unsigned_tx
            .output
            .iter()
            .map(|o| o.value.to_sat())
            .fold(0u64, u64::saturating_add);
        ins.saturating_sub(outs)
    }

    #[test]
    fn build_total_fee_sums_built_cpfp_children() {
        let build =
            build_exit(&single_leaf_plan(), &ResolvedExitState::default(), FEE_RATE).unwrap();
        assert!(
            build.fan_out.is_none(),
            "single-input plan needs no fan-out"
        );
        let children_fee: u64 = build
            .branches
            .iter()
            .flat_map(|b| b.txs.iter())
            .filter_map(|t| t.to_sign.as_ref())
            .map(psbt_fee)
            .fold(0u64, u64::saturating_add);
        assert!(children_fee > 0);
        assert_eq!(build.total_fee_sat, children_fee);
    }

    #[test]
    fn resume_confirmed_node_lowers_total_fee() {
        let all_driven =
            build_exit(&single_leaf_plan(), &ResolvedExitState::default(), FEE_RATE).unwrap();
        let resolved = ResolvedExitState {
            nodes: [(id("root"), NodeState::ConfirmedCpfp { change: None })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let resumed = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        assert!(
            resumed.total_fee_sat < all_driven.total_fee_sat,
            "a confirmed node is not rebuilt, so the resume pays less \
             ({} vs {})",
            resumed.total_fee_sat,
            all_driven.total_fee_sat
        );
    }

    #[test]
    fn plan_single_leaf_two_utxo_funding_boundary_is_exact() {
        // A single leaf funded with TWO UTXOs: build funds the first CPFP child with
        // both, so its fee is sized on their combined weight. The plan gate must
        // charge that, not the one-input estimate the selection pass uses; otherwise
        // funding in the gap passes the plan then fails build_cpfp_child.
        let root = node("root", None, anchor_tx(1), None);
        let leaf = node("leaf", Some("root"), anchor_tx(2), Some(anchor_tx(3)));
        let leaf_id = leaf.id.clone();
        let nodes: HashMap<TreeNodeId, TreeNode> = [(id("root"), root), (leaf_id.clone(), leaf)]
            .into_iter()
            .collect();

        let change_len = funding(0).witness_utxo.script_pubkey.len();
        let dest_len = change_len;

        // The first child's extra fee for the second input (272 wu more), which the
        // old one-input gate omitted. The first bumped tx is the root's node_tx.
        let two_input = compute_cpfp_package_fee(
            anchor_tx(1).weight(),
            Weight::from_wu(544),
            change_len,
            FEE_RATE,
        );
        let one_input = compute_cpfp_package_fee(
            anchor_tx(1).weight(),
            Weight::from_wu(272),
            change_len,
            FEE_RATE,
        );
        let extra_second_input_fee = two_input - one_input;
        assert!(extra_second_input_fee > 0);

        let two = |total: u64| {
            let mut a = funding(total / 2);
            a.outpoint.vout = 0;
            let mut b = funding(total - total / 2);
            b.outpoint.vout = 1;
            plan_unilateral_exit(
                nodes.clone(),
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::ProfitableOnly,
                vec![a, b],
                CpfpFundingShape::PerBranch,
                FEE_RATE,
                dest_len,
            )
        };
        let one = |total: u64| {
            let mut only = funding(total);
            only.outpoint.vout = 0;
            plan_unilateral_exit(
                nodes.clone(),
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::ProfitableOnly,
                vec![only],
                CpfpFundingShape::PerBranch,
                FEE_RATE,
                dest_len,
            )
        };

        // The gate reports its exact floor when funding is zero.
        let floor_two = match two(0) {
            Err(ServiceError::InsufficientCpfpBudget { required_sat }) => required_sat,
            other => panic!("expected InsufficientCpfpBudget, got {other:?}"),
        };
        let floor_one = match one(0) {
            Err(ServiceError::InsufficientCpfpBudget { required_sat }) => required_sat,
            other => panic!("expected InsufficientCpfpBudget, got {other:?}"),
        };
        // The two-UTXO floor is exactly the one-UTXO floor plus the first child's
        // second-input fee: the window the old one-input gate under-charged.
        assert_eq!(floor_two, floor_one + extra_second_input_fee);

        // Exactly the two-UTXO floor both plans AND builds: no plan/build mismatch.
        let plan = two(floor_two).expect("funding at the two-UTXO floor plans");
        assert!(plan.fan_out_psbt.is_none());
        assert_eq!(plan.per_branch_funding.len(), 1);
        let build = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE)
            .expect("funding at the plan floor also builds");
        assert_eq!(build.cpfp_change_inputs.len(), 1);

        // One sat under rejects up front with that exact floor.
        match two(floor_two - 1) {
            Err(ServiceError::InsufficientCpfpBudget { required_sat }) => {
                assert_eq!(required_sat, floor_two);
            }
            other => panic!("expected InsufficientCpfpBudget, got {other:?}"),
        }
        // Funding the old one-input gate would have accepted is now rejected.
        assert!(two(floor_one).is_err());
    }

    #[test]
    fn plan_two_branch_multi_input_builds() {
        // Two independent leaves, each funded with two UTXOs: the assignment lands
        // two inputs per branch and the build funds each branch's first CPFP child
        // with both. Proves the multi-branch arm handles >1 input per branch.
        let leaf_a = node("leafA", None, anchor_tx(1), Some(anchor_tx(2)));
        let leaf_b = node("leafB", None, anchor_tx(3), Some(anchor_tx(4)));
        let a_id = leaf_a.id.clone();
        let b_id = leaf_b.id.clone();
        let nodes: HashMap<TreeNodeId, TreeNode> = [(a_id.clone(), leaf_a), (b_id.clone(), leaf_b)]
            .into_iter()
            .collect();

        let change_len = funding(0).witness_utxo.script_pubkey.len();
        let dust = funding(0)
            .witness_utxo
            .script_pubkey
            .minimal_non_dust()
            .to_sat();

        // Each identical branch's one-UTXO requirement, split across two inputs so
        // the assignment lands two per branch.
        let quote = quote_unilateral_exit(
            &nodes,
            &[a_id.clone(), b_id.clone()],
            UnilateralExitLeafFilter::ProfitableOnly,
            272,
            change_len,
            dust,
            CpfpFundingShape::PerBranch,
            FEE_RATE,
            change_len,
        )
        .unwrap();
        let half = quote.per_branch_funding[0].1 / 2 + 1;

        let inputs: Vec<CpfpInput> = (0..4u32)
            .map(|vout| {
                let mut f = funding(half);
                f.outpoint.vout = vout;
                f
            })
            .collect();
        let plan = plan_unilateral_exit(
            nodes,
            &[a_id, b_id],
            UnilateralExitLeafFilter::ProfitableOnly,
            inputs,
            CpfpFundingShape::PerBranch,
            FEE_RATE,
            change_len,
        )
        .unwrap();
        assert!(
            plan.fan_out_psbt.is_none(),
            "four inputs partition two-per-branch without a fan-out"
        );
        assert_eq!(plan.per_branch_funding.len(), 2);
        assert!(
            plan.per_branch_funding
                .iter()
                .all(|(_, ins)| ins.len() == 2),
            "each branch is funded with two inputs"
        );
        let build = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE).unwrap();
        assert_eq!(build.branches.len(), 2);
    }

    /// One funding UTXO per transaction, told apart by vout.
    fn per_node_inputs(count: u32, value: u64) -> Vec<CpfpInput> {
        (0..count)
            .map(|vout| {
                let mut input = funding(value);
                input.outpoint.vout = vout;
                input
            })
            .collect()
    }

    /// A root and its leaf, so the branch fee-bumps three transactions: the root's
    /// node tx, the leaf's node tx, and the leaf's refund.
    /// Two leaves under one shared root, so the branches fee-bump an unequal
    /// number of transactions (3 and 2) and the fan-out has to be split between
    /// them in plan order.
    fn per_node_two_branch_plan(
        inputs: Vec<CpfpInput>,
    ) -> Result<UnilateralExitPlan, ServiceError> {
        let root = node("root", None, anchor_tx(1), None);
        let leaf_a = node("leafA", Some("root"), anchor_tx(2), Some(anchor_tx(3)));
        let leaf_b = node("leafB", Some("root"), anchor_tx(4), Some(anchor_tx(5)));
        plan_unilateral_exit(
            to_node_map(vec![root, leaf_a, leaf_b]),
            &[id("leafA"), id("leafB")],
            UnilateralExitLeafFilter::All,
            inputs,
            CpfpFundingShape::PerNode,
            FEE_RATE,
            22,
        )
    }

    fn per_node_plan(
        inputs: Vec<CpfpInput>,
        root_status: TreeNodeStatus,
    ) -> Result<UnilateralExitPlan, ServiceError> {
        let mut root = node("root", None, anchor_tx(1), None);
        root.status = root_status;
        let leaf = node("leaf", Some("root"), anchor_tx(2), Some(anchor_tx(3)));
        plan_unilateral_exit(
            to_node_map(vec![root, leaf]),
            &[id("leaf")],
            UnilateralExitLeafFilter::All,
            inputs,
            CpfpFundingShape::PerNode,
            FEE_RATE,
            22,
        )
    }

    /// The outpoints each built CPFP child spends, child by child.
    fn child_inputs(branch: &ExitBranch) -> Vec<Vec<OutPoint>> {
        branch
            .txs
            .iter()
            .filter_map(|tx| tx.to_sign.as_ref())
            .map(|psbt| {
                psbt.unsigned_tx
                    .input
                    .iter()
                    .map(|i| i.previous_output)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn build_per_node_funds_each_child_from_its_own_utxo() {
        // The point of funding per node: no child spends another child's change, so
        // a child's inputs are settled before the rate any other child is built at.
        let inputs = per_node_inputs(3, 5_000);
        let plan = per_node_plan(inputs.clone(), TreeNodeStatus::Available).unwrap();
        assert!(plan.fan_out_psbt.is_none(), "an exact set needs no fan-out");

        let build = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE).unwrap();
        let branch = &build.branches[0];
        let spent = child_inputs(branch);
        assert_eq!(spent.len(), 3, "root, leaf and refund each get a child");
        assert_eq!(
            spent.iter().map(|i| i[0]).collect::<Vec<_>>(),
            inputs.iter().map(|i| i.outpoint).collect::<Vec<_>>(),
            "each child spends the UTXO supplied for its own transaction"
        );

        let children: HashSet<Txid> = branch
            .txs
            .iter()
            .filter_map(|tx| tx.to_sign.as_ref())
            .map(|psbt| psbt.unsigned_tx.compute_txid())
            .collect();
        assert!(
            spent
                .iter()
                .flatten()
                .all(|outpoint| !children.contains(&outpoint.txid)),
            "no child spends another child's output"
        );
    }

    #[test]
    fn build_per_node_leaves_every_change_off_the_sweep() {
        // Each child writes its own change to the caller's funding script. Folding
        // them all in would grow the sweep by one input per transaction, to recover
        // outputs that are already spendable where they sit.
        let plan = per_node_plan(per_node_inputs(3, 5_000), TreeNodeStatus::Available).unwrap();
        let build = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE).unwrap();

        assert_eq!(build.refund_outputs.len(), 1);
        assert!(
            build.cpfp_change_inputs.is_empty(),
            "per-node change stays on the funding script"
        );
    }

    #[test]
    fn build_per_node_keeps_a_confirmed_node_from_shifting_the_rest() {
        // Funding is keyed by the transaction it pays for, not by position. A node
        // the chain reports confirmed drops out of the build, and the UTXOs after it
        // must stay on their own transactions rather than sliding up one.
        let inputs = per_node_inputs(3, 5_000);
        let plan = per_node_plan(inputs.clone(), TreeNodeStatus::Available).unwrap();
        let resolved = ResolvedExitState {
            nodes: [(id("root"), NodeState::ConfirmedCpfp { change: None })]
                .into_iter()
                .collect(),
            ..Default::default()
        };

        let build = build_exit(&plan, &resolved, FEE_RATE).unwrap();
        let spent = child_inputs(&build.branches[0]);
        assert_eq!(spent.len(), 2, "the confirmed root is not bumped again");
        assert_eq!(
            spent.iter().map(|i| i[0]).collect::<Vec<_>>(),
            vec![inputs[1].outpoint, inputs[2].outpoint],
            "the leaf and its refund keep their own UTXOs"
        );
    }

    #[test]
    fn build_per_node_names_a_transaction_it_has_no_funding_for() {
        // A plan that funds fewer transactions than the branch bumps has to say which
        // one it left out. Quietly reusing another transaction's UTXO would build two
        // children spending it, and neither the caller nor the chain would say why.
        let mut plan = per_node_plan(per_node_inputs(3, 5_000), TreeNodeStatus::Available).unwrap();
        assert_eq!(plan.per_node_funding[0].1.len(), 3);
        plan.per_node_funding[0].1.remove(0);
        plan.per_branch_funding[0].1.remove(0);

        let err = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE)
            .expect_err("the root still needs a child");
        assert!(
            matches!(err, SparkWalletError::ValidationError(ref m) if m.contains("root")),
            "expected the unfunded node to be named, got: {err:?}"
        );
    }

    #[test]
    fn build_per_node_refuses_a_branch_whose_inputs_do_not_match_its_list() {
        // The named transactions and the inputs are paired by position, so a plan
        // carrying fewer inputs than it names cannot say which transaction each one
        // pays for. Pairing what is there would leave the last transaction unfunded
        // with nothing to say why, so the mismatch itself is the error.
        let mut plan = per_node_plan(per_node_inputs(3, 5_000), TreeNodeStatus::Available).unwrap();
        plan.per_branch_funding[0].1.pop();

        let err = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE)
            .expect_err("three transactions, two inputs");
        assert!(
            matches!(err, SparkWalletError::Generic(ref m) if m.contains("names 3") && m.contains("2 funding")),
            "expected the mismatch to be named, got: {err:?}"
        );
    }

    #[test]
    fn build_per_node_waits_on_the_fan_out_at_every_driven_step() {
        // Each child spends a fan-out output of its own, so each one has to wait for
        // the fan-out to confirm. Carrying that dependency only on the branch's first
        // transaction would hand the caller a package they can broadcast as soon as
        // its parent confirms, spending an output that does not exist yet.
        let plan = per_node_plan(vec![funding(100_000)], TreeNodeStatus::Available).unwrap();
        let fan_out_txid = plan
            .fan_out_psbt
            .as_ref()
            .expect("one UTXO, three transactions")
            .unsigned_tx
            .compute_txid();

        // The chain reports the root confirmed, so it is emitted but drives nothing.
        let resolved = ResolvedExitState {
            nodes: [(id("root"), NodeState::ConfirmedCpfp { change: None })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let build = build_exit(&plan, &resolved, FEE_RATE).unwrap();

        for tx in &build.branches[0].txs {
            if tx.to_sign.is_none() {
                continue;
            }
            assert!(
                tx.depends_on.contains(&fan_out_txid),
                "{:?} {} is funded by the fan-out but does not wait on it",
                tx.kind,
                tx.txid
            );
        }
    }

    #[test]
    fn build_per_node_resume_ignores_a_confirmed_steps_fan_out_output() {
        // A confirmed step builds no child, so its fan-out output, spent by the
        // earlier run and fixed at that run's rate, cannot be held to the current
        // rate: that would refuse a resume the still driven steps can afford.
        let plan = per_node_plan(vec![funding(100_000)], TreeNodeStatus::Available).unwrap();
        let psbt = plan
            .fan_out_psbt
            .clone()
            .expect("one UTXO, three transactions");
        let confirmed_txid = Txid::from_byte_array([0x99; 32]);
        let output_values = [1, 5_000, 5_000];
        let branch_outputs = |values: [u64; 3]| {
            values
                .iter()
                .enumerate()
                .map(|(vout, value)| ConfirmedOutput {
                    outpoint: OutPoint {
                        txid: confirmed_txid,
                        vout: vout as u32,
                    },
                    value: *value,
                })
                .collect::<Vec<_>>()
        };

        // The root confirmed, so its one-sat output is dead and passes ungated.
        let resolved = ResolvedExitState {
            fan_out: Some(ConfirmedFanOut {
                tx: psbt.unsigned_tx.clone(),
                branch_outputs: [(id("leaf"), branch_outputs(output_values))]
                    .into_iter()
                    .collect(),
            }),
            nodes: [(id("root"), NodeState::ConfirmedCpfp { change: None })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let build = build_exit(&plan, &resolved, FEE_RATE).unwrap();
        let spent = child_inputs(&build.branches[0]);
        assert_eq!(
            spent.iter().map(|i| i[0].vout).collect::<Vec<_>>(),
            vec![1, 2],
            "the driven steps keep their own adopted outputs"
        );

        // Still driven, the same one-sat output is short and refused.
        let resolved = ResolvedExitState {
            fan_out: Some(ConfirmedFanOut {
                tx: psbt.unsigned_tx.clone(),
                branch_outputs: [(id("leaf"), branch_outputs(output_values))]
                    .into_iter()
                    .collect(),
            }),
            ..Default::default()
        };
        let err = build_exit(&plan, &resolved, FEE_RATE)
            .expect_err("a driven step's output is held to its funding amount");
        assert!(matches!(
            err,
            SparkWalletError::ServiceError(ServiceError::InsufficientCpfpBudget { .. })
        ));
    }

    #[test]
    fn build_per_node_refuses_funding_spent_from_under_a_driven_step() {
        // A per-node UTXO looks like any other coin to the caller's wallet. Spent
        // away from the still driven step it funds, the exit refuses and names the
        // outpoint rather than handing back a child that cannot broadcast.
        let inputs = per_node_inputs(3, 5_000);
        let plan = per_node_plan(inputs.clone(), TreeNodeStatus::Available).unwrap();
        let prepared = PreparedUnilateralExit {
            plan,
            leaf_refund_addresses: HashMap::new(),
        };
        let foreign = Txid::from_byte_array([0xaa; 32]);
        let spend = |confirmed: bool| {
            vec![Observation {
                query: ChainQuery::Outspend(inputs[1].outpoint),
                result: ChainResult::Spend(Some(SpendInfo {
                    spender_txid: foreign,
                    confirmed,
                })),
            }]
        };

        let err = build_unilateral_exit(&prepared, &spend(true), FEE_RATE)
            .expect_err("a confirmed foreign spend of driven funding refuses");
        assert!(
            matches!(
                err,
                SparkWalletError::ServiceError(ServiceError::FundingUtxoConflict { .. })
            ),
            "expected FundingUtxoConflict, got: {err:?}"
        );

        // An unconfirmed spender is the step's own replaceable child.
        assert!(build_unilateral_exit(&prepared, &spend(false), FEE_RATE).is_ok());
    }

    #[test]
    fn build_per_node_fan_out_funds_one_transaction_per_output() {
        // A caller with one coin fans it out per transaction. On resume the confirmed
        // fan-out's outputs have to land back on the same transactions, in order.
        let plan = per_node_plan(vec![funding(100_000)], TreeNodeStatus::Available).unwrap();
        let psbt = plan
            .fan_out_psbt
            .clone()
            .expect("one UTXO, three transactions");
        assert_eq!(psbt.unsigned_tx.output.len(), 3);

        let confirmed_txid = Txid::from_byte_array([0x99; 32]);
        let branch_outputs = (0..3u32)
            .map(|vout| ConfirmedOutput {
                outpoint: OutPoint {
                    txid: confirmed_txid,
                    vout,
                },
                value: 5_000,
            })
            .collect();
        let resolved = ResolvedExitState {
            fan_out: Some(ConfirmedFanOut {
                tx: psbt.unsigned_tx.clone(),
                branch_outputs: [(id("leaf"), branch_outputs)].into_iter().collect(),
            }),
            ..Default::default()
        };

        let build = build_exit(&plan, &resolved, FEE_RATE).unwrap();
        let spent = child_inputs(&build.branches[0]);
        assert_eq!(
            spent.iter().map(|i| i[0].vout).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "each transaction keeps the fan-out output the plan gave it"
        );
        assert!(
            spent.iter().all(|inputs| inputs[0].txid == confirmed_txid),
            "the adopted fan-out replaces the planned outpoints"
        );
    }

    #[test]
    fn build_per_node_splits_a_fan_out_between_branches_of_unequal_length() {
        // Two branches sharing a root fee-bump three transactions and two. A
        // confirmed fan-out pays one output per transaction, in plan order, and
        // each branch has to take its own run: hand one branch an output belonging
        // to the other and two children would spend the same UTXO.
        let plan = per_node_two_branch_plan(vec![funding(400_000)]).unwrap();
        let named: Vec<usize> = plan.per_node_funding.iter().map(|(_, f)| f.len()).collect();
        assert_eq!(named, vec![3, 2], "the shared root is named once, by leafA");
        let psbt = plan
            .fan_out_psbt
            .clone()
            .expect("one UTXO, five transactions");
        assert_eq!(psbt.unsigned_tx.output.len(), 5);

        let confirmed_txid = Txid::from_byte_array([0x99; 32]);
        let adopted: Vec<ConfirmedOutput> = (0..5u32)
            .map(|vout| ConfirmedOutput {
                outpoint: OutPoint {
                    txid: confirmed_txid,
                    vout,
                },
                value: 60_000,
            })
            .collect();
        let resolved = ResolvedExitState {
            fan_out: Some(ConfirmedFanOut {
                tx: psbt.unsigned_tx.clone(),
                branch_outputs: [
                    (id("leafA"), adopted[..3].to_vec()),
                    (id("leafB"), adopted[3..].to_vec()),
                ]
                .into_iter()
                .collect(),
            }),
            ..Default::default()
        };

        let build = build_exit(&plan, &resolved, FEE_RATE).unwrap();
        let mut spent: Vec<u32> = build
            .branches
            .iter()
            .flat_map(|b| b.txs.iter())
            .filter_map(|tx| tx.to_sign.as_ref())
            .map(|child| {
                let funding_input = child
                    .unsigned_tx
                    .input
                    .iter()
                    .find(|i| i.previous_output.txid == confirmed_txid)
                    .expect("each child spends an adopted output");
                funding_input.previous_output.vout
            })
            .collect();
        spent.sort_unstable();
        assert_eq!(
            spent,
            vec![0, 1, 2, 3, 4],
            "every adopted output funds exactly one child"
        );
    }

    #[test]
    fn build_per_node_refuses_an_adopted_fan_out_short_of_a_branch() {
        // The interpreter only adopts a fan-out paying exactly the planned outputs, so
        // a branch handed fewer than it has transactions is a resolved state no chain
        // produced. Pairing what is there would fund the last transactions from
        // nothing; the mismatch is refused instead.
        let plan = per_node_plan(vec![funding(100_000)], TreeNodeStatus::Available).unwrap();
        let psbt = plan
            .fan_out_psbt
            .clone()
            .expect("one UTXO, three transactions");
        let branch_outputs = (0..2u32)
            .map(|vout| ConfirmedOutput {
                outpoint: OutPoint {
                    txid: Txid::from_byte_array([0x99; 32]),
                    vout,
                },
                value: 5_000,
            })
            .collect();
        let resolved = ResolvedExitState {
            fan_out: Some(ConfirmedFanOut {
                tx: psbt.unsigned_tx.clone(),
                branch_outputs: [(id("leaf"), branch_outputs)].into_iter().collect(),
            }),
            ..Default::default()
        };

        let err = build_exit(&plan, &resolved, FEE_RATE).expect_err("two outputs, three inputs");
        assert!(
            matches!(err, SparkWalletError::Generic(ref m) if m.contains("2 outputs for 3 inputs")),
            "expected the short branch to be named, got: {err:?}"
        );
    }

    #[test]
    fn plan_three_leaves_two_utxos_fans_out_and_builds() {
        // Case e: more leaves than funding UTXOs. The plan fans out (2 inputs, 3
        // outputs) and build_exit threads all three branches off that fan-out.
        let leaf_a = node("leafA", None, anchor_tx(1), Some(anchor_tx(2)));
        let leaf_b = node("leafB", None, anchor_tx(3), Some(anchor_tx(4)));
        let leaf_c = node("leafC", None, anchor_tx(5), Some(anchor_tx(6)));
        let (a_id, b_id, c_id) = (leaf_a.id.clone(), leaf_b.id.clone(), leaf_c.id.clone());
        let nodes: HashMap<TreeNodeId, TreeNode> = [
            (a_id.clone(), leaf_a),
            (b_id.clone(), leaf_b),
            (c_id.clone(), leaf_c),
        ]
        .into_iter()
        .collect();

        // Two UTXOs cannot be matched one-per-branch across three leaves. Funded
        // generously so the fan-out itself is affordable.
        let two = |v: u64| {
            let mut a = funding(v);
            a.outpoint.vout = 0;
            let mut b = funding(v);
            b.outpoint.vout = 1;
            vec![a, b]
        };
        let change_len = funding(0).witness_utxo.script_pubkey.len();
        let plan = plan_unilateral_exit(
            nodes,
            &[a_id, b_id, c_id],
            UnilateralExitLeafFilter::ProfitableOnly,
            two(50_000),
            CpfpFundingShape::PerBranch,
            FEE_RATE,
            change_len,
        )
        .unwrap();
        assert!(
            plan.fan_out_psbt.is_some(),
            "two UTXOs for three leaves must fan out"
        );
        assert_eq!(plan.per_branch_funding.len(), 3);

        let build = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE).unwrap();
        assert!(build.fan_out.is_some());
        assert_eq!(build.branches.len(), 3);
    }
}

#[cfg(test)]
mod interpret_tests {
    use super::*;
    use bitcoin::{
        CompressedPublicKey, ScriptBuf, Sequence, TxIn, TxOut, absolute::LockTime, hashes::Hash,
        secp256k1::PublicKey, transaction::Version,
    };
    use spark::{
        Identifier,
        tree::{SigningKeyshare, TreeNodeStatus},
    };
    use std::str::FromStr;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const PK: &str = "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443";

    fn pubkey() -> PublicKey {
        PublicKey::from_str(PK).unwrap()
    }

    fn tx_spending(prev: OutPoint, nonce: u32) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::from_height(nonce).unwrap(),
            input: vec![TxIn {
                previous_output: prev,
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(10_000),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    fn treenode(id: &str, parent: Option<&str>, node_tx: Transaction, vout: u32) -> TreeNode {
        let pk = pubkey();
        TreeNode {
            id: TreeNodeId::from_str(id).unwrap(),
            tree_id: "t".to_string(),
            value: 100_000,
            parent_node_id: parent.map(|p| TreeNodeId::from_str(p).unwrap()),
            node_tx,
            refund_tx: None,
            direct_tx: None,
            direct_refund_tx: None,
            direct_from_cpfp_refund_tx: None,
            vout,
            verifying_public_key: pk,
            owner_identity_public_key: Some(pk),
            signing_keyshare: SigningKeyshare {
                public_key: pk,
                owner_identifiers: vec![Identifier::try_from(1u16).unwrap()],
                threshold: 2,
            },
            status: TreeNodeStatus::Available,
        }
    }

    fn leaf_addr() -> Address {
        Address::p2wpkh(&CompressedPublicKey(pubkey()), bitcoin::Network::Regtest)
    }

    fn id(s: &str) -> TreeNodeId {
        TreeNodeId::from_str(s).unwrap()
    }

    fn prepared_of(root: TreeNode, leaf: TreeNode) -> PreparedUnilateralExit {
        let leaf_id = leaf.id.clone();
        PreparedUnilateralExit {
            plan: UnilateralExitPlan {
                funding_shape: CpfpFundingShape::PerBranch,
                per_node_funding: vec![],
                selected_leaves: vec![],
                fan_out_psbt: None,
                per_branch_funding: vec![(leaf_id.clone(), vec![])],
                tree_nodes: to_node_map(vec![root, leaf]),
            },
            leaf_refund_addresses: [(leaf_id, leaf_addr())].into_iter().collect(),
        }
    }

    /// An output paying the shared test funding script.
    fn funding_output(value: u64) -> TxOut {
        TxOut {
            value: Amount::from_sat(value),
            script_pubkey: leaf_addr().script_pubkey(),
        }
    }

    /// A transaction spending the funding outpoint into `outputs`, the shape a
    /// fan-out takes on-chain.
    fn spend_funding_with(funding_outpoint: OutPoint, outputs: Vec<TxOut>) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: funding_outpoint,
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                ..Default::default()
            }],
            output: outputs,
        }
    }

    fn spent(outpoint: OutPoint, spender: Txid) -> Observation {
        Observation {
            query: ChainQuery::Outspend(outpoint),
            result: ChainResult::Spend(Some(SpendInfo {
                spender_txid: spender,
                confirmed: true,
            })),
        }
    }

    fn no_refund(leaf_id: &TreeNodeId) -> Observation {
        Observation {
            query: ChainQuery::RefundAddress {
                leaf_id: leaf_id.clone(),
                address: leaf_addr(),
            },
            result: ChainResult::AddressUtxos(vec![]),
        }
    }

    fn refund_scan(leaf_id: &TreeNodeId, refund_txid: Txid, value: u64) -> Observation {
        Observation {
            query: ChainQuery::RefundAddress {
                leaf_id: leaf_id.clone(),
                address: leaf_addr(),
            },
            result: ChainResult::AddressUtxos(vec![AddressUtxo {
                txid: refund_txid,
                vout: 0,
                value,
                confirmed: true,
            }]),
        }
    }

    fn unspent(outpoint: OutPoint) -> Observation {
        Observation {
            query: ChainQuery::Outspend(outpoint),
            result: ChainResult::Spend(None),
        }
    }

    fn spent_unconfirmed(outpoint: OutPoint, spender: Txid) -> Observation {
        Observation {
            query: ChainQuery::Outspend(outpoint),
            result: ChainResult::Spend(Some(SpendInfo {
                spender_txid: spender,
                confirmed: false,
            })),
        }
    }

    #[test]
    fn next_query_probes_deposit_and_refund_address() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root = treenode("root", None, tx_spending(deposit, 1), 0);
        let leaf = treenode(
            "leaf",
            Some("root"),
            tx_spending(
                OutPoint {
                    txid: root.node_tx.compute_txid(),
                    vout: 0,
                },
                2,
            ),
            0,
        );
        let prepared = prepared_of(root, leaf);

        let queries = next_chain_queries(&prepared, &[]).unwrap();
        assert!(queries.contains(&ChainQuery::Outspend(deposit)));
        assert!(
            queries
                .iter()
                .any(|q| matches!(q, ChainQuery::RefundAddress { .. }))
        );
    }

    #[test]
    fn interpret_detects_leaf_direct() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root_tx = tx_spending(deposit, 1);
        let root_txid = root_tx.compute_txid();
        let root = treenode("root", None, root_tx, 0);

        let leaf_parent_out = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf_direct = tx_spending(leaf_parent_out, 3);
        let leaf_direct_txid = leaf_direct.compute_txid();
        let mut leaf = treenode("leaf", Some("root"), tx_spending(leaf_parent_out, 2), 0);
        leaf.direct_tx = Some(leaf_direct);
        leaf.direct_refund_tx = Some(tx_spending(
            OutPoint {
                txid: leaf_direct_txid,
                vout: 0,
            },
            4,
        ));
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let observed = vec![
            spent(deposit, root_txid),
            spent(leaf_parent_out, leaf_direct_txid),
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert!(interp.pending.is_empty(), "state is fully resolved");
        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp { change: None }),
            "the leaf went direct, so the root's cpfp change is never resolved"
        );
        assert_eq!(
            interp.resolved.nodes.get(&leaf_id),
            Some(&NodeState::ConfirmedDirect)
        );
        assert!(matches!(
            interp.resolved.refunds.get(&leaf_id),
            Some(RefundState::DriveDirect)
        ));
    }

    /// A per-node root-to-leaf plan over real spending transactions, with every
    /// UTXO named, so the walk and the funding checks run as they would on-chain.
    fn per_node_prepared(
        root_tx: Transaction,
        leaf_tx: Transaction,
        refund_tx: Transaction,
    ) -> PreparedUnilateralExit {
        let root = treenode("root", None, root_tx.clone(), 0);
        let mut leaf = treenode("leaf", Some("root"), leaf_tx.clone(), 0);
        leaf.refund_tx = Some(refund_tx.clone());
        let leaf_id = leaf.id.clone();
        let named = |node: &str, txid: Txid, refund: bool| UnilateralExitNodeFunding {
            leaf_id: leaf_id.clone(),
            node_id: id(node),
            txid,
            refund,
            funding_sat: 5_000,
        };
        let input = |vout: u32| CpfpInput {
            outpoint: OutPoint {
                txid: Txid::from_byte_array([7u8; 32]),
                vout,
            },
            witness_utxo: funding_output(5_000),
            signed_input_weight: 272,
        };
        PreparedUnilateralExit {
            plan: UnilateralExitPlan {
                funding_shape: CpfpFundingShape::PerNode,
                per_node_funding: vec![(
                    leaf_id.clone(),
                    vec![
                        named("root", root_tx.compute_txid(), false),
                        named("leaf", leaf_tx.compute_txid(), false),
                        named("leaf", refund_tx.compute_txid(), true),
                    ],
                )],
                selected_leaves: vec![],
                fan_out_psbt: None,
                per_branch_funding: vec![(leaf_id.clone(), vec![input(0), input(1), input(2)])],
                tree_nodes: to_node_map(vec![root, leaf]),
            },
            leaf_refund_addresses: [(leaf_id, leaf_addr())].into_iter().collect(),
        }
    }

    /// Drives [`next_chain_queries`] to completion against `chain`, one round at
    /// a time, the way the SDK's observation loop does.
    fn drive(
        prepared: &PreparedUnilateralExit,
        chain: impl Fn(&ChainQuery) -> ChainResult,
    ) -> Result<Vec<Observation>, SparkWalletError> {
        let mut observed = Vec::new();
        loop {
            let queries = next_chain_queries(prepared, &observed)?;
            if queries.is_empty() {
                return Ok(observed);
            }
            for query in queries {
                let result = chain(&query);
                observed.push(Observation { query, result });
            }
        }
    }

    #[test]
    fn interpret_per_node_resume_waits_for_the_walk_before_judging_funding() {
        // The walk classifies one level per round while every funding UTXO can be
        // queried at once. A step whose own child confirmed in an earlier run has
        // its UTXO spent, and read as driven before the walk reaches it that spend
        // would look like a conflict. The check waits for the walk instead.
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        // Each transaction carries the anchor its child spends.
        let anchored = |mut tx: Transaction| {
            tx.output.push(TxOut {
                value: Amount::from_sat(0),
                script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
            });
            tx
        };
        let root_tx = anchored(tx_spending(deposit, 1));
        let root_txid = root_tx.compute_txid();
        let leaf_tx = anchored(tx_spending(
            OutPoint {
                txid: root_txid,
                vout: 0,
            },
            2,
        ));
        let leaf_txid = leaf_tx.compute_txid();
        let refund_tx = anchored(tx_spending(
            OutPoint {
                txid: leaf_txid,
                vout: 0,
            },
            3,
        ));
        let prepared = per_node_prepared(root_tx, leaf_tx, refund_tx);
        let funding = |vout: u32| OutPoint {
            txid: Txid::from_byte_array([7u8; 32]),
            vout,
        };
        let confirmed_by = |spender: Txid| {
            ChainResult::Spend(Some(SpendInfo {
                spender_txid: spender,
                confirmed: true,
            }))
        };
        let root_child = Txid::from_byte_array([0xa1; 32]);
        let leaf_child = Txid::from_byte_array([0xa2; 32]);

        // Root and leaf confirmed through their own children, which spent the
        // first two UTXOs; the refund is still waiting out its timelock.
        let chain = move |query: &ChainQuery| match query {
            ChainQuery::Outspend(op) if *op == deposit => confirmed_by(root_txid),
            ChainQuery::Outspend(op) if op.txid == root_txid && op.vout == 0 => {
                confirmed_by(leaf_txid)
            }
            ChainQuery::Outspend(op) if *op == funding(0) => confirmed_by(root_child),
            ChainQuery::Outspend(op) if *op == funding(1) => confirmed_by(leaf_child),
            ChainQuery::Outspend(_) => ChainResult::Spend(None),
            ChainQuery::RefundAddress { .. } => ChainResult::AddressUtxos(vec![]),
            ChainQuery::Transaction(_) => ChainResult::Unavailable,
        };
        let observed = drive(&prepared, chain).expect("a resume over confirmed steps");
        let build = build_unilateral_exit(&prepared, &observed, 250).unwrap();
        let driven: Vec<_> = build.branches[0]
            .txs
            .iter()
            .filter(|tx| tx.to_sign.is_some())
            .map(|tx| tx.kind)
            .collect();
        assert_eq!(
            driven,
            vec![ExitTxKind::Refund],
            "only the refund is left to drive"
        );

        // The refund's own UTXO spent away by something else is a conflict, found
        // once the walk has finished.
        let foreign = Txid::from_byte_array([0xee; 32]);
        let chain = move |query: &ChainQuery| match query {
            ChainQuery::Outspend(op) if *op == funding(2) => confirmed_by(foreign),
            other => chain(other),
        };
        let err = drive(&prepared, chain).expect_err("the refund's funding is gone");
        assert!(
            matches!(
                err,
                SparkWalletError::ServiceError(ServiceError::FundingUtxoConflict { vout: 2, .. })
            ),
            "expected the refund's outpoint named, got: {err:?}"
        );

        // A lookup that fails cannot say either way, so the refund is still built,
        // flagged unverified rather than handed back as a plain unconfirmed step.
        let chain = move |query: &ChainQuery| match query {
            ChainQuery::Outspend(op) if *op == funding(2) => ChainResult::Unavailable,
            other => chain(other),
        };
        let observed = drive(&prepared, chain).expect("an unverifiable funding still builds");
        let build = build_unilateral_exit(&prepared, &observed, 250).unwrap();
        let refund = build.branches[0]
            .txs
            .iter()
            .find(|tx| tx.kind == ExitTxKind::Refund)
            .unwrap();
        assert!(refund.to_sign.is_some());
        assert_eq!(refund.status, ExitTxStatus::Unverified);

        // The walk stopping at a lookup it could not make classifies nothing below
        // it, so the leaf, confirmed through its own child in truth, reads as
        // driven. Its spent UTXO is no conflict: the branch is built from the
        // unverified node down, flagged there, rather than refused.
        let chain = move |query: &ChainQuery| match query {
            ChainQuery::Outspend(op) if *op == deposit => ChainResult::Unavailable,
            other => chain(other),
        };
        let observed = drive(&prepared, chain).expect("an unverifiable walk still builds");
        let build = build_unilateral_exit(&prepared, &observed, 250).unwrap();
        let statuses: Vec<_> = build.branches[0]
            .txs
            .iter()
            .filter(|tx| tx.to_sign.is_some())
            .map(|tx| (tx.kind, tx.status))
            .collect();
        assert_eq!(
            statuses,
            vec![
                (ExitTxKind::Node, ExitTxStatus::Unverified),
                (ExitTxKind::Node, ExitTxStatus::Unconfirmed),
                (ExitTxKind::Refund, ExitTxStatus::Unconfirmed),
            ],
            "every step from the unverified node down is driven"
        );
    }

    #[test]
    fn interpret_per_branch_still_takes_one_output_per_branch_from_a_wider_fan_out() {
        // Per-branch adoption is unchanged by the second shape: a confirmed fan-out
        // paying the funding script more times than the plan has branches is still
        // adopted, one output per branch in plan order, as it is on a narrowed
        // re-quote. Only a per-node plan refuses a wider one.
        let funding_outpoint = OutPoint {
            txid: Txid::from_byte_array([9u8; 32]),
            vout: 0,
        };
        let mut fan_out_psbt = bitcoin::Psbt::from_unsigned_tx(spend_funding_with(
            funding_outpoint,
            vec![funding_output(5_000)],
        ))
        .unwrap();
        fan_out_psbt.inputs[0].witness_utxo = Some(funding_output(20_000));
        let prepared = PreparedUnilateralExit {
            plan: UnilateralExitPlan {
                funding_shape: CpfpFundingShape::PerBranch,
                per_node_funding: vec![],
                selected_leaves: vec![],
                fan_out_psbt: Some(fan_out_psbt),
                per_branch_funding: vec![(
                    id("a"),
                    vec![CpfpInput {
                        outpoint: funding_outpoint,
                        witness_utxo: funding_output(5_000),
                        signed_input_weight: 272,
                    }],
                )],
                tree_nodes: to_node_map(vec![]),
            },
            leaf_refund_addresses: HashMap::new(),
        };
        let observe = |outputs: usize| {
            let confirmed =
                spend_funding_with(funding_outpoint, vec![funding_output(5_000); outputs]);
            let txid = confirmed.compute_txid();
            vec![
                spent(funding_outpoint, txid),
                Observation {
                    query: ChainQuery::Transaction(txid),
                    result: ChainResult::Transaction(confirmed),
                },
            ]
        };

        for outputs in [1, 2, 3] {
            let interp = interpret_chain(&prepared, &observe(outputs))
                .unwrap_or_else(|e| panic!("{outputs} outputs adopted under per-branch: {e}"));
            let adopted = &interp
                .resolved
                .fan_out
                .expect("fan-out adopted")
                .branch_outputs[&id("a")];
            assert_eq!(
                adopted.iter().map(|o| o.outpoint.vout).collect::<Vec<_>>(),
                vec![0],
                "the branch takes the first output"
            );
        }
    }

    #[test]
    fn interpret_rejects_a_branch_named_twice() {
        // Two branches under one leaf id would resolve to the same fan-out output and
        // spend it twice, so the adoption refuses rather than hand that back.
        let funding_outpoint = OutPoint {
            txid: Txid::from_byte_array([9u8; 32]),
            vout: 0,
        };
        let output = funding_output;
        let spend_funding = |outputs: Vec<TxOut>| spend_funding_with(funding_outpoint, outputs);

        let mut fan_out_psbt =
            bitcoin::Psbt::from_unsigned_tx(spend_funding(vec![output(5_000), output(5_000)]))
                .unwrap();
        fan_out_psbt.inputs[0].witness_utxo = Some(output(20_000));
        let confirmed = spend_funding(vec![output(5_000), output(5_000)]);
        let confirmed_txid = confirmed.compute_txid();

        let prepared = PreparedUnilateralExit {
            plan: UnilateralExitPlan {
                funding_shape: CpfpFundingShape::PerBranch,
                per_node_funding: vec![],
                selected_leaves: vec![],
                fan_out_psbt: Some(fan_out_psbt),
                per_branch_funding: vec![(id("a"), vec![]), (id("a"), vec![])],
                tree_nodes: to_node_map(vec![]),
            },
            leaf_refund_addresses: HashMap::new(),
        };
        let observed = vec![
            spent(funding_outpoint, confirmed_txid),
            Observation {
                query: ChainQuery::Transaction(confirmed_txid),
                result: ChainResult::Transaction(confirmed),
            },
        ];

        assert!(
            matches!(
                interpret_chain(&prepared, &observed),
                Err(SparkWalletError::ServiceError(
                    ServiceError::FundingUtxoConflict { .. }
                ))
            ),
            "one leaf id cannot fund two branches"
        );
    }

    #[test]
    fn interpret_per_node_adopts_a_fan_out_across_branches_and_watches_its_outputs() {
        // A confirmed fan-out pays one output per fee-bumped transaction, and two
        // branches of unequal length take their own runs of it in plan order. Those
        // adopted outputs are what the funding watch then reads: one spent from
        // under a still driven step refuses the resume, exactly as a supplied UTXO
        // would. Nothing else drives this arm, so a fan-out that adopted into the
        // wrong branch, or a watch that read the pre-fan-out outpoints, would pass
        // every other test in this file.
        let funding_outpoint = OutPoint {
            txid: Txid::from_byte_array([9u8; 32]),
            vout: 0,
        };
        let output = funding_output;
        let mut fan_out_psbt = bitcoin::Psbt::from_unsigned_tx(spend_funding_with(
            funding_outpoint,
            vec![output(5_000); 3],
        ))
        .unwrap();
        fan_out_psbt.inputs[0].witness_utxo = Some(output(20_000));
        let input_at = |vout: u32| CpfpInput {
            outpoint: OutPoint {
                txid: funding_outpoint.txid,
                vout,
            },
            witness_utxo: output(5_000),
            signed_input_weight: 272,
        };
        let target = |leaf: &str, node: &str, refund: bool| UnilateralExitNodeFunding {
            leaf_id: id(leaf),
            node_id: id(node),
            txid: Txid::from_byte_array([u8::try_from(node.len()).unwrap(); 32]),
            refund,
            funding_sat: 5_000,
        };
        let prepared = PreparedUnilateralExit {
            plan: UnilateralExitPlan {
                funding_shape: CpfpFundingShape::PerNode,
                // leafA fee-bumps two transactions, leafB one.
                per_node_funding: vec![
                    (
                        id("a"),
                        vec![target("a", "rootnode", false), target("a", "a", true)],
                    ),
                    (id("b"), vec![target("b", "b", true)]),
                ],
                selected_leaves: vec![],
                fan_out_psbt: Some(fan_out_psbt),
                per_branch_funding: vec![
                    (id("a"), vec![input_at(0), input_at(1)]),
                    (id("b"), vec![input_at(2)]),
                ],
                tree_nodes: to_node_map(vec![]),
            },
            leaf_refund_addresses: HashMap::new(),
        };

        let confirmed = spend_funding_with(funding_outpoint, vec![output(5_000); 3]);
        let confirmed_txid = confirmed.compute_txid();
        let adopted = |vout: u32| OutPoint {
            txid: confirmed_txid,
            vout,
        };
        let seen = vec![
            spent(funding_outpoint, confirmed_txid),
            Observation {
                query: ChainQuery::Transaction(confirmed_txid),
                result: ChainResult::Transaction(confirmed),
            },
        ];

        let Ok(interp) = interpret_chain(&prepared, &seen) else {
            panic!("the fan-out is adopted");
        };
        let branch_outputs = &interp.resolved.fan_out.as_ref().unwrap().branch_outputs;
        assert_eq!(
            branch_outputs[&id("a")]
                .iter()
                .map(|o| o.outpoint.vout)
                .collect::<Vec<_>>(),
            vec![0, 1],
            "the longer branch takes the first run of outputs"
        );
        assert_eq!(
            branch_outputs[&id("b")]
                .iter()
                .map(|o| o.outpoint.vout)
                .collect::<Vec<_>>(),
            vec![2],
            "the next branch continues where it left off"
        );

        // The watch runs over the adopted outputs, not the planned ones: the last
        // one, funding leafB's still driven refund, is spent away by a stranger.
        let foreign = Txid::from_byte_array([0xee; 32]);
        let mut with_conflict = seen.clone();
        with_conflict.push(spent(adopted(2), foreign));
        let Err(err) = interpret_chain(&prepared, &with_conflict) else {
            panic!("leafB's adopted funding is gone, the resume must refuse");
        };
        assert!(
            matches!(
                err,
                SparkWalletError::ServiceError(ServiceError::FundingUtxoConflict { vout: 2, ref txid })
                    if txid == &confirmed_txid.to_string()
            ),
            "expected the adopted outpoint named, got: {err:?}"
        );
    }

    #[test]
    fn interpret_rejects_a_fan_out_wider_than_the_plan_under_per_node() {
        // Per-node outputs are matched to transactions by position, and an Auto
        // re-quote that dropped a leaf names fewer of them. A wider fan-out from
        // the earlier run would slide the survivors onto the wrong outputs, so it
        // is refused; naming the original leaves restores the width.
        let funding_outpoint = OutPoint {
            txid: Txid::from_byte_array([9u8; 32]),
            vout: 0,
        };
        let output = funding_output;
        let spend_funding = |outputs: Vec<TxOut>| spend_funding_with(funding_outpoint, outputs);

        // The plan now fee-bumps two transactions, so it plans a two-output fan-out.
        let mut fan_out_psbt =
            bitcoin::Psbt::from_unsigned_tx(spend_funding(vec![output(5_000), output(5_000)]))
                .unwrap();
        fan_out_psbt.inputs[0].witness_utxo = Some(output(20_000));
        let branch_funding = vec![CpfpInput {
            outpoint: funding_outpoint,
            witness_utxo: output(5_000),
            signed_input_weight: 272,
        }];

        // The confirmed one from the earlier run fanned out three.
        let confirmed = spend_funding(vec![output(5_000), output(5_000), output(5_000)]);
        let confirmed_txid = confirmed.compute_txid();
        let observed = vec![
            spent(funding_outpoint, confirmed_txid),
            Observation {
                query: ChainQuery::Transaction(confirmed_txid),
                result: ChainResult::Transaction(confirmed),
            },
        ];

        let prepared_of = |shape| PreparedUnilateralExit {
            plan: UnilateralExitPlan {
                funding_shape: shape,
                per_node_funding: vec![],
                selected_leaves: vec![],
                fan_out_psbt: Some(fan_out_psbt.clone()),
                per_branch_funding: vec![
                    (id("a"), branch_funding.clone()),
                    (id("b"), branch_funding.clone()),
                ],
                tree_nodes: to_node_map(vec![]),
            },
            leaf_refund_addresses: HashMap::new(),
        };

        assert!(
            matches!(
                interpret_chain(&prepared_of(CpfpFundingShape::PerNode), &observed),
                Err(SparkWalletError::ServiceError(
                    ServiceError::FundingUtxoConflict { .. }
                ))
            ),
            "per-node funding must refuse a fan-out built for more transactions"
        );
        assert!(
            interpret_chain(&prepared_of(CpfpFundingShape::PerBranch), &observed).is_ok(),
            "per-branch funding still takes one output per branch"
        );
    }

    #[test]
    fn interpret_flags_funding_conflict() {
        let funding_outpoint = OutPoint {
            txid: Txid::from_byte_array([9u8; 32]),
            vout: 0,
        };
        let funding_script = leaf_addr().script_pubkey();

        let fan_out_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: funding_outpoint,
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                ..Default::default()
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(5_000),
                    script_pubkey: funding_script.clone(),
                },
                TxOut {
                    value: Amount::from_sat(5_000),
                    script_pubkey: funding_script.clone(),
                },
            ],
        };
        let mut fan_out_psbt = bitcoin::Psbt::from_unsigned_tx(fan_out_tx).unwrap();
        fan_out_psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(12_000),
            script_pubkey: funding_script,
        });

        let prepared = PreparedUnilateralExit {
            plan: UnilateralExitPlan {
                funding_shape: CpfpFundingShape::PerBranch,
                per_node_funding: vec![],
                selected_leaves: vec![],
                fan_out_psbt: Some(fan_out_psbt),
                per_branch_funding: vec![(id("a"), vec![]), (id("b"), vec![])],
                tree_nodes: to_node_map(vec![]),
            },
            leaf_refund_addresses: HashMap::new(),
        };

        let conflicting = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: funding_outpoint,
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(11_000),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        let conflicting_txid = conflicting.compute_txid();
        let observed = vec![
            spent(funding_outpoint, conflicting_txid),
            Observation {
                query: ChainQuery::Transaction(conflicting_txid),
                result: ChainResult::Transaction(conflicting),
            },
        ];

        assert!(
            matches!(
                interpret_chain(&prepared, &observed),
                Err(SparkWalletError::ServiceError(
                    ServiceError::FundingUtxoConflict { .. }
                ))
            ),
            "a non-fan-out spender of the funding UTXO must be a FundingUtxoConflict"
        );
    }

    #[test]
    fn interpret_stops_branch_on_foreign_spend() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root = treenode("root", None, tx_spending(deposit, 1), 0);
        let leaf = treenode(
            "leaf",
            Some("root"),
            tx_spending(
                OutPoint {
                    txid: root.node_tx.compute_txid(),
                    vout: 0,
                },
                2,
            ),
            0,
        );
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let observed = vec![
            spent(deposit, Txid::from_byte_array([9u8; 32])),
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert!(
            interp.resolved.stopped.contains(&leaf_id),
            "a foreign spender stops the branch"
        );
        assert!(
            !interp.resolved.nodes.contains_key(&id("root")),
            "a stopped branch records no node state"
        );
        assert!(
            !interp.resolved.refunds.contains_key(&leaf_id),
            "a stopped branch drives no refund"
        );
    }

    #[test]
    fn interpret_adopts_onchain_refund() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root_tx = tx_spending(deposit, 1);
        let root_txid = root_tx.compute_txid();
        let root = treenode("root", None, root_tx, 0);
        let leaf_parent_out = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf_cpfp = tx_spending(leaf_parent_out, 2);
        let leaf_cpfp_txid = leaf_cpfp.compute_txid();
        let leaf = treenode("leaf", Some("root"), leaf_cpfp, 0);
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let refund_tx = tx_spending(
            OutPoint {
                txid: leaf_cpfp_txid,
                vout: 0,
            },
            5,
        );
        let refund_txid = refund_tx.compute_txid();

        let refund_outpoint = OutPoint {
            txid: refund_txid,
            vout: 0,
        };
        let observed = vec![
            spent(deposit, root_txid),
            spent(leaf_parent_out, leaf_cpfp_txid),
            refund_scan(&leaf_id, refund_txid, 42_000),
            unspent(refund_outpoint),
            Observation {
                query: ChainQuery::Transaction(refund_txid),
                result: ChainResult::Transaction(refund_tx),
            },
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert!(interp.pending.is_empty());
        match interp.resolved.refunds.get(&leaf_id) {
            Some(RefundState::Adopted(adopted)) => {
                assert_eq!(adopted.outpoint.txid, refund_txid);
                assert_eq!(adopted.value, 42_000);
            }
            other => panic!("expected an adopted refund, got {other:?}"),
        }
    }

    #[test]
    fn interpret_readopts_pending_swept_refund() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root_tx = tx_spending(deposit, 1);
        let root_txid = root_tx.compute_txid();
        let root = treenode("root", None, root_tx, 0);
        let leaf_parent_out = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf_cpfp = tx_spending(leaf_parent_out, 2);
        let leaf_cpfp_txid = leaf_cpfp.compute_txid();
        let leaf = treenode("leaf", Some("root"), leaf_cpfp, 0);
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let refund_tx = tx_spending(
            OutPoint {
                txid: leaf_cpfp_txid,
                vout: 0,
            },
            5,
        );
        let refund_txid = refund_tx.compute_txid();
        let refund_outpoint = OutPoint {
            txid: refund_txid,
            vout: 0,
        };
        // The refund is confirmed but spent by an unconfirmed sweep (a sweep sitting
        // in the mempool), so it must be re-adopted, not treated as done.
        let observed = vec![
            spent(deposit, root_txid),
            spent(leaf_parent_out, leaf_cpfp_txid),
            refund_scan(&leaf_id, refund_txid, 42_000),
            spent_unconfirmed(refund_outpoint, Txid::from_byte_array([7u8; 32])),
            Observation {
                query: ChainQuery::Transaction(refund_txid),
                result: ChainResult::Transaction(refund_tx),
            },
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert!(interp.pending.is_empty());
        assert!(
            matches!(
                interp.resolved.refunds.get(&leaf_id),
                Some(RefundState::Adopted(_))
            ),
            "a refund spent only by an unconfirmed sweep is re-adopted, not swept"
        );
    }

    #[test]
    fn interpret_marks_swept_refund() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root_tx = tx_spending(deposit, 1);
        let root_txid = root_tx.compute_txid();
        let root = treenode("root", None, root_tx, 0);
        let leaf_parent_out = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf_cpfp = tx_spending(leaf_parent_out, 2);
        let leaf_cpfp_txid = leaf_cpfp.compute_txid();
        let leaf = treenode("leaf", Some("root"), leaf_cpfp, 0);
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let refund_txid = Txid::from_byte_array([5u8; 32]);
        let refund_outpoint = OutPoint {
            txid: refund_txid,
            vout: 0,
        };
        // The refund is confirmed and spent by a confirmed sweep: fully done.
        let observed = vec![
            spent(deposit, root_txid),
            spent(leaf_parent_out, leaf_cpfp_txid),
            refund_scan(&leaf_id, refund_txid, 42_000),
            spent(refund_outpoint, Txid::from_byte_array([7u8; 32])),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert!(interp.pending.is_empty(), "state is fully resolved");
        assert!(
            matches!(
                interp.resolved.refunds.get(&leaf_id),
                Some(RefundState::Swept)
            ),
            "a refund spent by a confirmed sweep is swept"
        );
    }

    #[test]
    fn interpret_empty_scan_leaves_refund_unresolved() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root_tx = tx_spending(deposit, 1);
        let root_txid = root_tx.compute_txid();
        let root = treenode("root", None, root_tx, 0);
        let leaf_parent_out = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf_cpfp = tx_spending(leaf_parent_out, 2);
        let leaf_cpfp_txid = leaf_cpfp.compute_txid();
        let leaf = treenode("leaf", Some("root"), leaf_cpfp, 0);
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let observed = vec![
            spent(deposit, root_txid),
            spent(leaf_parent_out, leaf_cpfp_txid),
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert!(interp.pending.is_empty());
        assert!(
            !interp.resolved.refunds.contains_key(&leaf_id),
            "a never-funded refund is driven fresh, not marked swept"
        );
    }

    #[test]
    fn next_query_probes_outspend_after_refund_found() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root = treenode("root", None, tx_spending(deposit, 1), 0);
        let leaf = treenode(
            "leaf",
            Some("root"),
            tx_spending(
                OutPoint {
                    txid: root.node_tx.compute_txid(),
                    vout: 0,
                },
                2,
            ),
            0,
        );
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        // Once a confirmed refund output is found, its spend is probed to tell an
        // adoptable refund from an already-swept one.
        let refund_txid = Txid::from_byte_array([5u8; 32]);
        let queries =
            next_chain_queries(&prepared, &[refund_scan(&leaf_id, refund_txid, 42_000)]).unwrap();
        assert!(queries.contains(&ChainQuery::Outspend(OutPoint {
            txid: refund_txid,
            vout: 0,
        })));
    }

    #[test]
    fn interpret_marks_unavailable_unverified() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root = treenode("root", None, tx_spending(deposit, 1), 0);
        let leaf = treenode(
            "leaf",
            Some("root"),
            tx_spending(
                OutPoint {
                    txid: root.node_tx.compute_txid(),
                    vout: 0,
                },
                2,
            ),
            0,
        );
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let observed = vec![
            Observation {
                query: ChainQuery::Outspend(deposit),
                result: ChainResult::Unavailable,
            },
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert!(
            interp.pending.is_empty(),
            "an unavailable lookup is not retried"
        );
        assert!(interp.unverified.contains(&id("root")));
        assert!(!interp.resolved.nodes.contains_key(&id("root")));
    }

    #[test]
    fn interpret_falls_back_to_onchain_status() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root_tx = tx_spending(deposit, 1);
        let root_txid = root_tx.compute_txid();
        let mut root = treenode("root", None, root_tx, 0);
        root.status = TreeNodeStatus::OnChain;

        let leaf_parent_out = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf_cpfp = tx_spending(leaf_parent_out, 2);
        let leaf_cpfp_txid = leaf_cpfp.compute_txid();
        let leaf = treenode("leaf", Some("root"), leaf_cpfp, 0);
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let observed = vec![
            Observation {
                query: ChainQuery::Outspend(deposit),
                result: ChainResult::Unavailable,
            },
            spent(leaf_parent_out, leaf_cpfp_txid),
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert!(interp.pending.is_empty());
        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp { change: None })
        );
        assert_eq!(
            interp.resolved.nodes.get(&leaf_id),
            Some(&NodeState::ConfirmedCpfp { change: None })
        );
    }

    #[test]
    fn interpret_flags_driven_child_below_operator_confirmed_node() {
        // The root's chain lookup is unavailable, so its confirmation rests on the
        // operators' OnChain flag: spent_funding can't see the spend, so the leaf
        // driven below it is flagged unverified rather than broadcast.
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let mut root = treenode("root", None, tx_spending(deposit, 1), 0);
        root.status = TreeNodeStatus::OnChain;
        let root_txid = root.node_tx.compute_txid();
        let leaf_parent = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf = treenode("leaf", Some("root"), tx_spending(leaf_parent, 2), 0);
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let observed = vec![
            Observation {
                query: ChainQuery::Outspend(deposit),
                result: ChainResult::Unavailable,
            },
            Observation {
                query: ChainQuery::Outspend(leaf_parent),
                result: ChainResult::Spend(None),
            },
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp { change: None }),
            "root confirmed via the operator fallback"
        );
        assert!(
            !interp.resolved.nodes.contains_key(&leaf_id),
            "the leaf is the driven frontier"
        );
        assert!(
            interp.unverified.contains(&leaf_id),
            "a driven child below an operator-confirmed node is flagged unverified"
        );
    }

    #[test]
    fn interpret_does_not_flag_chain_verified_unresolved_change() {
        // The root is confirmed on-chain (spend visible) but carries no anchor, so
        // its CPFP change can't be resolved. spent_funding still protects any reused
        // input, so the leaf driven below stays unconfirmed rather than flagged.
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root = treenode("root", None, tx_spending(deposit, 1), 0);
        let root_txid = root.node_tx.compute_txid();
        let leaf_parent = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf = treenode("leaf", Some("root"), tx_spending(leaf_parent, 2), 0);
        let leaf_id = leaf.id.clone();
        let prepared = prepared_of(root, leaf);

        let observed = vec![
            spent(deposit, root_txid),
            Observation {
                query: ChainQuery::Outspend(leaf_parent),
                result: ChainResult::Spend(None),
            },
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp { change: None }),
            "root is chain-confirmed but its change is unresolved"
        );
        assert!(
            !interp.unverified.contains(&leaf_id),
            "a chain-verified confirmation does not flag the driven child"
        );
    }

    #[test]
    fn interpret_resolves_confirmed_node_change() {
        let anchor = ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]);
        let funding_script = leaf_addr().script_pubkey();
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::from_height(1).unwrap(),
            input: vec![TxIn {
                previous_output: deposit,
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                ..Default::default()
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(99_000),
                    script_pubkey: ScriptBuf::new(),
                },
                TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: anchor,
                },
            ],
        };
        let root_txid = root_tx.compute_txid();
        let root = treenode("root", None, root_tx, 0);
        let leaf_parent = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf = treenode("leaf", Some("root"), tx_spending(leaf_parent, 2), 0);
        let leaf_id = leaf.id.clone();

        let child_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::from_height(3).unwrap(),
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: root_txid,
                    vout: 1,
                },
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(88_000),
                script_pubkey: funding_script.clone(),
            }],
        };
        let child_txid = child_tx.compute_txid();

        let funding_input = CpfpInput {
            outpoint: OutPoint {
                txid: Txid::from_byte_array([7u8; 32]),
                vout: 0,
            },
            witness_utxo: TxOut {
                value: Amount::from_sat(100_000),
                script_pubkey: funding_script,
            },
            signed_input_weight: 272,
        };
        let prepared = PreparedUnilateralExit {
            plan: UnilateralExitPlan {
                funding_shape: CpfpFundingShape::PerBranch,
                per_node_funding: vec![],
                selected_leaves: vec![],
                fan_out_psbt: None,
                per_branch_funding: vec![(leaf_id.clone(), vec![funding_input])],
                tree_nodes: to_node_map(vec![root, leaf]),
            },
            leaf_refund_addresses: [(leaf_id.clone(), leaf_addr())].into_iter().collect(),
        };

        let observed = vec![
            spent(deposit, root_txid),
            Observation {
                query: ChainQuery::Outspend(leaf_parent),
                result: ChainResult::Spend(None),
            },
            spent(
                OutPoint {
                    txid: root_txid,
                    vout: 1,
                },
                child_txid,
            ),
            Observation {
                query: ChainQuery::Transaction(child_txid),
                result: ChainResult::Transaction(child_tx),
            },
            Observation {
                query: ChainQuery::Outspend(OutPoint {
                    txid: Txid::from_byte_array([7u8; 32]),
                    vout: 0,
                }),
                result: ChainResult::Spend(None),
            },
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert!(interp.pending.is_empty(), "all lookups observed");
        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp {
                change: Some(ConfirmedOutput {
                    outpoint: OutPoint {
                        txid: child_txid,
                        vout: 0,
                    },
                    value: 88_000,
                }),
            }),
            "the root's on-chain CPFP-child change is resolved from chain"
        );
    }

    #[test]
    fn interpret_flags_branch_when_confirmed_child_body_unavailable() {
        // The root is chain-confirmed and its CPFP child's anchor spend is visible
        // (confirmed child txid known), but the child tx body lookup is Unavailable,
        // so the child's change output can't be resolved. A rebuild would reuse the
        // supplied input the confirmed child already spent, so the branch is flagged
        // unverified rather than emitted unconfirmed.
        let anchor = ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]);
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::from_height(1).unwrap(),
            input: vec![TxIn {
                previous_output: deposit,
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                ..Default::default()
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(99_000),
                    script_pubkey: ScriptBuf::new(),
                },
                TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: anchor,
                },
            ],
        };
        let root_txid = root_tx.compute_txid();
        let root = treenode("root", None, root_tx, 0);
        let leaf_parent = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let leaf = treenode("leaf", Some("root"), tx_spending(leaf_parent, 2), 0);
        let leaf_id = leaf.id.clone();

        // The child body is never observed, so only its txid is needed here.
        let child_txid = Txid::from_byte_array([9u8; 32]);
        let funding_input = CpfpInput {
            outpoint: OutPoint {
                txid: Txid::from_byte_array([7u8; 32]),
                vout: 0,
            },
            witness_utxo: TxOut {
                value: Amount::from_sat(100_000),
                script_pubkey: leaf_addr().script_pubkey(),
            },
            signed_input_weight: 272,
        };
        let prepared = PreparedUnilateralExit {
            plan: UnilateralExitPlan {
                funding_shape: CpfpFundingShape::PerBranch,
                per_node_funding: vec![],
                selected_leaves: vec![],
                fan_out_psbt: None,
                per_branch_funding: vec![(leaf_id.clone(), vec![funding_input])],
                tree_nodes: to_node_map(vec![root, leaf]),
            },
            leaf_refund_addresses: [(leaf_id.clone(), leaf_addr())].into_iter().collect(),
        };

        let observed = vec![
            spent(deposit, root_txid),
            Observation {
                query: ChainQuery::Outspend(leaf_parent),
                result: ChainResult::Spend(None),
            },
            spent(
                OutPoint {
                    txid: root_txid,
                    vout: 1,
                },
                child_txid,
            ),
            Observation {
                query: ChainQuery::Transaction(child_txid),
                result: ChainResult::Unavailable,
            },
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed).unwrap();

        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp { change: None }),
            "root is confirmed but its child's change is unresolved"
        );
        assert!(
            interp.unverified.contains(&leaf_id),
            "the driven child below a confirmed node with an unavailable child body is flagged"
        );
    }
}
