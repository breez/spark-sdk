use std::collections::{HashMap, HashSet};

use bitcoin::{Address, Amount, OutPoint, ScriptBuf, Transaction, TxOut, Txid};
use spark::{
    services::{
        CpfpInput, ServiceError, UnilateralExitPlan, branch_required_funding, build_cpfp_child,
        csv_timelock, walk_unilateral_exit_chain,
    },
    tree::{TreeNode, TreeNodeId, TreeNodeStatus},
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

/// An already-confirmed fan-out adopted in place of building a fresh one.
#[derive(Clone, Debug)]
pub(crate) struct ConfirmedFanOut {
    pub tx: Transaction,
    pub branch_outputs: HashMap<TreeNodeId, ConfirmedOutput>,
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

fn result_for<'a>(observed: &'a [Observation], query: &ChainQuery) -> Option<&'a ChainResult> {
    observed
        .iter()
        .find(|o| &o.query == query)
        .map(|o| &o.result)
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
    let node_map: HashMap<TreeNodeId, TreeNode> = plan
        .tree_nodes
        .iter()
        .map(|n| (n.id.clone(), n.clone()))
        .collect();

    let mut pending: Vec<ChainQuery> = Vec::new();
    let mut unverified: HashSet<TreeNodeId> = HashSet::new();

    let (fan_out, fan_out_unverified) = interpret_fan_out(plan, observed, &mut pending)?;

    let mut nodes: HashMap<TreeNodeId, NodeState> = HashMap::new();
    let mut refunds: HashMap<TreeNodeId, RefundState> = HashMap::new();
    let mut stopped: HashSet<TreeNodeId> = HashSet::new();
    let mut needs_change: HashSet<TreeNodeId> = HashSet::new();
    let mut operator_confirmed: HashSet<TreeNodeId> = HashSet::new();
    for (leaf_id, _) in &plan.per_branch_funding {
        walk_branch(
            &node_map,
            leaf_id,
            observed,
            &mut nodes,
            &mut refunds,
            &mut stopped,
            &mut needs_change,
            &mut unverified,
            &mut operator_confirmed,
            &mut pending,
        );
    }

    resolve_confirmed_changes(
        &node_map,
        plan,
        &mut nodes,
        &needs_change,
        observed,
        &mut pending,
    );

    flag_unverifiable_confirmation_branches(&node_map, plan, &operator_confirmed, &mut unverified);

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
            if !branch_has_tracked_change(&node_map, leaf_id, &nodes) {
                continue;
            }
            for input in funding {
                let query = ChainQuery::Outspend(input.outpoint);
                match result_for(observed, &query) {
                    Some(ChainResult::Spend(Some(info))) if info.confirmed => {
                        spent_funding.insert(input.outpoint);
                    }
                    None => pending.push(query),
                    _ => {}
                }
            }
        }
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

/// Marks a branch's driven txs unverified when one of its nodes was confirmed via
/// the operator-OnChain fallback (the chain lookup was unavailable). That spend is
/// invisible to `spent_funding`, so a re-supplied input the confirmed child already
/// spent wouldn't be dropped and the next driven child would double-spend; flagging
/// (not the `Unconfirmed` the build would otherwise emit) tells the caller not to
/// broadcast until a later run confirms it on a healthy chain. Chain-verified
/// confirmations are left alone: their spend is visible, so `spent_funding` drops
/// any reused input. Only `Unconfirmed` txs are upgraded, so confirmed nodes keep
/// their state.
fn flag_unverifiable_confirmation_branches(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    plan: &UnilateralExitPlan,
    operator_confirmed: &HashSet<TreeNodeId>,
    unverified: &mut HashSet<TreeNodeId>,
) {
    for (leaf_id, _) in &plan.per_branch_funding {
        let Some(leaf) = node_map.get(leaf_id) else {
            continue;
        };
        let Ok(chain) = walk_unilateral_exit_chain(node_map, leaf) else {
            continue;
        };
        if chain.iter().any(|n| operator_confirmed.contains(&n.id)) {
            for n in &chain {
                unverified.insert(n.id.clone());
            }
        }
    }
}

/// Resolves each `needs_change` node's on-chain CPFP-child change (the output
/// paying its funding script), driving the two lookups through `pending`; an
/// unresolved lookup leaves the change `None`.
fn resolve_confirmed_changes(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    plan: &UnilateralExitPlan,
    nodes: &mut HashMap<TreeNodeId, NodeState>,
    needs_change: &HashSet<TreeNodeId>,
    observed: &[Observation],
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
        let Some(spend) = result_for(observed, &spend_query) else {
            pending.push(spend_query);
            continue;
        };
        let child_txid = match spend {
            ChainResult::Spend(Some(info)) if info.confirmed => info.spender_txid,
            _ => continue,
        };
        let tx_query = ChainQuery::Transaction(child_txid);
        let Some(tx_result) = result_for(observed, &tx_query) else {
            pending.push(tx_query);
            continue;
        };
        let ChainResult::Transaction(child_tx) = tx_result else {
            continue;
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
    observed: &[Observation],
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
    let branch_leaf_ids: Vec<TreeNodeId> = plan
        .per_branch_funding
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    let conflict = || {
        SparkWalletError::ServiceError(ServiceError::FundingUtxoConflict {
            txid: funding_outpoint.txid.to_string(),
            vout: funding_outpoint.vout,
        })
    };

    let spend_query = ChainQuery::Outspend(funding_outpoint);
    let Some(result) = result_for(observed, &spend_query) else {
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
    let Some(result) = result_for(observed, &tx_query) else {
        pending.push(tx_query);
        return Ok((None, false));
    };
    let tx = match result {
        ChainResult::Transaction(tx) => tx.clone(),
        ChainResult::Unavailable => return Ok((None, true)),
        _ => return Ok((None, false)),
    };
    // Per-branch outputs pay the funding script in branch order (an optional change
    // output pays it too, last); take one per branch.
    let branch_outputs: HashMap<TreeNodeId, ConfirmedOutput> = tx
        .output
        .iter()
        .enumerate()
        .filter(|(_, o)| o.script_pubkey == funding_script)
        .filter_map(|(vout, o)| u32::try_from(vout).ok().map(|v| (v, o.value.to_sat())))
        .zip(branch_leaf_ids.iter())
        .map(|((vout, value), leaf_id)| {
            (
                leaf_id.clone(),
                ConfirmedOutput {
                    outpoint: OutPoint {
                        txid: spender,
                        vout,
                    },
                    value,
                },
            )
        })
        .collect();
    if branch_outputs.len() < branch_leaf_ids.len() {
        return Err(conflict());
    }
    Ok((Some(ConfirmedFanOut { tx, branch_outputs }), false))
}

/// Follows the confirmed spender from the deposit down one branch, classifying
/// each node into `nodes`/`refunds`. Stops (emitting the next lookup into
/// `pending`) at the first output whose spender is not yet observed.
#[allow(clippy::too_many_arguments)]
fn walk_branch(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    leaf_id: &TreeNodeId,
    observed: &[Observation],
    nodes: &mut HashMap<TreeNodeId, NodeState>,
    refunds: &mut HashMap<TreeNodeId, RefundState>,
    stopped: &mut HashSet<TreeNodeId>,
    // Confirmed cpfp nodes whose CPFP change is resolved afterwards.
    needs_change: &mut HashSet<TreeNodeId>,
    unverified: &mut HashSet<TreeNodeId>,
    // Nodes confirmed via the operator-OnChain fallback (chain lookup unavailable),
    // whose on-chain spend `spent_funding` therefore can't see.
    operator_confirmed: &mut HashSet<TreeNodeId>,
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
        let Some(result) = result_for(observed, &query) else {
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
                    operator_confirmed.insert(node.id.clone());
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
    observed: &[Observation],
    refunds: &mut HashMap<TreeNodeId, RefundState>,
    unverified: &mut HashSet<TreeNodeId>,
    pending: &mut Vec<ChainQuery>,
) {
    let scan_query = ChainQuery::RefundAddress {
        leaf_id: leaf_id.clone(),
        address: address.clone(),
    };
    let Some(result) = result_for(observed, &scan_query) else {
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
    let Some(spend) = result_for(observed, &outspend_query) else {
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
    let Some(result) = result_for(observed, &tx_query) else {
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
    let node_map: HashMap<TreeNodeId, TreeNode> = plan
        .tree_nodes
        .iter()
        .map(|n| (n.id.clone(), n.clone()))
        .collect();

    let (fan_out, per_branch_funding) = resolve_fan_out_funding(plan, resolved)?;
    let fan_out_txid = fan_out.as_ref().map(|f| f.txid);

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
        let chain = walk_unilateral_exit_chain(&node_map, leaf).map_err(|missing| {
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
                    if first_in_branch && let Some(fo) = fan_out_txid {
                        depends_on.push(fo);
                    }

                    let to_sign = match node_state {
                        Some(NodeState::ConfirmedCpfp { change: Some(c) }) => {
                            if let (Some(script), Some(weight)) =
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
                            let child =
                                build_cpfp_child(&node.node_tx, &funding, fee_rate_sat_per_kw)?;
                            cpfp_fee_sat = cpfp_fee_sat.saturating_add(child.fee_sat);
                            funding = vec![child.change_input];
                            Some(child.psbt)
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
                let child = build_cpfp_child(&refund_tx, &funding, fee_rate_sat_per_kw)?;
                cpfp_fee_sat = cpfp_fee_sat.saturating_add(child.fee_sat);
                refund_outputs.push(RefundOutput {
                    outpoint: OutPoint {
                        txid: refund_txid,
                        vout: 0,
                    },
                    leaf_id: leaf_id.clone(),
                    value: refund_value,
                });
                // The refund child's change is the branch's terminal sweep input.
                cpfp_change_inputs.push(CpfpChangeInput {
                    outpoint: child.change_input.outpoint,
                    witness_utxo: child.change_input.witness_utxo.clone(),
                    signed_input_weight: child.change_input.signed_input_weight,
                });
                txs.push(ExitTx {
                    kind: ExitTxKind::Refund,
                    node_id: Some(leaf_id.clone()),
                    txid: refund_txid,
                    base_tx: refund_tx,
                    to_sign: Some(child.psbt),
                    csv_timelock_blocks: refund_csv,
                    depends_on: leaf_node_txid.into_iter().collect(),
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
    // built with, so it must still cover the branch cost plus terminal change dust.
    let leaf_by_id: HashMap<&TreeNodeId, _> =
        plan.selected_leaves.iter().map(|l| (&l.id, l)).collect();
    let mut per_branch = plan.per_branch_funding.clone();
    for (leaf_id, funding) in &mut per_branch {
        let adopted = confirmed.branch_outputs.get(leaf_id).ok_or_else(|| {
            SparkWalletError::Generic(format!(
                "adopted fan-out is missing an output for branch {leaf_id}"
            ))
        })?;
        // The fan-out funds each branch with exactly one output.
        let Some(first) = funding.first_mut() else {
            continue;
        };
        // Dust from the branch's own funding script, not the plan's change_dust_limit.
        let dust = first.witness_utxo.script_pubkey.minimal_non_dust().to_sat();
        let required = leaf_by_id
            .get(leaf_id)
            .map_or(dust, |leaf| branch_required_funding(leaf, dust));
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
            selected_leaves: vec![UnilateralExitSelectedLeaf {
                id: leaf.id.clone(),
                value: 100_000,
                estimated_cost: 2_000,
                cpfp_cost: 2_000,
            }],
            fan_out_psbt: None,
            per_branch_funding: vec![(leaf.id.clone(), vec![funding(100_000)])],
            tree_nodes: vec![root, leaf],
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
            selected_leaves: vec![UnilateralExitSelectedLeaf {
                id: leaf.id.clone(),
                value: 100_000,
                estimated_cost: 2_000,
                cpfp_cost: 2_000,
            }],
            fan_out_psbt: Some(fan_out_psbt),
            per_branch_funding: vec![(leaf.id.clone(), vec![branch_output.clone()])],
            tree_nodes: vec![root, leaf],
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
            tree_nodes: vec![root, mid, leaf_a, leaf_b],
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
            tree_nodes: vec![root, mid, leaf_a, leaf_b],
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
                selected_leaves: vec![],
                fan_out_psbt: None,
                per_branch_funding: vec![(leaf_id.clone(), vec![])],
                tree_nodes: vec![root, leaf],
            },
            leaf_refund_addresses: [(leaf_id, leaf_addr())].into_iter().collect(),
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
                selected_leaves: vec![],
                fan_out_psbt: Some(fan_out_psbt),
                per_branch_funding: vec![(id("a"), vec![]), (id("b"), vec![])],
                tree_nodes: vec![],
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
                selected_leaves: vec![],
                fan_out_psbt: None,
                per_branch_funding: vec![(leaf_id.clone(), vec![funding_input])],
                tree_nodes: vec![root, leaf],
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
}
