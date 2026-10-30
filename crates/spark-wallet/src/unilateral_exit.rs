use std::collections::{HashMap, HashSet};

use bitcoin::{Address, Amount, FeeRate, OutPoint, Transaction, Txid};
use spark::{
    services::{
        ConfirmedExitNode, CpfpInput, ExitChainState, ExitNodeConfirmation, ExitRefund,
        ExitRefundState, UnilateralExitPlan, build_anchorless_refund_child, build_cpfp_child,
        csv_timelock, p2tr_key_path_input_weight, walk_unilateral_exit_chain,
    },
    signer::LeafSigningKey,
    tree::{LeafPedigree, TreeNode, TreeNodeId, TreeNodeStatus},
    utils::transactions::has_anchor,
};
use tracing::{debug, trace, warn};

use crate::SparkWalletError;

/// Which leaves to unilaterally exit.
#[derive(Clone, Debug)]
pub enum ExitLeafSelection {
    /// Exit every stored leaf that is profitable to exit.
    Auto,
    /// Exit the leaves in one of the [`EXITING_STATUSES`] that are profitable to
    /// exit.
    Exiting,
    /// Exit exactly these leaves, regardless of profitability.
    Specific(Vec<TreeNodeId>),
}

/// The statuses of a leaf whose exit started on-chain.
pub const EXITING_STATUSES: [TreeNodeStatus; 3] = [
    TreeNodeStatus::OnChain,
    TreeNodeStatus::Exited,
    TreeNodeStatus::ParentExited,
];

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

/// The exit's on-chain state; empty drives a fresh cpfp exit.
///
/// The pre-signed txs only continue along the cpfp `node_tx` chain (every child
/// and the cpfp refund spend the parent's `node_tx` output), so a node taken
/// on-chain by any non-cpfp tx cannot be continued.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedExitState {
    /// A node absent from the map is driven: emit its `node_tx` with a fresh child.
    pub nodes: HashMap<TreeNodeId, NodeState>,
    /// A leaf absent from the map has its cpfp `refund_tx` driven fresh.
    pub refunds: HashMap<TreeNodeId, RefundState>,
    /// Leaves whose cpfp lineage was taken on-chain by an uncontinuable tx; the
    /// branch drives nothing and is absent from the built set.
    pub stopped: HashSet<TreeNodeId>,
}

/// How a node was resolved on-chain (absent = driven via cpfp).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeState {
    /// Confirmed via the cpfp `node_tx`, its fee paid by a child. What that child
    /// left over is funding like any other, and reaches the build as an input.
    ConfirmedCpfp { block_height: Option<u32> },
    /// Confirmed via the self-fee `direct_tx`. Only ever a leaf: an intermediate's
    /// children spend its cpfp output, which a direct spend never creates.
    ConfirmedDirect { block_height: Option<u32> },
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

/// A leaf refund already on-chain (any variant), adopted for the sweep.
#[derive(Clone, Debug)]
pub(crate) struct ConfirmedRefund {
    pub tx: Transaction,
    pub outpoint: bitcoin::OutPoint,
    pub value: u64,
    pub block_height: Option<u32>,
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
    /// For a refund without a P2A anchor, the CPFP child that pays its fee out
    /// of the refund's own output. The wallet signs it with the leaf's key, so
    /// the caller signs nothing for it.
    pub cpfp_tx: Option<Transaction>,
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
    /// On-chain and confirmed (or an adopted, already-confirmed output), in the
    /// block at this height where that is known.
    Confirmed {
        block_height: Option<u32>,
    },
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
    /// CPFP-package fees of the txs built fresh, paid by the funding UTXOs.
    pub cpfp_fee_sat: u64,
    /// A freshly-broadcast fan-out's fee, paid by the funding UTXO. Zero when
    /// there is no fan-out or it was adopted already-confirmed.
    pub fanout_fee_sat: u64,
    /// The fees paid out of the leaves' value before the sweep: the value of
    /// each leaf with an output in `refund_outputs`, less the value of that
    /// output. A direct transaction's output is its input less its fee.
    pub refund_fee_sat: u64,
}

/// A refund output sitting on-chain after a unilateral exit.
#[derive(Clone, Debug)]
pub struct RefundOutput {
    pub outpoint: bitcoin::OutPoint,
    pub leaf_id: TreeNodeId,
    pub value: u64,
    /// The key the leaf was held under, which the refund pays to.
    pub signing_key: LeafSigningKey,
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
    /// Is this transaction in a block? A "no" does not say what took its place;
    /// [`ChainQuery::Outspend`] on one of its inputs answers that.
    TxConfirmed(Txid),
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
    /// Whether the transaction is in a block, and which one. The height is what
    /// a relative timelock counts from, so a caller that has it need not fetch
    /// the transaction again to work out when a child of it can go out.
    Confirmed {
        confirmed: bool,
        block_height: Option<u32>,
    },
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
    /// The block the spender is in, when it is in one.
    pub block_height: Option<u32>,
}

/// An output found at a refund address, spent or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressUtxo {
    pub txid: Txid,
    pub vout: u32,
    pub value: u64,
    pub confirmed: bool,
    /// The block it is in, when it is in one.
    pub block_height: Option<u32>,
}

/// A performed [`ChainQuery`] paired with its [`ChainResult`].
#[derive(Clone, Debug)]
pub struct Observation {
    pub query: ChainQuery,
    pub result: ChainResult,
}

/// The outcome of interpreting the observations: resolved state plus the lookups
/// still needed while the walk is incomplete.
struct ChainInterpretation {
    resolved: ResolvedExitState,
    unverified: HashSet<TreeNodeId>,
}

/// An [`ExitChainState`] and the lookups still needed to complete it.
pub struct ExitChainScan {
    pub state: ExitChainState,
    pub pending: Vec<ChainQuery>,
    /// The sweep of each refund that the results show swept, sorted by leaf id.
    pub sweeps: Vec<RefundSweep>,
}

/// The sweep of a leaf's refund, in a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefundSweep {
    pub leaf_id: TreeNodeId,
    pub txid: Txid,
    /// Unset when the chain service did not report the block.
    pub block_height: Option<u32>,
}

/// Whether a sweep of a leaf's refund is in a block, read from the results of
/// a scan's queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefundSweepLookup {
    /// A query the lookup needs has no result.
    Pending,
    /// A sweep of the refund is in a block.
    Swept(RefundSweep),
    /// The results show no sweep in a block: the refund is unspent, its sweep
    /// is in no block, the address has no refund in a block, or its listing
    /// contradicts itself.
    NotSwept,
}

pub struct RefundSweepScan {
    pub lookups: HashMap<TreeNodeId, RefundSweepLookup>,
    /// The queries the caller has yet to execute.
    pub pending: Vec<ChainQuery>,
}

/// The exit's on-chain state as the tree alone shows it: which nodes are
/// confirmed, which refunds landed, which branches can no longer be continued.
/// Answered without any funding, so it can be resolved before an exit is funded.
#[derive(Default)]
struct ExitChainWalk {
    nodes: HashMap<TreeNodeId, NodeState>,
    refunds: HashMap<TreeNodeId, RefundState>,
    sweeps: HashMap<TreeNodeId, (Txid, Option<u32>)>,
    stopped: HashSet<TreeNodeId>,
    unverified: HashSet<TreeNodeId>,
    /// Confirmed nodes whose on-chain spend `scan_funding` cannot see: the
    /// operator-OnChain fallback taken because the chain lookup was unavailable.
    unverifiable_confirmed: HashSet<TreeNodeId>,
    pending: Vec<ChainQuery>,
}

/// An index over the observations for O(1) lookup by query, built once per
/// [`interpret_chain`] pass instead of scanning the growing observation list on
/// every lookup. The first observation of a query wins, matching the prior scan.
pub(crate) struct ObservedIndex<'a> {
    by_query: HashMap<&'a ChainQuery, &'a ChainResult>,
}

impl<'a> ObservedIndex<'a> {
    pub(crate) fn new(observed: &'a [Observation]) -> Self {
        let mut by_query = HashMap::with_capacity(observed.len());
        for obs in observed {
            by_query.entry(&obs.query).or_insert(&obs.result);
        }
        Self { by_query }
    }

    pub(crate) fn get(&self, query: &ChainQuery) -> Option<&'a ChainResult> {
        self.by_query.get(query).copied()
    }
}

/// The funding an exit can spend, and the lookups still needed to know it.
pub struct FundingScan {
    pub inputs: Vec<CpfpInput>,
    pub pending: Vec<ChainQuery>,
}

/// Resolves what the supplied funding is worth now. A UTXO a prior run already
/// spent is replaced by what that spend produced, so a caller can hand back the
/// outpoint it was given and still be funded from the fan-out outputs or CPFP
/// change it turned into.
///
/// Only a descendant paying the same script is followed: it came from money the
/// caller offered and pays a script the caller controls, which is what makes it
/// theirs to spend. Anything the spend paid elsewhere is somebody else's.
///
/// Drive it like the other scans: perform `pending`, feed the results back, and
/// repeat until nothing is pending. An outpoint whose spend cannot be read is
/// kept, not being known to be spent; one known spent by a transaction that
/// cannot be read is dropped, since building over it would double-spend.
pub fn scan_funding(supplied: &[CpfpInput], observed: &[Observation]) -> FundingScan {
    let index = ObservedIndex::new(observed);
    let mut inputs: HashMap<OutPoint, CpfpInput> = HashMap::new();
    let mut pending: Vec<ChainQuery> = Vec::new();
    let mut visited: HashSet<OutPoint> = HashSet::new();
    let mut frontier: Vec<CpfpInput> = supplied.to_vec();

    while let Some(input) = frontier.pop() {
        if !visited.insert(input.outpoint) {
            continue;
        }
        let query = ChainQuery::Outspend(input.outpoint);
        let Some(result) = index.get(&query) else {
            pending.push(query);
            inputs.insert(input.outpoint, input);
            continue;
        };
        let spender = match result {
            // Spent only by an unconfirmed transaction: our own child, which a
            // rebuild replaces, so the outpoint is still ours to spend.
            ChainResult::Spend(Some(info)) if info.confirmed => info.spender_txid,
            _ => {
                inputs.insert(input.outpoint, input);
                continue;
            }
        };

        let tx_query = ChainQuery::Transaction(spender);
        let Some(ChainResult::Transaction(tx)) = index.get(&tx_query) else {
            if index.get(&tx_query).is_none() {
                pending.push(tx_query);
            }
            continue;
        };
        for (vout, out) in tx.output.iter().enumerate() {
            if out.script_pubkey != input.witness_utxo.script_pubkey {
                continue;
            }
            let Ok(vout) = u32::try_from(vout) else {
                continue;
            };
            frontier.push(CpfpInput {
                outpoint: OutPoint {
                    txid: spender,
                    vout,
                },
                witness_utxo: out.clone(),
                // The same script signs the same way, so the bound the caller
                // gave for its own UTXO holds for what it turned into.
                signed_input_weight: input.signed_input_weight,
            });
        }
    }

    let mut inputs: Vec<CpfpInput> = inputs.into_values().collect();
    inputs.sort_by_key(|input| (input.outpoint.txid, input.outpoint.vout));

    let mut seen: HashSet<ChainQuery> = HashSet::new();
    pending.retain(|query| seen.insert(query.clone()));

    FundingScan { inputs, pending }
}

/// One transaction of an exit a caller kept, as the check reads it back.
pub struct ExitCheckInput {
    pub tx: Transaction,
    /// The CPFP child that pays its fee, where it has one.
    pub cpfp: Option<Transaction>,
    /// Whether the caller last recorded it in a block. Where the check starts,
    /// not what it concludes.
    pub confirmed: bool,
}

/// What the chain says about an exit a caller kept.
pub struct ExitCheck {
    /// The transactions now in a block, against the height of that block where
    /// the chain reported one.
    pub confirmed: HashMap<Txid, Option<u32>>,
    /// Whether the chain no longer matches: something other than this exit's own
    /// transaction took an outpoint it still needs.
    pub diverged: bool,
    /// Transactions this check asked the chain about, where the chain replied
    /// that they are not in a block. A txid in neither this nor `confirmed` was
    /// either not asked about or its lookup failed, and neither of those says
    /// anything about where the transaction is.
    pub not_confirmed: HashSet<Txid>,
    /// Transactions in the caller's list that still have to confirm, where the
    /// chain service failed a query for an outpoint that the transaction or its
    /// CPFP child spends.
    pub unverified: HashSet<Txid>,
    pub pending: Vec<ChainQuery>,
}

/// A transaction in the caller's list, or the CPFP child of one.
struct ExitTransaction<'a> {
    txid: Txid,
    tx: &'a Transaction,
    /// Set for a CPFP child: its parent in the caller's list.
    cpfp_parent: Option<Txid>,
    /// Whether the caller last recorded it in a block. The caller records no
    /// CPFP child.
    confirmed: bool,
}

/// The transactions in the caller's list, in its order, each followed by its
/// CPFP child.
fn exit_transactions(txs: &[ExitCheckInput]) -> Vec<ExitTransaction<'_>> {
    let mut transactions = Vec::with_capacity(txs.len().saturating_mul(2));
    for input in txs {
        let txid = input.tx.compute_txid();
        transactions.push(ExitTransaction {
            txid,
            tx: &input.tx,
            cpfp_parent: None,
            confirmed: input.confirmed,
        });
        if let Some(cpfp) = &input.cpfp {
            transactions.push(ExitTransaction {
                txid: cpfp.compute_txid(),
                tx: cpfp,
                cpfp_parent: Some(txid),
                confirmed: false,
            });
        }
    }
    transactions
}

/// Reads back an exit a caller kept, saying which of its transactions are in a
/// block and whether the chain still matches it. Drive it like the other scans:
/// perform `pending`, feed the results back, and repeat until nothing is
/// pending.
///
/// A CPFP child is a transaction of the exit like the ones in the caller's
/// list. For each transaction that still has to confirm, the check reads the
/// spend of every outpoint its inputs refer to.
#[must_use]
pub fn check_exit_chain(txs: &[ExitCheckInput], observed: &[Observation]) -> ExitCheck {
    let index = ObservedIndex::new(observed);
    let mut pending: Vec<ChainQuery> = Vec::new();
    let mut confirmed: HashMap<Txid, Option<u32>> = HashMap::new();
    let mut not_confirmed: HashSet<Txid> = HashSet::new();
    let mut unverified: HashSet<Txid> = HashSet::new();
    let mut diverged = false;

    let transactions = exit_transactions(txs);
    let by_txid: HashMap<Txid, &ExitTransaction> = transactions
        .iter()
        .map(|transaction| (transaction.txid, transaction))
        .collect();
    let spenders = spenders(&transactions, &by_txid);
    let mut expected = expected_in_block(&transactions, &by_txid, &spenders);

    // Deepest first. The check queries a transaction it expects in a block
    // only when no other one it expects in a block has an output of it as an
    // input: when that one is in a block, so is this one. It also queries the
    // parent of a CPFP child, for the parent's block height.
    for transaction in transactions.iter().rev() {
        let txid = transaction.txid;
        if !expected.contains(&txid) || confirmed.contains_key(&txid) {
            continue;
        }
        let spender_expected = spenders.get(&txid).into_iter().flatten().any(|spender| {
            expected.contains(spender)
                && by_txid
                    .get(spender)
                    .is_some_and(|spender| spender.cpfp_parent != Some(txid))
        });
        if spender_expected {
            continue;
        }
        let query = ChainQuery::TxConfirmed(txid);
        match index.get(&query) {
            Some(ChainResult::Confirmed {
                confirmed: true,
                block_height,
            }) => mark_in_block(&by_txid, &index, txid, *block_height, &mut confirmed),
            Some(ChainResult::Confirmed {
                confirmed: false, ..
            }) => {
                expected.remove(&txid);
                not_confirmed.insert(txid);
            }
            // After a failed lookup, the check still expects a transaction the
            // caller recorded in a block there. The caller records no CPFP
            // child, so the check reads the inputs of a CPFP child instead.
            Some(_) => {
                if !transaction.confirmed {
                    expected.remove(&txid);
                }
            }
            None => pending.push(query),
        }
    }

    if pending.is_empty() {
        // Forward, so a transaction found in a block here opens the one below it
        // in the same pass: the list is in the order the exit was built.
        for transaction in &transactions {
            let in_block = |txid: &Txid| confirmed.contains_key(txid) || expected.contains(txid);
            if !has_to_confirm(transaction, &by_txid, &spenders, &in_block) {
                continue;
            }
            // The check skips an output of a transaction of the exit that it has
            // not found in a block: until that transaction is in a block, the
            // output does not exist, so it cannot have been spent.
            let outpoints: Vec<OutPoint> = transaction
                .tx
                .input
                .iter()
                .map(|input| input.previous_output)
                .filter(|outpoint| {
                    !by_txid.contains_key(&outpoint.txid) || in_block(&outpoint.txid)
                })
                .collect();
            match outpoint_spends(transaction.txid, &outpoints, &index) {
                OutpointSpends::InBlock { block_height } => {
                    mark_in_block(
                        &by_txid,
                        &index,
                        transaction.txid,
                        block_height,
                        &mut confirmed,
                    );
                }
                OutpointSpends::Taken => diverged = true,
                OutpointSpends::NotSpentInBlock {
                    without_result,
                    unreadable,
                } => {
                    pending.extend(without_result);
                    if unreadable {
                        unverified.insert(transaction.cpfp_parent.unwrap_or(transaction.txid));
                    }
                }
            }
        }
    }

    let mut seen: HashSet<ChainQuery> = HashSet::new();
    pending.retain(|query| seen.insert(query.clone()));

    not_confirmed.retain(|txid| !confirmed.contains_key(txid));
    unverified.retain(|txid| !confirmed.contains_key(txid));

    ExitCheck {
        confirmed,
        diverged,
        not_confirmed,
        unverified,
        pending,
    }
}

/// Whether `transaction` still has to confirm before the exit is finished. A
/// transaction in the caller's list has to until it is in a block. A CPFP child
/// has to while its parent is in no block, or while a transaction that still
/// has to confirm has an output of the child as an input.
fn has_to_confirm(
    transaction: &ExitTransaction,
    by_txid: &HashMap<Txid, &ExitTransaction>,
    spenders: &HashMap<Txid, Vec<Txid>>,
    in_block: &dyn Fn(&Txid) -> bool,
) -> bool {
    if in_block(&transaction.txid) {
        return false;
    }
    let Some(parent) = transaction.cpfp_parent else {
        return true;
    };
    !in_block(&parent)
        || spenders
            .get(&transaction.txid)
            .into_iter()
            .flatten()
            .filter_map(|spender| by_txid.get(spender))
            .any(|spender| has_to_confirm(spender, by_txid, spenders, in_block))
}

/// What the check read about the outpoints of a transaction's inputs. Once one
/// of them is spent in a block, the check needs no other: spent by the
/// transaction itself, the transaction is in a block, and spent by another
/// transaction, the transaction can no longer confirm.
enum OutpointSpends {
    InBlock {
        block_height: Option<u32>,
    },
    Taken,
    NotSpentInBlock {
        without_result: Vec<ChainQuery>,
        /// Whether the chain service failed the query for one of them.
        unreadable: bool,
    },
}

fn outpoint_spends(
    txid: Txid,
    outpoints: &[OutPoint],
    index: &ObservedIndex<'_>,
) -> OutpointSpends {
    let mut without_result = Vec::new();
    let mut unreadable = false;
    for outpoint in outpoints {
        let query = ChainQuery::Outspend(*outpoint);
        let info = match index.get(&query) {
            Some(ChainResult::Spend(Some(info))) if info.confirmed => info,
            Some(ChainResult::Spend(_)) => continue,
            Some(_) => {
                unreadable = true;
                continue;
            }
            None => {
                without_result.push(query);
                continue;
            }
        };
        if info.spender_txid == txid {
            return OutpointSpends::InBlock {
                block_height: info.block_height,
            };
        }
        warn!(%outpoint, spender = %info.spender_txid, "check: outpoint taken");
        return OutpointSpends::Taken;
    }
    OutpointSpends::NotSpentInBlock {
        without_result,
        unreadable,
    }
}

/// For each transaction of the exit, the transactions of the exit with one of
/// its outputs as an input.
fn spenders(
    transactions: &[ExitTransaction],
    by_txid: &HashMap<Txid, &ExitTransaction>,
) -> HashMap<Txid, Vec<Txid>> {
    let mut spenders: HashMap<Txid, Vec<Txid>> = HashMap::new();
    for transaction in transactions {
        for parent in spend_parents(transaction, by_txid) {
            spenders.entry(parent).or_default().push(transaction.txid);
        }
    }
    spenders
}

/// The transactions the check expects in a block: the ones the caller recorded
/// there, every transaction of the exit with an output among their inputs,
/// directly or through others, and the CPFP child of a recorded transaction
/// that, by that record, still has to confirm. The caller records no CPFP
/// child.
fn expected_in_block(
    transactions: &[ExitTransaction],
    by_txid: &HashMap<Txid, &ExitTransaction>,
    spenders: &HashMap<Txid, Vec<Txid>>,
) -> HashSet<Txid> {
    let recorded: HashSet<Txid> = transactions
        .iter()
        .filter(|transaction| transaction.confirmed)
        .map(|transaction| transaction.txid)
        .collect();
    let in_record = |txid: &Txid| recorded.contains(txid);
    let children: Vec<Txid> = transactions
        .iter()
        .filter(|transaction| {
            transaction
                .cpfp_parent
                .is_some_and(|parent| recorded.contains(&parent))
                && has_to_confirm(transaction, by_txid, spenders, &in_record)
        })
        .map(|transaction| transaction.txid)
        .collect();
    let mut expected = recorded.clone();
    let mut stack: Vec<Txid> = recorded.into_iter().collect();
    while let Some(txid) = stack.pop() {
        let Some(transaction) = by_txid.get(&txid) else {
            continue;
        };
        for parent in spend_parents(transaction, by_txid) {
            if expected.insert(parent) {
                stack.push(parent);
            }
        }
    }
    expected.extend(children);
    expected
}

/// The transactions of this exit with an output among the inputs of `transaction`.
fn spend_parents<'a>(
    transaction: &'a ExitTransaction,
    by_txid: &'a HashMap<Txid, &ExitTransaction>,
) -> impl Iterator<Item = Txid> + 'a {
    transaction
        .tx
        .input
        .iter()
        .map(|i| i.previous_output.txid)
        .filter(|txid| by_txid.contains_key(txid))
}

/// Records `txid` in a block at `block_height`, and with it every transaction
/// of the exit with an output among its inputs, directly or through others.
/// Each of those gets the height of its own query's result, where the check
/// has one.
fn mark_in_block(
    by_txid: &HashMap<Txid, &ExitTransaction>,
    index: &ObservedIndex<'_>,
    txid: Txid,
    block_height: Option<u32>,
    confirmed: &mut HashMap<Txid, Option<u32>>,
) {
    confirmed.insert(txid, block_height);
    let mut stack: Vec<Txid> = by_txid
        .get(&txid)
        .map(|transaction| spend_parents(transaction, by_txid).collect())
        .unwrap_or_default();
    while let Some(txid) = stack.pop() {
        if confirmed.contains_key(&txid) {
            continue;
        }
        let reported = match index.get(&ChainQuery::TxConfirmed(txid)) {
            Some(ChainResult::Confirmed {
                confirmed: true,
                block_height,
            }) => *block_height,
            _ => None,
        };
        confirmed.insert(txid, reported);
        if let Some(transaction) = by_txid.get(&txid) {
            stack.extend(spend_parents(transaction, by_txid));
        }
    }
}

/// Restores the walk's own reading from an [`ExitChainState`] resolved earlier,
/// so a build needs no chain of its own for the tree.
fn restore_exit_chain_walk(state: &ExitChainState, leaf_ids: &[TreeNodeId]) -> ExitChainWalk {
    let confirmed: HashMap<&TreeNodeId, ExitNodeConfirmation> = state
        .nodes
        .iter()
        .map(|node| (&node.node_id, node.confirmed_by))
        .collect();

    let nodes: HashMap<TreeNodeId, NodeState> = state
        .nodes
        .iter()
        .map(|node| {
            let restored = match node.confirmed_by {
                // Change is funding-dependent, so it is resolved separately, by
                // the caller that holds a funding script.
                ExitNodeConfirmation::Cpfp => NodeState::ConfirmedCpfp {
                    block_height: node.block_height,
                },
                ExitNodeConfirmation::Direct => NodeState::ConfirmedDirect {
                    block_height: node.block_height,
                },
            };
            (node.node_id.clone(), restored)
        })
        .collect();

    let stopped: HashSet<TreeNodeId> = state.stopped_leaves.iter().cloned().collect();

    // A leaf taken on-chain by its direct spend is refunded by the self-fee
    // direct refund, which pays its own fee.
    let mut refunds: HashMap<TreeNodeId, RefundState> = HashMap::new();
    for leaf_id in leaf_ids {
        if !stopped.contains(leaf_id)
            && confirmed.get(leaf_id) == Some(&ExitNodeConfirmation::Direct)
        {
            refunds.insert(leaf_id.clone(), RefundState::DriveDirect);
        }
    }

    // A refund the chain reports overrides what the branch alone implies.
    for refund in &state.refunds {
        let restored = match &refund.state {
            ExitRefundState::OnChain {
                tx,
                vout,
                value,
                block_height,
            } => RefundState::Adopted(ConfirmedRefund {
                outpoint: OutPoint {
                    txid: tx.compute_txid(),
                    vout: *vout,
                },
                tx: tx.clone(),
                value: *value,
                block_height: *block_height,
            }),
            ExitRefundState::Swept => RefundState::Swept,
        };
        refunds.insert(refund.leaf_id.clone(), restored);
    }

    ExitChainWalk {
        nodes,
        refunds,
        sweeps: HashMap::new(),
        stopped,
        unverified: state.unverified_nodes.iter().cloned().collect(),
        unverifiable_confirmed: state.unverifiable_confirmed_nodes.iter().cloned().collect(),
        pending: Vec::new(),
    }
}

/// The address each leaf's refund lands on. Every refund variant pays the leaf's
/// key, and the leaf's own `refund_tx` is built to pay exactly that, so the
/// address is read off the tree rather than derived from the signer.
///
/// A leaf with no `refund_tx` is unexitable and is left out.
pub fn leaf_refund_addresses(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    network: bitcoin::Network,
) -> HashMap<TreeNodeId, Address> {
    let mut addresses = HashMap::new();
    for leaf_id in leaf_ids {
        let address = tree_nodes
            .get(leaf_id)
            .and_then(|node| node.refund_tx.as_ref())
            .and_then(|tx| tx.output.first())
            .and_then(|out| Address::from_script(&out.script_pubkey, network).ok());
        if let Some(address) = address {
            addresses.insert(leaf_id.clone(), address);
        }
    }
    addresses
}

/// The fee of one refund input of a sweep at `fee_rate`. `None` when the fee
/// overflows.
pub fn refund_sweep_input_fee(fee_rate: FeeRate) -> Option<Amount> {
    fee_rate.fee_wu(p2tr_key_path_input_weight())
}

/// Reads what the chain has already done to each leaf's branch, with no funding
/// involved. Drive it like the full walk: perform `pending`, feed the results
/// back, and repeat until nothing is pending.
///
/// A step the lookups could not resolve is left out of the state rather than
/// guessed, so an unreadable chain reports less progress than there is, never
/// more.
pub fn scan_exit_chain(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    refund_addresses: &HashMap<TreeNodeId, Address>,
    observed: &[Observation],
) -> ExitChainScan {
    let index = ObservedIndex::new(observed);
    let walk = walk_exit_chain(tree_nodes, leaf_ids, refund_addresses, &index);

    let mut pending = walk.pending;
    let mut seen: HashSet<ChainQuery> = HashSet::new();
    pending.retain(|query| seen.insert(query.clone()));

    let mut nodes: Vec<ConfirmedExitNode> = walk
        .nodes
        .into_iter()
        .map(|(node_id, state)| ConfirmedExitNode {
            node_id,
            confirmed_by: match state {
                NodeState::ConfirmedCpfp { .. } => ExitNodeConfirmation::Cpfp,
                NodeState::ConfirmedDirect { .. } => ExitNodeConfirmation::Direct,
            },
            block_height: match state {
                NodeState::ConfirmedCpfp { block_height }
                | NodeState::ConfirmedDirect { block_height } => block_height,
            },
        })
        .collect();
    nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));

    // DriveDirect is not carried: a leaf's direct refund is the one to broadcast
    // exactly when its own node is confirmed via the direct spend, which `nodes`
    // already says.
    let mut refunds: Vec<ExitRefund> = walk
        .refunds
        .into_iter()
        .filter_map(|(leaf_id, state)| {
            let state = match state {
                RefundState::Adopted(refund) => ExitRefundState::OnChain {
                    block_height: refund.block_height,
                    tx: refund.tx,
                    vout: refund.outpoint.vout,
                    value: refund.value,
                },
                RefundState::Swept => ExitRefundState::Swept,
                RefundState::DriveDirect => return None,
            };
            Some(ExitRefund { leaf_id, state })
        })
        .collect();
    refunds.sort_by(|a, b| a.leaf_id.cmp(&b.leaf_id));

    let sorted = |ids: HashSet<TreeNodeId>| {
        let mut ids: Vec<TreeNodeId> = ids.into_iter().collect();
        ids.sort();
        ids
    };
    let mut sweeps: Vec<RefundSweep> = walk
        .sweeps
        .into_iter()
        .map(|(leaf_id, (txid, block_height))| RefundSweep {
            leaf_id,
            txid,
            block_height,
        })
        .collect();
    sweeps.sort_by(|a, b| a.leaf_id.cmp(&b.leaf_id));

    ExitChainScan {
        state: ExitChainState {
            nodes,
            refunds,
            stopped_leaves: sorted(walk.stopped),
            unverified_nodes: sorted(walk.unverified),
            unverifiable_confirmed_nodes: sorted(walk.unverifiable_confirmed),
        },
        pending,
        sweeps,
    }
}

/// Looks up whether a sweep of each leaf's refund is in a block, from the
/// leaf's refund address alone. Run it again with the results of `pending`
/// until none is left.
pub fn scan_refund_sweeps(
    refund_addresses: &HashMap<TreeNodeId, Address>,
    observed: &[Observation],
) -> RefundSweepScan {
    let index = ObservedIndex::new(observed);
    let mut pending = Vec::new();
    let lookups = refund_addresses
        .iter()
        .map(|(leaf_id, address)| {
            let lookup = match follow_refund(leaf_id, address, &index, &mut pending) {
                RefundTrail::Pending | RefundTrail::Unavailable => RefundSweepLookup::Pending,
                RefundTrail::Swept { txid, block_height } => {
                    RefundSweepLookup::Swept(RefundSweep {
                        leaf_id: leaf_id.clone(),
                        txid,
                        block_height,
                    })
                }
                RefundTrail::Unreadable | RefundTrail::NotOnChain | RefundTrail::Unspent(_) => {
                    RefundSweepLookup::NotSwept
                }
            };
            (leaf_id.clone(), lookup)
        })
        .collect();
    RefundSweepScan { lookups, pending }
}

/// Walks each leaf's branch and reads back its refund, emitting the lookups still
/// needed. Reads the tree and nothing else: no funding is consulted, so the answer
/// holds whatever an exit is later funded with.
///
/// The walk follows the confirmed spender down each branch (a non-`node_tx` spend
/// breaks it); each leaf's refund is recovered independently by an address scan, so
/// it survives a broken branch.
fn walk_exit_chain(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    refund_addresses: &HashMap<TreeNodeId, Address>,
    observed: &ObservedIndex<'_>,
) -> ExitChainWalk {
    let mut walk = ExitChainWalk::default();

    for leaf_id in leaf_ids {
        walk_branch(node_map, leaf_id, observed, &mut walk);
    }

    // Runs per leaf independently of the walk; an adopted refund overrides it.
    for (leaf_id, address) in refund_addresses {
        interpret_refund(
            leaf_id,
            address,
            observed,
            &mut walk.refunds,
            &mut walk.sweeps,
            &mut walk.unverified,
            &mut walk.pending,
        );
    }

    walk
}

/// Restates the tree's on-chain state, read while preparing, in the terms the
/// build works in: which nodes are confirmed, which refunds are there to sweep,
/// and which branches can no longer be continued. Reads no chain of its own.
fn interpret_chain(plan: &UnilateralExitPlan, state: &ExitChainState) -> ChainInterpretation {
    let node_map = &plan.tree_nodes;

    let leaf_ids: Vec<TreeNodeId> = plan
        .per_branch_funding
        .iter()
        .map(|(leaf_id, _)| leaf_id.clone())
        .collect();
    let ExitChainWalk {
        nodes,
        refunds,
        stopped,
        mut unverified,
        unverifiable_confirmed,
        sweeps: _,
        pending: _,
    } = restore_exit_chain_walk(state, &leaf_ids);

    flag_unverifiable_confirmation_branches(
        node_map,
        plan,
        &unverifiable_confirmed,
        &mut unverified,
    );

    trace!(
        resolved_nodes = nodes.len(),
        resolved_refunds = refunds.len(),
        unverified = unverified.len(),
        "interpret_chain: exit state"
    );
    ChainInterpretation {
        resolved: ResolvedExitState {
            nodes,
            refunds,
            stopped,
        },
        unverified,
    }
}

/// Marks a branch's driven txs unverified when one of its nodes is confirmed but
/// its on-chain spend can't be verified: either confirmed via the operator-OnChain
/// fallback (the node's own chain lookup was unavailable), or chain-confirmed with
/// its CPFP child's spend or body lookup unavailable, so its change is unresolved.
/// In both cases the confirmed child's spend is invisible to `scan_funding`, so a
/// re-supplied input the child already spent wouldn't be dropped and the next driven
/// child would double-spend; flagging (not the `Unconfirmed` the build would
/// otherwise emit) tells the caller not to broadcast until a later run confirms it on
/// a healthy chain. Chain-verified confirmations with a resolved (or safely absent)
/// change are left alone. Only `Unconfirmed` txs are upgraded, so confirmed nodes
/// keep their state.
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

/// Where to start walking a branch, and what it proves.
///
/// The operators label a node they believe on-chain, so the deepest such node is
/// where the walk can pick up: a node's `node_tx` spends its parent's, so one in
/// a block puts every one above it in a block too, and by that same spend each
/// was confirmed via its cpfp transaction rather than any other way.
///
/// The label is only where to look. A block can be reorganised out, so the node
/// itself is checked, and the walk steps up to its parent for as long as the
/// answer is no.
fn resume_point(
    chain_nodes: &[&TreeNode],
    observed: &ObservedIndex<'_>,
    pending: &mut Vec<ChainQuery>,
) -> ResumePoint {
    let mut candidates = chain_nodes
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, node)| node.status == TreeNodeStatus::OnChain);

    for (index, node) in candidates.by_ref() {
        let query = ChainQuery::TxConfirmed(node.node_tx.compute_txid());
        match observed.get(&query) {
            Some(ChainResult::Confirmed {
                confirmed: true,
                block_height,
            }) => {
                trace!(node = %node.id, "walk: resuming below the deepest confirmed node");
                return ResumePoint::Below {
                    index,
                    block_height: *block_height,
                };
            }
            // Not in a block: a label that was never true, or one a reorg undid.
            // Its parent is the next place to look.
            Some(ChainResult::Confirmed {
                confirmed: false, ..
            }) => {}
            // Unreadable, so nothing is known. Start from the deposit, where the
            // walk's own fallbacks apply.
            Some(_) => return ResumePoint::Deposit,
            None => {
                pending.push(query);
                return ResumePoint::Awaiting;
            }
        }
    }
    ResumePoint::Deposit
}

/// Where [`resume_point`] says a branch walk should begin.
enum ResumePoint {
    /// Start below the node at this index, which is confirmed, along with every
    /// node above it.
    Below {
        index: usize,
        /// The block the node at `index` is in. The nodes above it are in a block
        /// too, by the same spend chain, but which one was never asked.
        block_height: Option<u32>,
    },
    /// Nothing is known to be on-chain: start at the deposit.
    Deposit,
    /// A lookup is outstanding; nothing to do until it is answered.
    Awaiting,
}

/// Follows the confirmed spender from the deposit down one branch, classifying
/// each node into `nodes`/`refunds`. Stops (emitting the next lookup into
/// `pending`) at the first output whose spender is not yet observed.
fn walk_branch(
    node_map: &HashMap<TreeNodeId, TreeNode>,
    leaf_id: &TreeNodeId,
    observed: &ObservedIndex<'_>,
    walk: &mut ExitChainWalk,
) {
    let ExitChainWalk {
        nodes,
        refunds,
        stopped,
        unverified,
        unverifiable_confirmed,
        sweeps: _,
        pending,
    } = walk;
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

    let resume = resume_point(&chain_nodes, observed, pending);
    let start = match resume {
        ResumePoint::Awaiting => return,
        ResumePoint::Deposit => 0,
        ResumePoint::Below {
            index,
            block_height,
        } => {
            for (i, node) in chain_nodes[..=index].iter().enumerate() {
                nodes.insert(
                    node.id.clone(),
                    NodeState::ConfirmedCpfp {
                        // Only the node that was checked has a known height.
                        block_height: if i == index { block_height } else { None },
                    },
                );
            }
            // The leaf itself was the confirmed one, so its refund is all that is
            // left, and that is read separately.
            if index + 1 >= chain_nodes.len() {
                return;
            }
            index + 1
        }
    };

    // Confirmed parent's node_tx txid; `None` at the root (spends the deposit).
    let mut prev_confirmed_txid: Option<Txid> = match start {
        0 => None,
        i => Some(chain_nodes[i - 1].node_tx.compute_txid()),
    };
    for node in &chain_nodes[start..] {
        let is_leaf = &node.id == leaf_id;
        let live_outpoint = match prev_confirmed_txid {
            Some(txid) => OutPoint {
                txid,
                // The index the node spends in its parent is the one its own
                // transaction carries, not the one the operators report:
                // operators predating buildonspark/spark@c8bfcfcf ("Use leaf
                // parent vout in renew signing") report the index the leaf held
                // under its previous parent after a timelock renewal, which
                // would walk the wrong output. This path has to hold without
                // the operators, so a stale index cannot wait for a swap.
                vout: node
                    .node_tx
                    .input
                    .first()
                    .map_or(node.vout, |input| input.previous_output.vout),
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
            ChainResult::Spend(Some(info)) if info.confirmed => *info,
            // The scan ignores a spender that is only in the mempool. No spender
            // in a block: the frontier, built from here. Its confirmed parent funds
            // it from its on-chain CPFP change.
            ChainResult::Spend(_) => {
                trace!(%leaf_id, node = %node.id, "walk: frontier reached (no spender in a block)");
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
                    nodes.insert(
                        node.id.clone(),
                        NodeState::ConfirmedCpfp { block_height: None },
                    );
                    unverifiable_confirmed.insert(node.id.clone());
                    if is_leaf {
                        return;
                    }
                    prev_confirmed_txid = Some(node.node_tx.compute_txid());
                    continue;
                }
                trace!(%leaf_id, node = %node.id, "walk: lookup unavailable, node unverified");
                unverified.insert(node.id.clone());
                return;
            }
            _ => return,
        };
        let node_txid = node.node_tx.compute_txid();
        let direct_txid = node.direct_tx.as_ref().map(Transaction::compute_txid);

        if info.spender_txid == node_txid {
            trace!(%leaf_id, node = %node.id, is_leaf, "walk: confirmed via cpfp node_tx");
            nodes.insert(
                node.id.clone(),
                NodeState::ConfirmedCpfp {
                    block_height: info.block_height,
                },
            );
            if is_leaf {
                return;
            }
            prev_confirmed_txid = Some(node_txid);
        } else if is_leaf && direct_txid == Some(info.spender_txid) {
            // A leaf is terminal, so its own direct spend is recoverable via the
            // direct refund, if held.
            if node.direct_refund_tx.is_some() {
                trace!(%leaf_id, node = %node.id, "walk: leaf went direct, driving direct refund");
                nodes.insert(
                    node.id.clone(),
                    NodeState::ConfirmedDirect {
                        block_height: info.block_height,
                    },
                );
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

/// Where following a leaf's refund through `observed` ends.
enum RefundTrail {
    /// A query it needs has no result yet. The query is in `pending`.
    Pending,
    /// The chain service returned no result for a query.
    Unavailable,
    /// The listing of the address contradicts itself.
    Unreadable,
    /// No output of the address is in a block: the refund is not on-chain.
    NotOnChain,
    /// The output in a block that no transaction in a block has as input: the
    /// refund's, or that of the child paying the refund's fee.
    Unspent(AddressUtxo),
    /// The sweep of the refund, in a block.
    Swept {
        txid: Txid,
        block_height: Option<u32>,
    },
}

/// Follows a leaf's refund from its address. The address scan returns every
/// output paid to it, spent or not, so its one refund output is found even after
/// a sweep has it as input. The [`ChainQuery::Outspend`] of that output then
/// tells the states apart:
///
/// - unspent, or an input of a transaction in no block: [`RefundTrail::Unspent`],
/// - an input of a transaction in a block that pays the address again: that
///   output takes the refund's place and is followed the same way,
/// - an input of any other transaction in a block: [`RefundTrail::Swept`].
fn follow_refund(
    leaf_id: &TreeNodeId,
    address: &Address,
    observed: &ObservedIndex<'_>,
    pending: &mut Vec<ChainQuery>,
) -> RefundTrail {
    let scan_query = ChainQuery::RefundAddress {
        leaf_id: leaf_id.clone(),
        address: address.clone(),
    };
    let Some(result) = observed.get(&scan_query) else {
        pending.push(scan_query);
        return RefundTrail::Pending;
    };
    let txos = match result {
        ChainResult::AddressUtxos(txos) => txos,
        ChainResult::Unavailable => return RefundTrail::Unavailable,
        _ => return RefundTrail::NotOnChain,
    };
    // Only one refund variant lands, but once it has the address is public and a
    // later payment to it is listed first, so the refund is the largest confirmed
    // output. No confirmed output means the refund is not on-chain yet.
    let Some(mut txo) = txos.iter().filter(|t| t.confirmed).max_by_key(|t| t.value) else {
        return RefundTrail::NotOnChain;
    };
    let mut steps = 0;
    loop {
        // Each step moves to another listed output, so more steps than outputs
        // means the listing contradicts itself.
        steps += 1;
        if steps > txos.len() {
            return RefundTrail::Unreadable;
        }
        let outspend_query = ChainQuery::Outspend(OutPoint {
            txid: txo.txid,
            vout: txo.vout,
        });
        let Some(spend) = observed.get(&outspend_query) else {
            pending.push(outspend_query);
            return RefundTrail::Pending;
        };
        let (spender, block_height) = match spend {
            ChainResult::Spend(Some(info)) if info.confirmed => {
                (info.spender_txid, info.block_height)
            }
            ChainResult::Unavailable => return RefundTrail::Unavailable,
            _ => return RefundTrail::Unspent(*txo),
        };
        // A child that pays the refund's fee out of its value pays the leaf's key
        // again.
        let Some(next) = txos
            .iter()
            .filter(|t| t.confirmed && t.txid == spender)
            .max_by_key(|t| t.value)
        else {
            return RefundTrail::Swept {
                txid: spender,
                block_height,
            };
        };
        txo = next;
    }
}

/// Resolves a leaf's refund from its address, see [`follow_refund`]:
///
/// - unspent: [`RefundState::Adopted`], swept by the build,
/// - swept: [`RefundState::Swept`], nothing left to do,
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
    sweeps: &mut HashMap<TreeNodeId, (Txid, Option<u32>)>,
    unverified: &mut HashSet<TreeNodeId>,
    pending: &mut Vec<ChainQuery>,
) {
    let txo = match follow_refund(leaf_id, address, observed, pending) {
        RefundTrail::Pending | RefundTrail::NotOnChain => return,
        RefundTrail::Unavailable | RefundTrail::Unreadable => {
            unverified.insert(leaf_id.clone());
            return;
        }
        RefundTrail::Swept { txid, block_height } => {
            trace!(%leaf_id, sweep = %txid, "interpret_chain: refund swept");
            refunds.insert(leaf_id.clone(), RefundState::Swept);
            sweeps.insert(leaf_id.clone(), (txid, block_height));
            return;
        }
        RefundTrail::Unspent(txo) => txo,
    };
    let refund_outpoint = OutPoint {
        txid: txo.txid,
        vout: txo.vout,
    };

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
            block_height: txo.block_height,
        }),
    );
}

/// Builds a complete unilateral exit from a `prepared` quote, the `state` a
/// [`scan_exit_chain`] resolved. A default `state` builds a fresh full exit.
///
/// Reads no chain of its own: the tree is in `state` and the funding was
/// resolved into the plan by [`scan_funding`]. Each not-yet-confirmed tx gets an
/// unsigned CPFP child that pays its fee; confirmed nodes and adopted refunds
/// are emitted without one. Each refund output is swept under the key derived
/// from its leaf's node id, the key a wallet's leaves are held under. The child
/// of a refund without an anchor is left unsigned for
/// [`SparkWallet::build_unilateral_exit`](crate::SparkWallet::build_unilateral_exit)
/// to sign.
pub(crate) fn build_unilateral_exit(
    plan: &UnilateralExitPlan,
    state: &ExitChainState,
    fee_rate_sat_per_kw: u64,
) -> Result<UnilateralExitBuild, SparkWalletError> {
    let interpretation = interpret_chain(plan, state);
    let mut build = build_exit(plan, &interpretation.resolved, fee_rate_sat_per_kw)?;
    flag_unverified_txs(&mut build, &interpretation);
    Ok(build)
}

/// Upgrades `Unconfirmed` to `Unverified` for txs whose chain lookup failed (the
/// build is chain-blind, so this is applied afterward).
fn flag_unverified_txs(build: &mut UnilateralExitBuild, interpretation: &ChainInterpretation) {
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

    // Always the plan's own fan-out. One that already confirmed never reaches
    // here: its outputs were traced back as funding, so the plan assigned them
    // to branches and built none.
    let fan_out = plan.fan_out_psbt.as_ref().map(|psbt| ExitTx {
        kind: ExitTxKind::FanOut,
        node_id: None,
        txid: psbt.unsigned_tx.compute_txid(),
        base_tx: psbt.unsigned_tx.clone(),
        to_sign: Some(psbt.clone()),
        cpfp_tx: None,
        csv_timelock_blocks: None,
        depends_on: vec![],
        status: ExitTxStatus::Unconfirmed,
    });
    let fan_out_txid = fan_out.as_ref().map(|f| f.txid);
    let per_branch_funding = plan.per_branch_funding.clone();

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
        let mut funding = branch_funding.clone();
        let mut txs: Vec<ExitTx> = Vec::new();
        let mut fan_out_dep = fan_out_txid;
        // Tracked so dependencies survive skipped shared ancestors.
        let mut prev_txid: Option<Txid> = None;

        let mut leaf_node_txid: Option<Txid> = None;

        // A stopped branch drives no nodes; only an adopted refund is swept below.
        if !stopped {
            for node in chain {
                let node_state = resolved.nodes.get(&node.id);
                let base_tx = if matches!(node_state, Some(NodeState::ConfirmedDirect { .. })) {
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
                    let to_sign = match node_state {
                        // Confirmed: its child already paid, and what that child
                        // left over came back as funding like any other input, so
                        // the branch keeps what it was assigned.
                        Some(_) => None,
                        None => {
                            let child =
                                build_cpfp_child(&node.node_tx, &funding, fee_rate_sat_per_kw)?;
                            cpfp_fee_sat = cpfp_fee_sat.saturating_add(child.fee_sat);
                            funding = vec![child.change_input];
                            Some(child.psbt)
                        }
                    };
                    let mut depends_on = Vec::new();
                    if let Some(p) = parent_txid {
                        depends_on.push(p);
                    }
                    if to_sign.is_some()
                        && let Some(fo) = fan_out_dep.take()
                    {
                        depends_on.push(fo);
                    }
                    let status = match node_state {
                        Some(
                            NodeState::ConfirmedCpfp { block_height }
                            | NodeState::ConfirmedDirect { block_height },
                        ) => ExitTxStatus::Confirmed {
                            block_height: *block_height,
                        },
                        None => ExitTxStatus::Unconfirmed,
                    };
                    txs.push(ExitTx {
                        kind: ExitTxKind::Node,
                        node_id: Some(node.id.clone()),
                        txid: node_txid,
                        csv_timelock_blocks: csv_timelock(&base_tx),
                        base_tx,
                        to_sign,
                        cpfp_tx: None,
                        depends_on,
                        status,
                    });
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
                    signing_key: LeafSigningKey {
                        derived_from: leaf_id.clone(),
                    },
                });
                txs.push(ExitTx {
                    kind: ExitTxKind::Refund,
                    node_id: Some(leaf_id.clone()),
                    txid: adopted.outpoint.txid,
                    csv_timelock_blocks: csv_timelock(&adopted.tx),
                    base_tx: adopted.tx.clone(),
                    to_sign: None,
                    cpfp_tx: None,
                    depends_on: vec![],
                    status: ExitTxStatus::Confirmed {
                        block_height: adopted.block_height,
                    },
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
                    signing_key: LeafSigningKey {
                        derived_from: leaf_id.clone(),
                    },
                });
                txs.push(ExitTx {
                    kind: ExitTxKind::Refund,
                    node_id: Some(leaf_id.clone()),
                    txid: refund_txid,
                    base_tx: direct_refund,
                    to_sign: None,
                    cpfp_tx: None,
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
                if !has_anchor(&refund_tx) {
                    let child = build_anchorless_refund_child(&refund_tx, fee_rate_sat_per_kw)?;
                    refund_outputs.push(RefundOutput {
                        outpoint: OutPoint {
                            txid: child.tx.compute_txid(),
                            vout: 0,
                        },
                        leaf_id: leaf_id.clone(),
                        value: refund_value.saturating_sub(child.fee_sat),
                        signing_key: LeafSigningKey {
                            derived_from: leaf_id.clone(),
                        },
                    });
                    // Nothing spent what is left of the branch's funding, so the
                    // sweep takes it back.
                    cpfp_change_inputs.extend(funding.drain(..).map(|input| CpfpChangeInput {
                        outpoint: input.outpoint,
                        witness_utxo: input.witness_utxo,
                        signed_input_weight: input.signed_input_weight,
                    }));
                    txs.push(ExitTx {
                        kind: ExitTxKind::Refund,
                        node_id: Some(leaf_id.clone()),
                        txid: refund_txid,
                        base_tx: refund_tx,
                        to_sign: None,
                        cpfp_tx: Some(child.tx),
                        csv_timelock_blocks: refund_csv,
                        depends_on: leaf_node_txid.into_iter().collect(),
                        status: ExitTxStatus::Unconfirmed,
                    });
                } else {
                    let child = build_cpfp_child(&refund_tx, &funding, fee_rate_sat_per_kw)?;
                    cpfp_fee_sat = cpfp_fee_sat.saturating_add(child.fee_sat);
                    refund_outputs.push(RefundOutput {
                        outpoint: OutPoint {
                            txid: refund_txid,
                            vout: 0,
                        },
                        leaf_id: leaf_id.clone(),
                        value: refund_value,
                        signing_key: LeafSigningKey {
                            derived_from: leaf_id.clone(),
                        },
                    });
                    // The refund child's change is the branch's terminal sweep input.
                    cpfp_change_inputs.push(CpfpChangeInput {
                        outpoint: child.change_input.outpoint,
                        witness_utxo: child.change_input.witness_utxo.clone(),
                        signed_input_weight: child.change_input.signed_input_weight,
                    });
                    let mut depends_on: Vec<Txid> = leaf_node_txid.into_iter().collect();
                    if let Some(fo) = fan_out_dep.take() {
                        depends_on.push(fo);
                    }
                    txs.push(ExitTx {
                        kind: ExitTxKind::Refund,
                        node_id: Some(leaf_id.clone()),
                        txid: refund_txid,
                        base_tx: refund_tx,
                        to_sign: Some(child.psbt),
                        cpfp_tx: None,
                        csv_timelock_blocks: refund_csv,
                        depends_on,
                        status: ExitTxStatus::Unconfirmed,
                    });
                }
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
    let fanout_fee_sat = fresh_fan_out_fee(plan, fan_out.as_ref());
    let refund_fee_sat = refund_outputs
        .iter()
        .filter_map(|output| {
            let leaf = node_map.get(&output.leaf_id)?;
            Some(leaf.value.saturating_sub(output.value))
        })
        .fold(0u64, u64::saturating_add);

    debug!(
        has_fan_out = fan_out.is_some(),
        branches = branches.len(),
        refund_outputs = refund_outputs.len(),
        cpfp_change_inputs = cpfp_change_inputs.len(),
        recoverable_value_sat,
        cpfp_fee_sat,
        fanout_fee_sat,
        refund_fee_sat,
        "build_unilateral_exit: assembled"
    );
    Ok(UnilateralExitBuild {
        fan_out,
        branches,
        refund_outputs,
        cpfp_change_inputs,
        recoverable_value_sat,
        cpfp_fee_sat,
        fanout_fee_sat,
        refund_fee_sat,
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

#[cfg(test)]
fn to_node_map(nodes: Vec<TreeNode>) -> HashMap<TreeNodeId, TreeNode> {
    nodes.into_iter().map(|n| (n.id.clone(), n)).collect()
}

#[cfg(test)]
mod exit_build_tests {
    use super::*;
    use bitcoin::{
        Amount, CompressedPublicKey, ScriptBuf, TxOut, Weight,
        absolute::LockTime,
        hashes::Hash,
        key::Secp256k1,
        secp256k1::{PublicKey, SecretKey},
        transaction::Version,
    };
    use spark::{
        Identifier,
        services::{
            ServiceError, UnilateralExitLeafFilter, UnilateralExitSelectedLeaf,
            compute_cpfp_package_fee, compute_sweep_fee, plan_unilateral_exit,
            quote_unilateral_exit,
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

    /// The refund of a leaf that `node` makes: an output that holds the leaf's
    /// value, then the anchor.
    fn refund_tx(nonce: u32) -> Transaction {
        let mut tx = anchor_tx(nonce);
        tx.output.insert(
            0,
            TxOut {
                value: Amount::from_sat(100_000),
                script_pubkey: ScriptBuf::new(),
            },
        );
        tx
    }

    fn tx_paying(value: u64, nonce: u32) -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::from_height(nonce).unwrap(),
            input: vec![],
            output: vec![TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::new(),
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
        let leaf = node("leaf", Some("root"), anchor_tx(2), Some(refund_tx(3)));
        plan_of(root, leaf)
    }

    fn direct_leaf_plan() -> UnilateralExitPlan {
        let root = node("root", None, anchor_tx(1), None);
        let mut leaf = node("leaf", Some("root"), anchor_tx(2), Some(refund_tx(3)));
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
                needs_funding: true,
                refund_fee: 0,
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
        assert_eq!(build.refund_outputs[0].signing_key.derived_from, id("leaf"));
        assert_eq!(build.refund_outputs[0].outpoint.txid, refund.txid);
        assert_eq!(build.refund_outputs[0].outpoint.vout, 0);
        assert_eq!(build.cpfp_change_inputs.len(), 1);
    }

    fn anchorless_refund(value: u64) -> Transaction {
        let mut script = vec![0x51, 0x20];
        script.extend_from_slice(&[7u8; 32]);
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![bitcoin::TxIn::default()],
            output: vec![TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::from_bytes(script),
            }],
        }
    }

    #[test]
    fn build_pays_an_anchorless_refund_from_the_leaf() {
        let root = node("root", None, anchor_tx(1), None);
        let refund_tx = anchorless_refund(100_000);
        let leaf = node("leaf", Some("root"), anchor_tx(2), Some(refund_tx.clone()));
        let plan = plan_of(root, leaf);

        let build = build_exit(&plan, &ResolvedExitState::default(), FEE_RATE).unwrap();

        let txs = &build.branches[0].txs;
        assert_eq!(txs.len(), 3);
        let refund = txs.last().unwrap();
        assert_eq!(refund.kind, ExitTxKind::Refund);
        assert_eq!(refund.txid, refund_tx.compute_txid());
        assert!(refund.to_sign.is_none(), "the funding pays nothing for it");
        let child = refund.cpfp_tx.as_ref().expect("a child pays its fee");
        assert_eq!(
            child.input[0].previous_output,
            OutPoint {
                txid: refund.txid,
                vout: 0
            }
        );
        assert_eq!(
            child.output[0].script_pubkey,
            refund_tx.output[0].script_pubkey
        );

        let fee = spark::services::anchorless_refund_package_fee(&refund_tx, FEE_RATE);
        assert_eq!(build.refund_fee_sat, fee);
        assert_eq!(child.output[0].value.to_sat(), 100_000 - fee);

        // The sweep takes the child's output, not the refund's.
        assert_eq!(build.refund_outputs.len(), 1);
        assert_eq!(
            build.refund_outputs[0].outpoint,
            OutPoint {
                txid: child.compute_txid(),
                vout: 0
            }
        );
        assert_eq!(build.refund_outputs[0].value, 100_000 - fee);
        // What the node's cpfp children left of the funding goes to the sweep too.
        assert_eq!(build.cpfp_change_inputs.len(), 1);
        let anchored =
            build_exit(&single_leaf_plan(), &ResolvedExitState::default(), FEE_RATE).unwrap();
        assert!(build.cpfp_fee_sat < anchored.cpfp_fee_sat);
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
                    block_height: None,
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
        assert_eq!(build.refund_outputs[0].signing_key.derived_from, id("leaf"));
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
            nodes: [(id("root"), NodeState::ConfirmedCpfp { block_height: None })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let build = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        let txs = &build.branches[0].txs;
        let root = &txs[0];
        assert_eq!(root.node_id.as_ref(), Some(&id("root")));
        assert!(root.to_sign.is_none(), "a confirmed node carries no child");
        assert_eq!(root.status, ExitTxStatus::Confirmed { block_height: None });
        assert!(
            txs.iter().skip(1).all(|t| t.to_sign.is_some()),
            "nodes below the confirmed one are still driven"
        );
    }

    #[test]
    fn build_drives_direct_refund() {
        let resolved = ResolvedExitState {
            nodes: [
                (id("root"), NodeState::ConfirmedCpfp { block_height: None }),
                (
                    id("leaf"),
                    NodeState::ConfirmedDirect { block_height: None },
                ),
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
        assert_eq!(build.refund_outputs[0].signing_key.derived_from, id("leaf"));
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
                    block_height: None,
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
        assert_eq!(build.refund_outputs[0].signing_key.derived_from, id("leaf"));
        assert_eq!(build.refund_outputs[0].outpoint, adopted_outpoint);
    }

    #[test]
    fn build_omits_swept_leaf_refund() {
        let resolved = ResolvedExitState {
            nodes: [
                (id("root"), NodeState::ConfirmedCpfp { block_height: None }),
                (id("leaf"), NodeState::ConfirmedCpfp { block_height: None }),
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
        let leaf_a = node("leafA", Some("mid"), anchor_tx(3), Some(refund_tx(4)));
        let leaf_b = node("leafB", Some("mid"), anchor_tx(5), Some(refund_tx(6)));
        UnilateralExitPlan {
            selected_leaves: vec![
                UnilateralExitSelectedLeaf {
                    id: leaf_a.id.clone(),
                    value: 100_000,
                    estimated_cost: 2_000,
                    cpfp_cost: 2_000,
                    needs_funding: true,
                    refund_fee: 0,
                },
                UnilateralExitSelectedLeaf {
                    id: leaf_b.id.clone(),
                    value: 100_000,
                    estimated_cost: 2_000,
                    cpfp_cost: 2_000,
                    needs_funding: true,
                    refund_fee: 0,
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
        let leaf_a = node("leafA", Some("mid"), anchor_tx(3), Some(refund_tx(4)));
        let leaf_b = node("leafB", Some("mid"), anchor_tx(5), Some(refund_tx(6)));

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
                    needs_funding: true,
                    refund_fee: 0,
                },
                UnilateralExitSelectedLeaf {
                    id: leaf_b.id.clone(),
                    value: 100_000,
                    estimated_cost: 2_000,
                    cpfp_cost: 2_000,
                    needs_funding: true,
                    refund_fee: 0,
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
    fn resumed_branch_hangs_the_fanout_dependency_on_the_first_built_tx() {
        // Resume with the shared ancestors already in a block. They are emitted
        // first but build no child, so the leaf's child is what spends the
        // fan-out output. A confirmed transaction never waits on anything, so a
        // dependency parked on one is silently lost.
        let plan = shared_ancestor_plan_with_fan_out();
        let fan_out_txid = plan
            .fan_out_psbt
            .as_ref()
            .unwrap()
            .unsigned_tx
            .compute_txid();
        let resolved = ResolvedExitState {
            nodes: [
                (
                    id("root"),
                    NodeState::ConfirmedCpfp {
                        block_height: Some(1),
                    },
                ),
                (
                    id("mid"),
                    NodeState::ConfirmedCpfp {
                        block_height: Some(2),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let build = build_exit(&plan, &resolved, FEE_RATE).unwrap();

        for branch in &build.branches {
            let carrying: Vec<&ExitTx> = branch
                .txs
                .iter()
                .filter(|t| t.depends_on.contains(&fan_out_txid))
                .collect();
            assert_eq!(
                carrying.len(),
                1,
                "exactly one tx per branch waits on the fan-out"
            );
            assert!(
                carrying[0].to_sign.is_some(),
                "the tx waiting on the fan-out is the one whose child spends it"
            );
        }
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
            cpfp_tx: None,
            csv_timelock_blocks: None,
            depends_on: vec![],
            status: if nonce == 1 {
                ExitTxStatus::Confirmed { block_height: None }
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
            cpfp_fee_sat: 0,
            fanout_fee_sat: 0,
            refund_fee_sat: 0,
        };
        let interpretation = ChainInterpretation {
            resolved: ResolvedExitState::default(),
            unverified: [id("mid"), id("leaf")].into_iter().collect(),
        };
        flag_unverified_txs(&mut build, &interpretation);

        let txs = &build.branches[0].txs;
        assert_eq!(
            txs[0].status,
            ExitTxStatus::Confirmed { block_height: None },
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
    fn build_cpfp_fee_sums_built_cpfp_children() {
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
        assert_eq!(build.cpfp_fee_sat, children_fee);
        assert_eq!(build.fanout_fee_sat, 0);
    }

    #[test]
    fn resume_confirmed_node_lowers_total_fee() {
        let all_driven =
            build_exit(&single_leaf_plan(), &ResolvedExitState::default(), FEE_RATE).unwrap();
        let resolved = ResolvedExitState {
            nodes: [(id("root"), NodeState::ConfirmedCpfp { block_height: None })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let resumed = build_exit(&single_leaf_plan(), &resolved, FEE_RATE).unwrap();
        assert!(
            resumed.cpfp_fee_sat < all_driven.cpfp_fee_sat,
            "a confirmed node is not rebuilt, so the resume pays less \
             ({} vs {})",
            resumed.cpfp_fee_sat,
            all_driven.cpfp_fee_sat
        );
    }

    #[test]
    fn plan_single_leaf_two_utxo_funding_boundary_is_exact() {
        // A single leaf funded with TWO UTXOs: build funds the first CPFP child with
        // both, so its fee is sized on their combined weight. The plan gate must
        // charge that, not the one-input estimate the selection pass uses; otherwise
        // funding in the gap passes the plan then fails build_cpfp_child.
        let root = node("root", None, anchor_tx(1), None);
        let leaf = node("leaf", Some("root"), anchor_tx(2), Some(refund_tx(3)));
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
                &ExitChainState::default(),
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
                &ExitChainState::default(),
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
        let leaf_a = node("leafA", None, anchor_tx(1), Some(refund_tx(2)));
        let leaf_b = node("leafB", None, anchor_tx(3), Some(refund_tx(4)));
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
            &ExitChainState::default(),
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
            &ExitChainState::default(),
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

    #[test]
    fn plan_three_leaves_two_utxos_fans_out_and_builds() {
        // Case e: more leaves than funding UTXOs. The plan fans out (2 inputs, 3
        // outputs) and build_exit threads all three branches off that fan-out.
        let leaf_a = node("leafA", None, anchor_tx(1), Some(refund_tx(2)));
        let leaf_b = node("leafB", None, anchor_tx(3), Some(refund_tx(4)));
        let leaf_c = node("leafC", None, anchor_tx(5), Some(refund_tx(6)));
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
            FEE_RATE,
            change_len,
            &ExitChainState::default(),
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

    #[test]
    fn leaves_under_one_root_build_from_the_quote_amounts_per_branch() {
        let root = node("root", None, anchor_tx(1), None);
        let leaf_a = node("leafA", Some("root"), anchor_tx(2), Some(refund_tx(3)));
        let mut leaf_b = node("leafB", Some("root"), anchor_tx(4), Some(refund_tx(5)));
        leaf_b.vout = 1;
        let leaf_ids = [leaf_a.id.clone(), leaf_b.id.clone()];
        let nodes = to_node_map(vec![root, leaf_a, leaf_b]);
        let change_len = funding(0).witness_utxo.script_pubkey.len();
        let dust = funding(0)
            .witness_utxo
            .script_pubkey
            .minimal_non_dust()
            .to_sat();

        let quote = quote_unilateral_exit(
            &nodes,
            &leaf_ids,
            UnilateralExitLeafFilter::All,
            272,
            change_len,
            dust,
            FEE_RATE,
            change_len,
            &ExitChainState::default(),
        )
        .unwrap();
        let inputs: Vec<CpfpInput> = quote
            .per_branch_funding
            .iter()
            .zip(0u32..)
            .map(|((_, sats), vout)| {
                let mut input = funding(*sats);
                input.outpoint.vout = vout;
                input
            })
            .collect();
        let plan = plan_unilateral_exit(
            nodes,
            &leaf_ids,
            UnilateralExitLeafFilter::All,
            inputs,
            FEE_RATE,
            change_len,
            &ExitChainState::default(),
        )
        .unwrap();
        let build = build_unilateral_exit(&plan, &ExitChainState::default(), FEE_RATE).unwrap();

        assert!(build.fan_out.is_none());
        // The builder builds the root's child in the first branch only.
        let children: Vec<usize> = build
            .branches
            .iter()
            .map(|branch| branch.txs.iter().filter(|tx| tx.to_sign.is_some()).count())
            .collect();
        assert_eq!(children, [3, 2]);
        assert_eq!(quote.cpfp_fee_sat, build.cpfp_fee_sat);
    }

    #[test]
    fn quote_matches_the_build_for_a_node_only_the_operators_report_on_chain() {
        let mut root = node("root", None, anchor_tx(1), None);
        root.status = TreeNodeStatus::OnChain;
        let leaf = node("leaf", Some("root"), anchor_tx(2), Some(refund_tx(3)));
        let leaf_id = leaf.id.clone();
        let nodes = to_node_map(vec![root, leaf]);
        // The scan did not find the root in a block.
        let on_chain = ExitChainState::default();
        let change_len = funding(0).witness_utxo.script_pubkey.len();
        let dust = funding(0)
            .witness_utxo
            .script_pubkey
            .minimal_non_dust()
            .to_sat();

        let quote = quote_unilateral_exit(
            &nodes,
            std::slice::from_ref(&leaf_id),
            UnilateralExitLeafFilter::All,
            272,
            change_len,
            dust,
            FEE_RATE,
            change_len,
            &on_chain,
        )
        .unwrap();
        let plan = plan_unilateral_exit(
            nodes,
            std::slice::from_ref(&leaf_id),
            UnilateralExitLeafFilter::All,
            vec![funding(quote.single_utxo_funding_sat)],
            FEE_RATE,
            change_len,
            &on_chain,
        )
        .unwrap();
        let build = build_unilateral_exit(&plan, &on_chain, FEE_RATE).unwrap();

        assert!(build.branches[0].txs.iter().all(|tx| tx.to_sign.is_some()));
        assert_eq!(quote.cpfp_fee_sat, build.cpfp_fee_sat);
    }

    /// The fee of a sweep of the refund outputs and change inputs in `build`, as
    /// `create_refund_sweep_transaction` computes it.
    fn sweep_fee(build: &UnilateralExitBuild, destination_script_len: usize) -> u64 {
        let refunds = p2tr_key_path_input_weight() * build.refund_outputs.len() as u64;
        let change: u64 = build
            .cpfp_change_inputs
            .iter()
            .map(|input| input.signed_input_weight)
            .sum();
        compute_sweep_fee(
            refunds + Weight::from_wu(change),
            destination_script_len,
            FEE_RATE,
        )
    }

    #[test]
    fn quote_and_plan_match_the_build_of_a_direct_refund() {
        let root = node("root", None, anchor_tx(1), None);
        let mut leaf = node("leaf", Some("root"), anchor_tx(2), Some(refund_tx(3)));
        // The output of each direct transaction holds its input less 955 sats.
        let direct_refund = tx_paying(98_090, 5);
        leaf.direct_tx = Some(tx_paying(99_045, 4));
        leaf.direct_refund_tx = Some(direct_refund.clone());
        let nodes = to_node_map(vec![root, leaf]);
        let leaf_id = id("leaf");
        let on_chain = ExitChainState {
            nodes: vec![
                ConfirmedExitNode {
                    node_id: leaf_id.clone(),
                    confirmed_by: ExitNodeConfirmation::Direct,
                    block_height: None,
                },
                ConfirmedExitNode {
                    node_id: id("root"),
                    confirmed_by: ExitNodeConfirmation::Cpfp,
                    block_height: None,
                },
            ],
            ..Default::default()
        };
        let change_len = funding(0).witness_utxo.script_pubkey.len();
        let dust = funding(0)
            .witness_utxo
            .script_pubkey
            .minimal_non_dust()
            .to_sat();

        let quote = quote_unilateral_exit(
            &nodes,
            std::slice::from_ref(&leaf_id),
            UnilateralExitLeafFilter::All,
            272,
            change_len,
            dust,
            FEE_RATE,
            change_len,
            &on_chain,
        )
        .unwrap();
        let plan = plan_unilateral_exit(
            nodes,
            std::slice::from_ref(&leaf_id),
            UnilateralExitLeafFilter::All,
            Vec::new(),
            FEE_RATE,
            change_len,
            &on_chain,
        )
        .unwrap();
        let build = build_unilateral_exit(&plan, &on_chain, FEE_RATE).unwrap();

        let txs = &build.branches[0].txs;
        assert_eq!(txs.last().unwrap().txid, direct_refund.compute_txid());
        assert!(txs.iter().all(|tx| tx.to_sign.is_none()));
        assert_eq!(quote.cpfp_fee_sat, build.cpfp_fee_sat);
        assert_eq!(quote.sweep_fee_sat, sweep_fee(&build, change_len));
        assert!(quote.per_branch_funding.is_empty());
        assert!(plan.per_branch_funding[0].1.is_empty());
    }

    #[test]
    fn quote_and_plan_match_the_build_of_a_stopped_branch() {
        let root = node("root", None, anchor_tx(1), None);
        let mut stopped = node("stopped", Some("root"), anchor_tx(2), Some(refund_tx(3)));
        // Worth more than its sibling, so it comes first.
        stopped.value = 100_001;
        let mut sibling = node("sibling", Some("root"), anchor_tx(4), Some(refund_tx(5)));
        sibling.vout = 1;
        let stopped_id = stopped.id.clone();
        let leaf_ids = [stopped_id.clone(), sibling.id.clone()];
        let nodes = to_node_map(vec![root, stopped, sibling]);
        let refund = tx_paying(100_001, 9);
        let on_chain = ExitChainState {
            nodes: vec![ConfirmedExitNode {
                node_id: id("root"),
                confirmed_by: ExitNodeConfirmation::Cpfp,
                block_height: None,
            }],
            refunds: vec![ExitRefund {
                leaf_id: stopped_id.clone(),
                state: ExitRefundState::OnChain {
                    tx: refund.clone(),
                    vout: 0,
                    value: 100_001,
                    block_height: None,
                },
            }],
            stopped_leaves: vec![stopped_id],
            ..Default::default()
        };
        let change_len = funding(0).witness_utxo.script_pubkey.len();
        let dust = funding(0)
            .witness_utxo
            .script_pubkey
            .minimal_non_dust()
            .to_sat();

        let quote = quote_unilateral_exit(
            &nodes,
            &leaf_ids,
            UnilateralExitLeafFilter::All,
            272,
            change_len,
            dust,
            FEE_RATE,
            change_len,
            &on_chain,
        )
        .unwrap();
        let plan = plan_unilateral_exit(
            nodes,
            &leaf_ids,
            UnilateralExitLeafFilter::All,
            vec![funding(quote.single_utxo_funding_sat)],
            FEE_RATE,
            change_len,
            &on_chain,
        )
        .unwrap();
        let build = build_unilateral_exit(&plan, &on_chain, FEE_RATE).unwrap();

        let stopped_txs = &build.branches[0].txs;
        assert_eq!(stopped_txs.len(), 1);
        assert_eq!(stopped_txs[0].txid, refund.compute_txid());
        assert!(stopped_txs[0].to_sign.is_none());
        assert!(plan.per_branch_funding[0].1.is_empty());

        let sibling_txs = &build.branches[1].txs;
        assert_eq!(sibling_txs.len(), 3, "root, node, refund");
        assert!(sibling_txs[0].to_sign.is_none());
        assert!(sibling_txs[1..].iter().all(|tx| tx.to_sign.is_some()));
        assert_eq!(quote.cpfp_fee_sat, build.cpfp_fee_sat);
        assert_eq!(quote.sweep_fee_sat, sweep_fee(&build, change_len));
        let funded: Vec<&TreeNodeId> = quote.per_branch_funding.iter().map(|(id, _)| id).collect();
        assert_eq!(funded, [&leaf_ids[1]]);
    }

    #[test]
    fn quote_and_build_report_the_fees_of_direct_transactions() {
        let leaf = || node("leaf", Some("root"), anchor_tx(2), Some(refund_tx(3)));
        let confirmed = |node_id: &str, confirmed_by: ExitNodeConfirmation| ConfirmedExitNode {
            node_id: id(node_id),
            confirmed_by,
            block_height: None,
        };

        let nothing_on_chain = (leaf(), ExitChainState::default());

        let direct_refund_on_chain = (
            leaf(),
            ExitChainState {
                nodes: vec![
                    confirmed("leaf", ExitNodeConfirmation::Cpfp),
                    confirmed("root", ExitNodeConfirmation::Cpfp),
                ],
                refunds: vec![ExitRefund {
                    leaf_id: id("leaf"),
                    state: ExitRefundState::OnChain {
                        tx: tx_paying(99_045, 4),
                        vout: 0,
                        value: 99_045,
                        block_height: None,
                    },
                }],
                ..Default::default()
            },
        );

        let mut went_direct = leaf();
        went_direct.direct_tx = Some(tx_paying(99_045, 4));
        went_direct.direct_refund_tx = Some(tx_paying(98_090, 5));
        let direct_tx_on_chain = (
            went_direct,
            ExitChainState {
                nodes: vec![
                    confirmed("leaf", ExitNodeConfirmation::Direct),
                    confirmed("root", ExitNodeConfirmation::Cpfp),
                ],
                ..Default::default()
            },
        );

        let change_len = funding(0).witness_utxo.script_pubkey.len();
        let dust = funding(0)
            .witness_utxo
            .script_pubkey
            .minimal_non_dust()
            .to_sat();
        for ((leaf, on_chain), refund_fee) in [
            (nothing_on_chain, 0),
            (direct_refund_on_chain, 955),
            (direct_tx_on_chain, 1_910),
        ] {
            let nodes = to_node_map(vec![node("root", None, anchor_tx(1), None), leaf]);
            let leaf_id = id("leaf");
            let quote = quote_unilateral_exit(
                &nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::All,
                272,
                change_len,
                dust,
                FEE_RATE,
                change_len,
                &on_chain,
            )
            .unwrap();
            let funding_inputs = if quote.per_branch_funding.is_empty() {
                Vec::new()
            } else {
                vec![funding(quote.single_utxo_funding_sat)]
            };
            let plan = plan_unilateral_exit(
                nodes,
                std::slice::from_ref(&leaf_id),
                UnilateralExitLeafFilter::All,
                funding_inputs,
                FEE_RATE,
                change_len,
                &on_chain,
            )
            .unwrap();
            let build = build_unilateral_exit(&plan, &on_chain, FEE_RATE).unwrap();

            assert_eq!(quote.refund_fee_sat, refund_fee);
            assert_eq!(build.refund_fee_sat, refund_fee);
            let swept: u64 = build.refund_outputs.iter().map(|output| output.value).sum();
            assert_eq!(build.recoverable_value_sat - build.refund_fee_sat, swept);
        }
    }
}

#[cfg(test)]
mod interpret_tests {
    use super::*;
    use bitcoin::{
        Amount, CompressedPublicKey, ScriptBuf, Sequence, TxIn, TxOut, absolute::LockTime,
        hashes::Hash, secp256k1::PublicKey, transaction::Version,
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

    fn prepared_of(root: TreeNode, leaf: TreeNode) -> UnilateralExitPlan {
        let leaf_id = leaf.id.clone();
        UnilateralExitPlan {
            selected_leaves: vec![],
            fan_out_psbt: None,
            per_branch_funding: vec![(leaf_id, vec![])],
            tree_nodes: to_node_map(vec![root, leaf]),
        }
    }

    fn funding(outpoint: OutPoint, value: u64, script: ScriptBuf) -> CpfpInput {
        CpfpInput {
            outpoint,
            witness_utxo: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: script,
            },
            signed_input_weight: 272,
        }
    }

    /// A caller can hand back the outpoint it was given, spent or not: what the
    /// spend produced at the same script is theirs, and is what funds the rest.
    #[test]
    fn scan_funding_follows_a_spend_to_what_it_produced() {
        let script = leaf_addr().script_pubkey();
        let original = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let elsewhere = ScriptBuf::from(vec![0x51]);
        let fan_out = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: original,
                ..Default::default()
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(5_000),
                    script_pubkey: script.clone(),
                },
                TxOut {
                    value: Amount::from_sat(6_000),
                    script_pubkey: script.clone(),
                },
                // Paid away: not the caller's to spend.
                TxOut {
                    value: Amount::from_sat(7_000),
                    script_pubkey: elsewhere,
                },
            ],
        };
        let fan_out_txid = fan_out.compute_txid();
        let branch = |vout| OutPoint {
            txid: fan_out_txid,
            vout,
        };

        let observed = vec![
            spent(original, fan_out_txid),
            Observation {
                query: ChainQuery::Transaction(fan_out_txid),
                result: ChainResult::Transaction(fan_out),
            },
            unspent(branch(0)),
            unspent(branch(1)),
        ];

        let scan = scan_funding(
            std::slice::from_ref(&funding(original, 20_000, script.clone())),
            &observed,
        );

        assert!(scan.pending.is_empty());
        let outpoints: Vec<OutPoint> = scan.inputs.iter().map(|i| i.outpoint).collect();
        assert!(
            !outpoints.contains(&original),
            "the spent outpoint is gone: {outpoints:?}"
        );
        assert_eq!(outpoints.len(), 2, "one per output paying our script");
        assert!(outpoints.contains(&branch(0)) && outpoints.contains(&branch(1)));
        assert!(
            scan.inputs.iter().all(|i| i.signed_input_weight == 272),
            "a descendant signs the way its ancestor did"
        );
    }

    /// The same UTXO reached twice, once given and once traced, is one input.
    #[test]
    fn scan_funding_counts_a_utxo_once() {
        let script = leaf_addr().script_pubkey();
        let original = OutPoint {
            txid: Txid::from_byte_array([2u8; 32]),
            vout: 0,
        };
        let child = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: original,
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(9_000),
                script_pubkey: script.clone(),
            }],
        };
        let child_txid = child.compute_txid();
        let change = OutPoint {
            txid: child_txid,
            vout: 0,
        };

        let observed = vec![
            spent(original, child_txid),
            Observation {
                query: ChainQuery::Transaction(child_txid),
                result: ChainResult::Transaction(child),
            },
            unspent(change),
        ];

        let scan = scan_funding(
            &[
                funding(original, 20_000, script.clone()),
                funding(change, 9_000, script),
            ],
            &observed,
        );

        assert_eq!(
            scan.inputs.len(),
            1,
            "the traced change and the given one are the same UTXO: {:?}",
            scan.inputs
        );
        assert_eq!(scan.inputs[0].outpoint, change);
    }

    /// A spend that cannot be read leaves the outpoint out. It is known spent, so
    /// building over it would double-spend; the exit is under-funded and says so,
    /// rather than producing a package that can never be broadcast.
    #[test]
    fn scan_funding_drops_a_spend_it_cannot_read() {
        let script = leaf_addr().script_pubkey();
        let original = OutPoint {
            txid: Txid::from_byte_array([4u8; 32]),
            vout: 0,
        };
        let spender = Txid::from_byte_array([5u8; 32]);

        let observed = vec![
            spent(original, spender),
            Observation {
                query: ChainQuery::Transaction(spender),
                result: ChainResult::Unavailable,
            },
        ];

        let scan = scan_funding(
            std::slice::from_ref(&funding(original, 20_000, script)),
            &observed,
        );

        assert!(scan.pending.is_empty());
        assert!(scan.inputs.is_empty(), "{:?}", scan.inputs);
    }

    /// Blind, it asks rather than guesses, and holds on to what it was given.
    #[test]
    fn scan_funding_keeps_what_it_cannot_read() {
        let script = leaf_addr().script_pubkey();
        let original = OutPoint {
            txid: Txid::from_byte_array([3u8; 32]),
            vout: 0,
        };

        let scan = scan_funding(
            std::slice::from_ref(&funding(original, 20_000, script)),
            &[],
        );

        assert_eq!(scan.pending, vec![ChainQuery::Outspend(original)]);
        assert_eq!(scan.inputs.len(), 1);
        assert_eq!(scan.inputs[0].outpoint, original);
    }

    fn spent(outpoint: OutPoint, spender: Txid) -> Observation {
        Observation {
            query: ChainQuery::Outspend(outpoint),
            result: ChainResult::Spend(Some(SpendInfo {
                spender_txid: spender,
                confirmed: true,
                block_height: None,
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

    /// A chain of three, the last one in a block. One lookup settles all three:
    /// a transaction in a block puts every one it spends from in a block too.
    #[test]
    fn check_settles_a_chain_from_its_deepest_confirmed() {
        let mut chain = exit_chain_of_three();
        // The caller last saw the whole chain in a block.
        for tx in &mut chain {
            tx.confirmed = true;
        }
        let deepest = chain[2].tx.compute_txid();

        let observed = vec![Observation {
            query: ChainQuery::TxConfirmed(deepest),
            result: ChainResult::Confirmed {
                confirmed: true,
                block_height: None,
            },
        }];
        let check = check_exit_chain(&chain, &observed);

        assert!(check.pending.is_empty(), "{:?}", check.pending);
        assert!(!check.diverged);
        assert_eq!(check.confirmed.len(), 3, "everything above it too");
    }

    /// A transaction that landed since the caller last looked is found, and the
    /// one below it opens in the same pass.
    #[test]
    fn check_finds_what_landed_since_the_caller_last_looked() {
        let chain = exit_chain_of_three();
        let first = chain[0].tx.compute_txid();
        let second = chain[1].tx.compute_txid();

        let observed = vec![
            Observation {
                query: ChainQuery::TxConfirmed(first),
                result: ChainResult::Confirmed {
                    confirmed: true,
                    block_height: None,
                },
            },
            // Broadcast and confirmed since; the caller still has it as pending.
            spent(chain[1].tx.input[0].previous_output, second),
            unspent(chain[2].tx.input[0].previous_output),
        ];
        let check = check_exit_chain(&chain, &observed);

        assert!(check.pending.is_empty(), "{:?}", check.pending);
        assert!(!check.diverged);
        assert!(
            check.confirmed.contains_key(&second),
            "the one that landed is found"
        );
    }

    /// The height a transaction confirmed at reaches the caller. A relative
    /// timelock counts from it, so without it every caller fetches the
    /// transaction again to work out when its child can go out.
    #[test]
    fn check_reports_the_height_a_transaction_confirmed_at() {
        let chain = exit_chain_of_three();
        let first = chain[0].tx.compute_txid();
        let second = chain[1].tx.compute_txid();

        let observed = vec![
            Observation {
                query: ChainQuery::TxConfirmed(first),
                result: ChainResult::Confirmed {
                    confirmed: true,
                    block_height: Some(880_000),
                },
            },
            // Landed since the caller last looked, so its height comes from the
            // outspend that found it rather than from a TxConfirmed.
            Observation {
                query: ChainQuery::Outspend(chain[1].tx.input[0].previous_output),
                result: ChainResult::Spend(Some(SpendInfo {
                    spender_txid: second,
                    confirmed: true,
                    block_height: Some(880_002),
                })),
            },
            unspent(chain[2].tx.input[0].previous_output),
        ];
        let check = check_exit_chain(&chain, &observed);

        assert!(check.pending.is_empty(), "{:?}", check.pending);
        assert_eq!(check.confirmed.get(&first), Some(&Some(880_000)));
        assert_eq!(check.confirmed.get(&second), Some(&Some(880_002)));
    }

    /// The branch's CPFP child spends the fan-out output; the branch transaction
    /// itself does not. Settling had followed the caller's broadcast order, which
    /// lists the fan-out, and ticked it off as confirmed without asking. Told it
    /// is in a block, the caller would never broadcast it.
    #[test]
    fn a_confirmed_branch_does_not_settle_the_fan_out_it_only_waits_for() {
        let mut chain = exit_chain_of_three();
        let fan_out = ExitCheckInput {
            tx: Transaction {
                version: bitcoin::transaction::Version::TWO,
                lock_time: LockTime::ZERO,
                input: vec![bitcoin::TxIn {
                    previous_output: OutPoint {
                        txid: Txid::from_byte_array([0x77; 32]),
                        vout: 0,
                    },
                    ..Default::default()
                }],
                output: vec![],
            },
            cpfp: None,
            confirmed: false,
        };
        let fan_out_txid = fan_out.tx.compute_txid();
        for tx in &mut chain {
            tx.confirmed = true;
        }
        let deepest = chain[2].tx.compute_txid();

        let mut inputs = vec![fan_out];
        inputs.extend(chain);
        let check = check_exit_chain(
            &inputs,
            &[Observation {
                query: ChainQuery::TxConfirmed(deepest),
                result: ChainResult::Confirmed {
                    confirmed: true,
                    block_height: Some(880_002),
                },
            }],
        );

        assert!(
            !check.confirmed.contains_key(&fan_out_txid),
            "the fan-out is not settled by a transaction that never spends it"
        );
        assert!(
            check.pending.contains(&ChainQuery::Outspend(OutPoint {
                txid: Txid::from_byte_array([0x77; 32]),
                vout: 0,
            })),
            "it is asked about instead"
        );
    }

    /// A settled ancestor is never queried, so the check reports it with no
    /// height. The height the caller already had is the only one there is.
    #[test]
    fn settling_along_the_spend_chain_reports_an_ancestor_without_a_height() {
        let mut chain = exit_chain_of_three();
        for tx in &mut chain {
            tx.confirmed = true;
        }
        let first = chain[0].tx.compute_txid();
        let deepest = chain[2].tx.compute_txid();

        let check = check_exit_chain(
            &chain,
            &[Observation {
                query: ChainQuery::TxConfirmed(deepest),
                result: ChainResult::Confirmed {
                    confirmed: true,
                    block_height: Some(880_002),
                },
            }],
        );

        assert_eq!(check.confirmed.get(&first), Some(&None));
    }

    /// A transaction the chain reports as not in a block is listed in
    /// `not_confirmed`, so a caller holding it as confirmed can discard that. A
    /// failed lookup is not listed: it says nothing about where the transaction
    /// is.
    #[test]
    fn check_reports_a_contradicted_confirmation_but_not_an_unreadable_one() {
        let mut chain = exit_chain_of_three();
        chain[0].confirmed = true;
        let first = chain[0].tx.compute_txid();

        let contradicted = check_exit_chain(
            &chain[..1],
            &[Observation {
                query: ChainQuery::TxConfirmed(first),
                result: ChainResult::Confirmed {
                    confirmed: false,
                    block_height: None,
                },
            }],
        );
        assert!(contradicted.not_confirmed.contains(&first));

        let unreadable = check_exit_chain(
            &chain[..1],
            &[Observation {
                query: ChainQuery::TxConfirmed(first),
                result: ChainResult::Unavailable,
            }],
        );
        assert!(
            unreadable.not_confirmed.is_empty(),
            "a failed lookup is not an answer"
        );
    }

    /// A frontier whose own input was taken by somebody else: the exit no longer
    /// matches the chain.
    #[test]
    fn check_reports_an_outpoint_taken_by_another_transaction() {
        let chain = exit_chain_of_three();
        let first = chain[0].tx.compute_txid();
        let frontier_input = chain[1].tx.input[0].previous_output;
        let stranger = Txid::from_byte_array([0xaa; 32]);

        let observed = vec![
            Observation {
                query: ChainQuery::TxConfirmed(first),
                result: ChainResult::Confirmed {
                    confirmed: true,
                    block_height: None,
                },
            },
            spent(frontier_input, stranger),
        ];
        let check = check_exit_chain(&chain[..2], &observed);

        assert!(check.diverged, "a stranger holds the outpoint it needs");
    }

    /// The check finds a transaction in a block whose anchor is spent by
    /// another transaction. The next CPFP child has the change of this
    /// transaction's own child as an input, so the exit can no longer be
    /// finished.
    #[test]
    fn check_reports_an_anchor_bumped_by_another_child() {
        let chain = exit_chain_with_children();
        let frontier = chain[1].tx.compute_txid();
        let anchor = anchor_of(&chain[1].tx);
        let stranger = Txid::from_byte_array([0xbb; 32]);

        let observed = vec![
            in_block(chain[0].cpfp.as_ref().unwrap().compute_txid(), true),
            in_block(chain[0].tx.compute_txid(), true),
            spent(chain[1].tx.input[0].previous_output, frontier),
            spent(anchor, stranger),
            unspent(chain[2].tx.input[0].previous_output),
        ];
        let check = check_exit_chain(&chain, &observed);

        assert!(check.pending.is_empty(), "{:?}", check.pending);
        assert!(check.diverged, "somebody else bumped the anchor");
    }

    fn in_block(txid: Txid, confirmed: bool) -> Observation {
        Observation {
            query: ChainQuery::TxConfirmed(txid),
            result: ChainResult::Confirmed {
                confirmed,
                block_height: None,
            },
        }
    }

    fn with_inputs(inputs: &[OutPoint], nonce: u32) -> Transaction {
        let mut tx = tx_spending(inputs[0], nonce);
        tx.input.extend(inputs[1..].iter().map(|outpoint| TxIn {
            previous_output: *outpoint,
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            ..Default::default()
        }));
        tx
    }

    fn anchor_of(tx: &Transaction) -> OutPoint {
        OutPoint {
            txid: tx.compute_txid(),
            vout: 1,
        }
    }

    fn change_of(cpfp: &Transaction) -> OutPoint {
        OutPoint {
            txid: cpfp.compute_txid(),
            vout: 0,
        }
    }

    fn funding_outpoint(vout: u32) -> OutPoint {
        OutPoint {
            txid: Txid::from_byte_array([0x66; 32]),
            vout,
        }
    }

    /// The chain of [`exit_chain_of_three`], each transaction with a CPFP child.
    /// The first child has a funding outpoint as an input, each later one the
    /// change of the child before it.
    fn exit_chain_with_children() -> Vec<ExitCheckInput> {
        let mut chain = exit_chain_of_three();
        let mut funding = funding_outpoint(0);
        for (nonce, input) in (90..).zip(&mut chain) {
            let cpfp = with_inputs(&[funding, anchor_of(&input.tx)], nonce);
            funding = change_of(&cpfp);
            input.cpfp = Some(cpfp);
        }
        chain
    }

    /// Runs the check until it returns no query, taking the result of each
    /// query from `results` and failing one that has none there. Returns the
    /// last check and every query it executed, in order.
    fn run_check(txs: &[ExitCheckInput], results: &[Observation]) -> (ExitCheck, Vec<ChainQuery>) {
        let mut observed: Vec<Observation> = Vec::new();
        loop {
            let check = check_exit_chain(txs, &observed);
            if check.pending.is_empty() {
                let executed = observed.into_iter().map(|o| o.query).collect();
                return (check, executed);
            }
            for query in check.pending {
                let result = results
                    .iter()
                    .find(|o| o.query == query)
                    .map_or(ChainResult::Unavailable, |o| o.result.clone());
                observed.push(Observation { query, result });
            }
        }
    }

    /// When a transaction is in a block, so is every one above it, so the check
    /// queries the deepest one the caller recorded and none above it.
    #[test]
    fn check_queries_only_the_deepest_recorded_transaction() {
        let mut chain = exit_chain_of_three();
        for tx in &mut chain {
            tx.confirmed = true;
        }
        let deepest = chain[2].tx.compute_txid();

        let (check, executed) = run_check(&chain, &[in_block(deepest, true)]);

        assert_eq!(executed, vec![ChainQuery::TxConfirmed(deepest)]);
        assert_eq!(check.confirmed.len(), 3);
    }

    /// The deepest recorded transaction is in no block, so the check queries
    /// the one above it next.
    #[test]
    fn check_steps_up_from_a_recorded_transaction_that_is_in_no_block() {
        let mut chain = exit_chain_of_three();
        for tx in &mut chain {
            tx.confirmed = true;
        }
        let [first, second, third] = [0, 1, 2].map(|i| chain[i].tx.compute_txid());
        let third_input = chain[2].tx.input[0].previous_output;

        let (check, executed) = run_check(
            &chain,
            &[
                in_block(third, false),
                in_block(second, true),
                unspent(third_input),
            ],
        );

        assert_eq!(
            executed,
            vec![
                ChainQuery::TxConfirmed(third),
                ChainQuery::TxConfirmed(second),
                ChainQuery::Outspend(third_input),
            ]
        );
        assert!(check.not_confirmed.contains(&third));
        assert!(check.confirmed.contains_key(&first));
    }

    /// The chain service fails the lookup of the deepest recorded transaction.
    /// The check then queries nothing above it and reports no transaction in or
    /// out of a block.
    #[test]
    fn check_queries_nothing_above_a_failed_lookup() {
        let mut chain = exit_chain_of_three();
        for tx in &mut chain {
            tx.confirmed = true;
        }
        let deepest = chain[2].tx.compute_txid();

        let (check, executed) = run_check(&chain, &[]);

        assert_eq!(executed, vec![ChainQuery::TxConfirmed(deepest)]);
        assert!(check.confirmed.is_empty());
        assert!(check.not_confirmed.is_empty());
        assert!(check.unverified.is_empty());
    }

    /// Below two transactions in a block, one still has to confirm. The check
    /// queries the deepest transaction in a block and its CPFP child, then the
    /// outpoint of the input of the one that still has to confirm and the
    /// funding of its CPFP child.
    #[test]
    fn check_returns_four_queries_for_the_next_transaction() {
        let mut chain = exit_chain_with_children();
        chain[1].confirmed = true;
        let second_child = chain[1].cpfp.as_ref().unwrap();
        let third_input = chain[2].tx.input[0].previous_output;

        let second = chain[1].tx.compute_txid();

        let (check, executed) = run_check(
            &chain,
            &[
                in_block(second_child.compute_txid(), true),
                in_block(second, true),
                unspent(third_input),
                unspent(change_of(second_child)),
            ],
        );

        assert_eq!(executed.len(), 4);
        assert_eq!(
            executed.into_iter().collect::<HashSet<_>>(),
            HashSet::from([
                ChainQuery::TxConfirmed(second_child.compute_txid()),
                ChainQuery::TxConfirmed(second),
                ChainQuery::Outspend(third_input),
                ChainQuery::Outspend(change_of(second_child)),
            ])
        );
        assert!(!check.diverged);
    }

    /// The caller stores the statuses the check returns, so on the next call the
    /// check starts from a transaction the caller recorded in a block. The
    /// anchor of that transaction is spent by another CPFP child.
    #[test]
    fn check_reports_a_bumped_anchor_of_a_transaction_recorded_in_a_block() {
        let mut chain = exit_chain_with_children();
        chain[1].confirmed = true;
        let [first, second, third] = [&chain[0], &chain[1], &chain[2]];
        let first_child = first.cpfp.as_ref().unwrap();
        let stranger = Txid::from_byte_array([0xbb; 32]);

        let (check, _) = run_check(
            &chain,
            &[
                in_block(second.cpfp.as_ref().unwrap().compute_txid(), false),
                in_block(second.tx.compute_txid(), true),
                in_block(first_child.compute_txid(), true),
                unspent(change_of(first_child)),
                spent(anchor_of(&second.tx), stranger),
                unspent(third.tx.input[0].previous_output),
            ],
        );

        assert!(check.diverged);
    }

    /// The funding of a CPFP child is spent by another transaction while the
    /// child's parent is in no block.
    #[test]
    fn check_reports_funding_of_a_cpfp_child_spent_by_another_transaction() {
        let chain = exit_chain_with_children();
        let first_child = chain[0].cpfp.as_ref().unwrap();
        let funding = change_of(first_child);
        let stranger = Txid::from_byte_array([0xcc; 32]);

        let (check, _) = run_check(
            &chain,
            &[
                in_block(first_child.compute_txid(), true),
                in_block(chain[0].tx.compute_txid(), true),
                unspent(chain[1].tx.input[0].previous_output),
                spent(funding, stranger),
            ],
        );

        assert!(check.diverged);
    }

    /// The change that is an input of a CPFP child does not exist while the
    /// child before it is in no block, so the check does not query its spend.
    /// The child before it still has to confirm, so the check queries the spend
    /// of that child's own funding.
    #[test]
    fn check_skips_funding_that_does_not_exist_yet() {
        let chain = exit_chain_with_children();
        let first_child = chain[0].cpfp.as_ref().unwrap();

        let (check, executed) = run_check(
            &chain,
            &[
                in_block(first_child.compute_txid(), false),
                in_block(chain[0].tx.compute_txid(), true),
                unspent(funding_outpoint(0)),
                unspent(anchor_of(&chain[0].tx)),
                unspent(chain[1].tx.input[0].previous_output),
            ],
        );

        assert!(
            !executed.contains(&ChainQuery::Outspend(change_of(first_child))),
            "{executed:?}"
        );
        assert!(executed.contains(&ChainQuery::Outspend(funding_outpoint(0))));
        assert!(!check.diverged);
    }

    /// A transaction and its CPFP child confirmed after the caller last checked.
    /// The check finds both, and reads the funding of the next child.
    #[test]
    fn check_finds_a_cpfp_child_that_confirmed_with_its_parent() {
        let chain = exit_chain_with_children();
        let second = chain[1].tx.compute_txid();
        let second_child = chain[1].cpfp.as_ref().unwrap();

        let mut observed = vec![
            in_block(chain[0].cpfp.as_ref().unwrap().compute_txid(), true),
            in_block(chain[0].tx.compute_txid(), true),
            spent(chain[1].tx.input[0].previous_output, second),
            spent(anchor_of(&chain[1].tx), second_child.compute_txid()),
            unspent(chain[2].tx.input[0].previous_output),
        ];
        let check = check_exit_chain(&chain, &observed);
        assert_eq!(
            check.pending,
            vec![ChainQuery::Outspend(change_of(second_child))]
        );

        observed.push(unspent(change_of(second_child)));
        let check = check_exit_chain(&chain, &observed);

        assert!(check.pending.is_empty(), "{:?}", check.pending);
        assert!(!check.diverged);
        assert!(check.confirmed.contains_key(&second));
        assert!(check.confirmed.contains_key(&second_child.compute_txid()));
    }

    /// A node transaction with a CPFP child, the refund below it already in a
    /// block when the wallet built the exit, and the sweep. No transaction of
    /// the exit has the child's change as an input.
    fn exit_with_a_cpfp_child_whose_change_is_not_an_input() -> Vec<ExitCheckInput> {
        let node = tx_spending(funding_outpoint(7), 1);
        let child = with_inputs(&[funding_outpoint(0), anchor_of(&node)], 90);
        let refund = tx_spending(
            OutPoint {
                txid: node.compute_txid(),
                vout: 0,
            },
            2,
        );
        let sweep = tx_spending(
            OutPoint {
                txid: refund.compute_txid(),
                vout: 0,
            },
            3,
        );
        vec![
            ExitCheckInput {
                tx: node,
                cpfp: Some(child),
                confirmed: false,
            },
            ExitCheckInput {
                tx: refund,
                cpfp: None,
                confirmed: true,
            },
            ExitCheckInput {
                tx: sweep,
                cpfp: None,
                confirmed: false,
            },
        ]
    }

    /// No transaction of the exit has the change of the node transaction's
    /// CPFP child as an input. Once the node transaction is in a block, the exit
    /// can be finished whichever child confirmed it, so the check does not query
    /// the anchor.
    #[test]
    fn check_ignores_a_cpfp_child_whose_change_is_not_an_input() {
        let txs = exit_with_a_cpfp_child_whose_change_is_not_an_input();
        let sweep_input = txs[2].tx.input[0].previous_output;

        let (check, executed) = run_check(
            &txs,
            &[
                in_block(txs[1].tx.compute_txid(), true),
                unspent(sweep_input),
            ],
        );

        assert!(!executed.contains(&ChainQuery::Outspend(anchor_of(&txs[0].tx))));
        assert!(!check.diverged);
    }

    /// The same once the caller recorded the node transaction in a block.
    #[test]
    fn check_ignores_such_a_child_of_a_recorded_transaction() {
        let mut txs = exit_with_a_cpfp_child_whose_change_is_not_an_input();
        txs[0].confirmed = true;
        let sweep_input = txs[2].tx.input[0].previous_output;

        let (check, executed) = run_check(
            &txs,
            &[
                in_block(txs[1].tx.compute_txid(), true),
                unspent(sweep_input),
            ],
        );

        assert!(!executed.contains(&ChainQuery::Outspend(anchor_of(&txs[0].tx))));
        assert!(!check.diverged);
    }

    /// The check queries a recorded transaction along with its CPFP child, and
    /// reports the block height of the transaction's own result.
    #[test]
    fn check_reports_the_height_of_a_transaction_whose_child_is_in_a_block() {
        let mut chain = exit_chain_with_children();
        chain[1].confirmed = true;
        let second = chain[1].tx.compute_txid();
        let second_child = chain[1].cpfp.as_ref().unwrap();

        let (check, _) = run_check(
            &chain,
            &[
                in_block(second_child.compute_txid(), true),
                Observation {
                    query: ChainQuery::TxConfirmed(second),
                    result: ChainResult::Confirmed {
                        confirmed: true,
                        block_height: Some(880_000),
                    },
                },
            ],
        );

        assert_eq!(check.confirmed.get(&second), Some(&Some(880_000)));
    }

    /// Two branches below one transaction. The chain service fails the lookup
    /// for the first branch. The check still finds the transaction of the
    /// second branch that confirmed after the caller last checked, and with it
    /// the transaction above both branches.
    #[test]
    fn check_reads_a_branch_next_to_one_whose_lookup_failed() {
        let shared = tx_spending(funding_outpoint(7), 1);
        let shared_txid = shared.compute_txid();
        let [first_branch, second_branch] = [0, 1].map(|vout| {
            tx_spending(
                OutPoint {
                    txid: shared_txid,
                    vout,
                },
                2 + vout,
            )
        });
        let second_branch_input = second_branch.input[0].previous_output;
        let second_branch_txid = second_branch.compute_txid();
        let txs = vec![
            ExitCheckInput {
                tx: shared,
                cpfp: None,
                confirmed: true,
            },
            ExitCheckInput {
                tx: first_branch.clone(),
                cpfp: None,
                confirmed: true,
            },
            ExitCheckInput {
                tx: second_branch,
                cpfp: None,
                confirmed: false,
            },
        ];

        let (check, executed) = run_check(&txs, &[spent(second_branch_input, second_branch_txid)]);

        assert_eq!(
            executed,
            vec![
                ChainQuery::TxConfirmed(first_branch.compute_txid()),
                ChainQuery::Outspend(second_branch_input),
            ]
        );
        assert!(check.confirmed.contains_key(&second_branch_txid));
        assert!(check.confirmed.contains_key(&shared_txid));
    }

    /// The inputs of the sweep are the refund output and the change of the
    /// refund's CPFP child. That change is spent by another transaction.
    #[test]
    fn check_reads_the_change_among_the_sweep_inputs() {
        let refund = tx_spending(funding_outpoint(7), 1);
        let child = with_inputs(&[funding_outpoint(0), anchor_of(&refund)], 90);
        let refund_output = OutPoint {
            txid: refund.compute_txid(),
            vout: 0,
        };
        let sweep = with_inputs(&[refund_output, change_of(&child)], 3);
        let stranger = Txid::from_byte_array([0xee; 32]);
        let txs = vec![
            ExitCheckInput {
                tx: refund.clone(),
                cpfp: Some(child.clone()),
                confirmed: true,
            },
            ExitCheckInput {
                tx: sweep.clone(),
                cpfp: None,
                confirmed: false,
            },
        ];

        let (check, _) = run_check(
            &txs,
            &[
                in_block(child.compute_txid(), true),
                in_block(refund.compute_txid(), true),
                unspent(refund_output),
                spent(change_of(&child), stranger),
            ],
        );

        assert!(check.diverged);
    }

    /// The inputs of the sweep are the refund outputs of two leaves. The second
    /// one is spent by another transaction.
    #[test]
    fn check_reads_every_input_of_a_transaction() {
        let refunds = [
            tx_spending(funding_outpoint(0), 1),
            tx_spending(funding_outpoint(1), 2),
        ];
        let refund_outputs = refunds.clone().map(|refund| OutPoint {
            txid: refund.compute_txid(),
            vout: 0,
        });
        let sweep = with_inputs(&refund_outputs, 3);
        let stranger = Txid::from_byte_array([0xdd; 32]);
        let mut txs: Vec<ExitCheckInput> = refunds
            .iter()
            .map(|refund| ExitCheckInput {
                tx: refund.clone(),
                cpfp: None,
                confirmed: true,
            })
            .collect();
        txs.push(ExitCheckInput {
            tx: sweep.clone(),
            cpfp: None,
            confirmed: false,
        });

        let observed = vec![
            in_block(refunds[0].compute_txid(), true),
            in_block(refunds[1].compute_txid(), true),
            unspent(refund_outputs[0]),
            spent(refund_outputs[1], stranger),
        ];
        let check = check_exit_chain(&txs, &observed);

        assert!(check.pending.is_empty(), "{:?}", check.pending);
        assert!(check.diverged);
    }

    /// The inputs of the sweep are the refund outputs of two leaves. One refund
    /// is in a block, and its output is spent by another transaction, while the
    /// other refund is in no block yet.
    #[test]
    fn check_reads_an_input_while_another_input_does_not_exist() {
        let refunds = [
            tx_spending(funding_outpoint(0), 1),
            tx_spending(funding_outpoint(1), 2),
        ];
        let refund_outputs = refunds.clone().map(|refund| OutPoint {
            txid: refund.compute_txid(),
            vout: 0,
        });
        let sweep = with_inputs(&refund_outputs, 3);
        let stranger = Txid::from_byte_array([0xdd; 32]);
        let txs = vec![
            ExitCheckInput {
                tx: refunds[0].clone(),
                cpfp: None,
                confirmed: true,
            },
            ExitCheckInput {
                tx: refunds[1].clone(),
                cpfp: None,
                confirmed: false,
            },
            ExitCheckInput {
                tx: sweep,
                cpfp: None,
                confirmed: false,
            },
        ];

        let (check, _) = run_check(
            &txs,
            &[
                in_block(refunds[0].compute_txid(), true),
                unspent(funding_outpoint(1)),
                spent(refund_outputs[0], stranger),
            ],
        );

        assert!(check.diverged);
    }

    /// The first CPFP child of each branch has an output of the fan-out as an
    /// input. The fan-out output for the CPFP child of the second branch's leaf
    /// node is spent by another transaction, while the node above both branches
    /// is in no block.
    #[test]
    fn check_reads_the_funding_of_a_child_below_a_shared_ancestor() {
        let fan_out = tx_spending(funding_outpoint(0), 1);
        let shared = tx_spending(funding_outpoint(7), 2);
        let first_leaf = tx_spending(
            OutPoint {
                txid: shared.compute_txid(),
                vout: 0,
            },
            3,
        );
        let second_leaf = tx_spending(
            OutPoint {
                txid: shared.compute_txid(),
                vout: 2,
            },
            4,
        );
        let fan_out_output = |vout| OutPoint {
            txid: fan_out.compute_txid(),
            vout,
        };
        let shared_child = with_inputs(&[fan_out_output(0), anchor_of(&shared)], 90);
        let first_leaf_child = with_inputs(&[change_of(&shared_child), anchor_of(&first_leaf)], 91);
        let second_leaf_child = with_inputs(&[fan_out_output(1), anchor_of(&second_leaf)], 92);
        let stranger = Txid::from_byte_array([0xcc; 32]);
        let txs = vec![
            ExitCheckInput {
                tx: fan_out.clone(),
                cpfp: None,
                confirmed: true,
            },
            ExitCheckInput {
                tx: shared,
                cpfp: Some(shared_child),
                confirmed: false,
            },
            ExitCheckInput {
                tx: first_leaf,
                cpfp: Some(first_leaf_child),
                confirmed: false,
            },
            ExitCheckInput {
                tx: second_leaf,
                cpfp: Some(second_leaf_child),
                confirmed: false,
            },
        ];

        let (check, _) = run_check(
            &txs,
            &[
                in_block(fan_out.compute_txid(), true),
                unspent(funding_outpoint(7)),
                unspent(fan_out_output(0)),
                spent(fan_out_output(1), stranger),
            ],
        );

        assert!(check.diverged);
    }

    /// The caller recorded the first transaction in a block, and another CPFP
    /// child of that transaction is in the same block. The chain service fails
    /// the confirmation query of the exit's own CPFP child, so the check reads
    /// that child's inputs.
    #[test]
    fn check_reads_the_inputs_of_a_cpfp_child_whose_lookup_failed() {
        let chain = exit_chain_with_children();
        let stranger = Txid::from_byte_array([0xbb; 32]);

        let (check, _) = run_check(
            &chain,
            &[
                in_block(chain[0].tx.compute_txid(), true),
                spent(anchor_of(&chain[0].tx), stranger),
                unspent(funding_outpoint(0)),
                unspent(chain[1].tx.input[0].previous_output),
            ],
        );

        assert!(check.diverged);
    }

    /// The caller recorded a node transaction in a block, but not the fan-out
    /// that funds its CPFP child. The chain service fails the confirmation
    /// query of that child, and the input of the fan-out is spent by another
    /// transaction.
    #[test]
    fn check_reads_a_fan_out_the_caller_did_not_record() {
        let caller_utxo = funding_outpoint(5);
        let fan_out = tx_spending(caller_utxo, 50);
        let node = tx_spending(funding_outpoint(7), 1);
        let node_output = OutPoint {
            txid: node.compute_txid(),
            vout: 0,
        };
        let child = with_inputs(
            &[
                OutPoint {
                    txid: fan_out.compute_txid(),
                    vout: 0,
                },
                anchor_of(&node),
            ],
            90,
        );
        let next = tx_spending(node_output, 2);
        let next_child = with_inputs(&[change_of(&child), anchor_of(&next)], 91);
        let stranger = Txid::from_byte_array([0xab; 32]);
        let txs = vec![
            ExitCheckInput {
                tx: fan_out,
                cpfp: None,
                confirmed: false,
            },
            ExitCheckInput {
                tx: node.clone(),
                cpfp: Some(child),
                confirmed: true,
            },
            ExitCheckInput {
                tx: next,
                cpfp: Some(next_child),
                confirmed: false,
            },
        ];

        let (check, _) = run_check(
            &txs,
            &[
                in_block(node.compute_txid(), true),
                unspent(node_output),
                spent(caller_utxo, stranger),
            ],
        );

        assert!(check.diverged);
    }

    /// The chain service fails the query for the funding of the next CPFP
    /// child, so the check reports that child's parent as unverified.
    #[test]
    fn check_reports_a_transaction_it_could_not_read_as_unverified() {
        let chain = exit_chain_with_children();

        let (check, _) = run_check(
            &chain,
            &[
                in_block(chain[0].cpfp.as_ref().unwrap().compute_txid(), true),
                in_block(chain[0].tx.compute_txid(), true),
                unspent(chain[1].tx.input[0].previous_output),
            ],
        );

        assert!(!check.diverged);
        assert_eq!(
            check.unverified,
            HashSet::from([chain[1].tx.compute_txid()])
        );
    }

    /// The child of a refund without an anchor has the refund output as its
    /// input, and the sweep has the child's output. The caller recorded the
    /// refund in a block, where its output is spent by another transaction.
    #[test]
    fn check_reports_another_spend_of_the_output_of_an_anchorless_refund() {
        let refund = tx_spending(funding_outpoint(7), 1);
        let refund_output = OutPoint {
            txid: refund.compute_txid(),
            vout: 0,
        };
        let child = tx_spending(refund_output, 90);
        let sweep = tx_spending(change_of(&child), 3);
        let stranger = Txid::from_byte_array([0xee; 32]);
        let txs = vec![
            ExitCheckInput {
                tx: refund.clone(),
                cpfp: Some(child.clone()),
                confirmed: true,
            },
            ExitCheckInput {
                tx: sweep,
                cpfp: None,
                confirmed: false,
            },
        ];

        let (check, _) = run_check(
            &txs,
            &[
                in_block(child.compute_txid(), false),
                in_block(refund.compute_txid(), true),
                spent(refund_output, stranger),
            ],
        );

        assert!(check.diverged);
    }

    /// Below an unconfirmed transaction there is no outpoint yet for anything to
    /// take, so nothing there is asked about.
    #[test]
    fn check_asks_nothing_below_the_frontier() {
        let chain = exit_chain_of_three();
        let first = chain[0].tx.compute_txid();

        let observed = vec![Observation {
            query: ChainQuery::TxConfirmed(first),
            result: ChainResult::Confirmed {
                confirmed: true,
                block_height: None,
            },
        }];
        let check = check_exit_chain(&chain, &observed);

        let asked: Vec<OutPoint> = check
            .pending
            .iter()
            .filter_map(|q| match q {
                ChainQuery::Outspend(outpoint) => Some(*outpoint),
                _ => None,
            })
            .collect();
        let deepest_input = chain[2].tx.input[0].previous_output;
        assert!(
            !asked.contains(&deepest_input),
            "the one below the frontier was asked about: {asked:?}"
        );
    }

    /// Three transactions, each spending the one before, the first recorded in a
    /// block and the rest not.
    fn exit_chain_of_three() -> Vec<ExitCheckInput> {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([5u8; 32]),
            vout: 0,
        };
        let first = tx_spending(deposit, 1);
        let second = tx_spending(
            OutPoint {
                txid: first.compute_txid(),
                vout: 0,
            },
            2,
        );
        let third = tx_spending(
            OutPoint {
                txid: second.compute_txid(),
                vout: 0,
            },
            3,
        );
        vec![
            ExitCheckInput {
                cpfp: None,
                confirmed: true,
                tx: first.clone(),
            },
            ExitCheckInput {
                cpfp: None,
                confirmed: false,
                tx: second.clone(),
            },
            ExitCheckInput {
                cpfp: None,
                confirmed: false,
                tx: third,
            },
        ]
    }

    /// The deepest node the operators label on-chain is where the walk picks up,
    /// once it has checked that node itself. Everything above it is in a block by
    /// the same spend chain, so it costs no lookups at all.
    #[test]
    fn scan_resumes_below_the_deepest_confirmed_node() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([7u8; 32]),
            vout: 0,
        };
        let root = treenode("root", None, tx_spending(deposit, 1), 0);
        let root_out = OutPoint {
            txid: root.node_tx.compute_txid(),
            vout: 0,
        };
        let mut mid = treenode("mid", Some("root"), tx_spending(root_out, 2), 0);
        mid.status = TreeNodeStatus::OnChain;
        let mid_txid = mid.node_tx.compute_txid();
        let mid_out = OutPoint {
            txid: mid_txid,
            vout: 0,
        };
        let leaf = treenode("leaf", Some("mid"), tx_spending(mid_out, 3), 0);
        let leaf_id = leaf.id.clone();
        let tree = to_node_map(vec![root, mid, leaf]);
        let addresses = HashMap::new();

        let confirmed = |txid, yes| Observation {
            query: ChainQuery::TxConfirmed(txid),
            result: ChainResult::Confirmed {
                confirmed: yes,
                block_height: None,
            },
        };

        let observed = vec![confirmed(mid_txid, true), unspent(mid_out)];
        let scan = scan_exit_chain(&tree, std::slice::from_ref(&leaf_id), &addresses, &observed);

        assert!(scan.pending.is_empty(), "{:?}", scan.pending);
        let confirmed_ids: Vec<String> = scan
            .state
            .nodes
            .iter()
            .map(|n| n.node_id.to_string())
            .collect();
        assert_eq!(
            confirmed_ids,
            vec!["mid".to_string(), "root".to_string()],
            "the node checked and everything above it"
        );
        // The deposit was never probed: one node in a block puts every one above
        // it in a block too.
        assert!(!observed_contains(
            &observed,
            &ChainQuery::Outspend(deposit)
        ));
    }

    /// A label a reorg undid sends the walk up to the parent, and on up for as
    /// long as the answer is no.
    #[test]
    fn scan_steps_up_when_the_deepest_confirmed_node_is_gone() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([8u8; 32]),
            vout: 0,
        };
        let mut root = treenode("root", None, tx_spending(deposit, 1), 0);
        root.status = TreeNodeStatus::OnChain;
        let root_txid = root.node_tx.compute_txid();
        let root_out = OutPoint {
            txid: root_txid,
            vout: 0,
        };
        let mut mid = treenode("mid", Some("root"), tx_spending(root_out, 2), 0);
        mid.status = TreeNodeStatus::OnChain;
        let mid_txid = mid.node_tx.compute_txid();
        let leaf = treenode(
            "leaf",
            Some("mid"),
            tx_spending(
                OutPoint {
                    txid: mid_txid,
                    vout: 0,
                },
                3,
            ),
            0,
        );
        let leaf_id = leaf.id.clone();
        let tree = to_node_map(vec![root, mid, leaf]);
        let addresses = HashMap::new();

        let confirmed = |txid, yes| Observation {
            query: ChainQuery::TxConfirmed(txid),
            result: ChainResult::Confirmed {
                confirmed: yes,
                block_height: None,
            },
        };

        // mid is labelled on-chain but is not in a block; root still is.
        let observed = vec![
            confirmed(mid_txid, false),
            confirmed(root_txid, true),
            unspent(root_out),
        ];
        let scan = scan_exit_chain(&tree, std::slice::from_ref(&leaf_id), &addresses, &observed);

        assert!(scan.pending.is_empty(), "{:?}", scan.pending);
        let confirmed_ids: Vec<String> = scan
            .state
            .nodes
            .iter()
            .map(|n| n.node_id.to_string())
            .collect();
        assert_eq!(
            confirmed_ids,
            vec!["root".to_string()],
            "only what is still in a block"
        );
    }

    fn observed_contains(observed: &[Observation], query: &ChainQuery) -> bool {
        observed.iter().any(|o| &o.query == query)
    }

    /// The scan answers what the chain has already done from the tree alone: no
    /// plan, no funding, and the refund address read off the leaf's own
    /// refund_tx rather than derived from the signer.
    #[test]
    fn scan_reads_a_landed_refund_without_any_funding() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([7u8; 32]),
            vout: 0,
        };
        let root = treenode("root", None, tx_spending(deposit, 1), 0);
        let root_out = OutPoint {
            txid: root.node_tx.compute_txid(),
            vout: 0,
        };
        let mut leaf = treenode("leaf", Some("root"), tx_spending(root_out, 2), 0);
        let leaf_out = OutPoint {
            txid: leaf.node_tx.compute_txid(),
            vout: 0,
        };
        let mut refund_tx = tx_spending(leaf_out, 3);
        refund_tx.output[0].script_pubkey = leaf_addr().script_pubkey();
        leaf.refund_tx = Some(refund_tx.clone());
        let leaf_id = leaf.id.clone();
        let refund_txid = refund_tx.compute_txid();
        let tree = to_node_map(vec![root, leaf]);

        // A refund on-chain and unspent: the three lookups it takes to adopt it.
        let observed = vec![
            refund_scan(&leaf_id, refund_txid, 55_000),
            unspent(OutPoint {
                txid: refund_txid,
                vout: 0,
            }),
            Observation {
                query: ChainQuery::Transaction(refund_txid),
                result: ChainResult::Transaction(refund_tx),
            },
        ];

        let addresses = leaf_refund_addresses(
            &tree,
            std::slice::from_ref(&leaf_id),
            bitcoin::Network::Regtest,
        );
        let scan = scan_exit_chain(&tree, std::slice::from_ref(&leaf_id), &addresses, &observed);
        match scan.state.refunds.as_slice() {
            [refund] => {
                assert_eq!(refund.leaf_id, leaf_id);
                assert!(matches!(refund.state, ExitRefundState::OnChain { .. }));
            }
            other => panic!("expected one landed refund, got {other:?}"),
        }

        // Blind, it reports nothing and asks the chain instead, so an unreadable
        // chain understates what is done rather than overstating it.
        let blind = scan_exit_chain(&tree, std::slice::from_ref(&leaf_id), &addresses, &[]);
        assert!(blind.state.refunds.is_empty());
        assert!(blind.state.nodes.is_empty());
        assert!(!blind.pending.is_empty());
    }

    /// Production scans the tree first and hands the result to the build, so the
    /// tests interpret the same way: one pass over the observations they set up.
    fn scan_of(plan: &UnilateralExitPlan, observed: &[Observation]) -> ExitChainScan {
        let leaf_ids: Vec<TreeNodeId> = plan
            .per_branch_funding
            .iter()
            .map(|(leaf_id, _)| leaf_id.clone())
            .collect();
        scan_exit_chain(
            &plan.tree_nodes,
            &leaf_ids,
            &leaf_ids
                .iter()
                .map(|id| (id.clone(), leaf_addr()))
                .collect(),
            observed,
        )
    }

    fn interpret_chain(plan: &UnilateralExitPlan, observed: &[Observation]) -> ChainInterpretation {
        super::interpret_chain(plan, &scan_of(plan, observed).state)
    }

    /// Every lookup reading the tree still needs. The build asks for none: its
    /// funding was resolved before the plan was made.
    fn next_chain_queries(
        plan: &UnilateralExitPlan,
        observed: &[Observation],
    ) -> Result<Vec<ChainQuery>, SparkWalletError> {
        Ok(scan_of(plan, observed).pending)
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
                block_height: None,
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
                block_height: None,
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
    fn walk_takes_the_parent_index_from_the_node_tx_not_the_reported_vout() {
        let deposit = OutPoint {
            txid: Txid::from_byte_array([1u8; 32]),
            vout: 0,
        };
        let root_tx = tx_spending(deposit, 1);
        let root_txid = root_tx.compute_txid();
        let root = treenode("root", None, root_tx, 0);

        // An operator that renewed a timelock without rewriting the index still
        // reports the one the leaf held under its previous parent, while its
        // transaction spends the new parent's output 1.
        let spent_index = OutPoint {
            txid: root_txid,
            vout: 1,
        };
        let leaf = treenode("leaf", Some("root"), tx_spending(spent_index, 2), 0);
        let prepared = prepared_of(root, leaf);

        let scan = scan_of(&prepared, &[spent(deposit, root_txid)]);

        assert!(
            scan.pending.contains(&ChainQuery::Outspend(spent_index)),
            "the walk follows the index the transaction spends"
        );
        assert!(
            !scan.pending.contains(&ChainQuery::Outspend(OutPoint {
                txid: root_txid,
                vout: 0,
            })),
            "the reported index is not followed"
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
        let interp = interpret_chain(&prepared, &observed);

        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp { block_height: None }),
            "the leaf went direct, so the root's cpfp change is never resolved"
        );
        assert_eq!(
            interp.resolved.nodes.get(&leaf_id),
            Some(&NodeState::ConfirmedDirect { block_height: None })
        );
        assert!(matches!(
            interp.resolved.refunds.get(&leaf_id),
            Some(RefundState::DriveDirect)
        ));
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
        let interp = interpret_chain(&prepared, &observed);

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
    fn interpret_ignores_a_spender_that_is_only_in_the_mempool() {
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
        let leaf_tx = tx_spending(leaf_parent_out, 2);
        let leaf_txid = leaf_tx.compute_txid();
        let leaf_direct = tx_spending(leaf_parent_out, 3);
        let leaf_direct_txid = leaf_direct.compute_txid();
        let mut leaf = treenode("leaf", Some("root"), leaf_tx, 0);
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

        // The leaf's node tx, its direct tx, then a transaction that is not the
        // leaf's.
        let foreign_txid = Txid::from_byte_array([9u8; 32]);
        for spender in [leaf_txid, leaf_direct_txid, foreign_txid] {
            let observed = vec![
                spent(deposit, root_txid),
                spent_unconfirmed(leaf_parent_out, spender),
                no_refund(&leaf_id),
            ];
            let interp = interpret_chain(&prepared, &observed);

            assert!(!interp.resolved.nodes.contains_key(&leaf_id));
            assert!(!interp.resolved.refunds.contains_key(&leaf_id));
            assert!(!interp.resolved.stopped.contains(&leaf_id));
        }
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
        let interp = interpret_chain(&prepared, &observed);

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
        let interp = interpret_chain(&prepared, &observed);

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
        // The refund is confirmed and spent by a confirmed sweep: fully done, even
        // with a later payment to the address listed ahead of it.
        let mut scan = refund_scan(&leaf_id, refund_txid, 42_000);
        if let ChainResult::AddressUtxos(txos) = &mut scan.result {
            txos.insert(
                0,
                AddressUtxo {
                    txid: Txid::from_byte_array([9u8; 32]),
                    vout: 0,
                    value: 330,
                    confirmed: true,
                    block_height: None,
                },
            );
        }
        let observed = vec![
            spent(deposit, root_txid),
            spent(leaf_parent_out, leaf_cpfp_txid),
            scan,
            spent(refund_outpoint, Txid::from_byte_array([7u8; 32])),
        ];
        let interp = interpret_chain(&prepared, &observed);

        assert!(
            matches!(
                interp.resolved.refunds.get(&leaf_id),
                Some(RefundState::Swept)
            ),
            "a refund spent by a confirmed sweep is swept"
        );
    }

    /// The leaf, its refund and a child of that refund paying the leaf's address
    /// again, with the address listing that holds both outputs.
    fn refund_with_child() -> (
        UnilateralExitPlan,
        TreeNodeId,
        Vec<Observation>,
        OutPoint,
        Transaction,
    ) {
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

        let refund_outpoint = OutPoint {
            txid: Txid::from_byte_array([5u8; 32]),
            vout: 0,
        };
        let child_tx = tx_spending(refund_outpoint, 6);
        let child_outpoint = OutPoint {
            txid: child_tx.compute_txid(),
            vout: 0,
        };
        let mut scan = refund_scan(&leaf_id, refund_outpoint.txid, 42_000);
        if let ChainResult::AddressUtxos(txos) = &mut scan.result {
            txos.push(AddressUtxo {
                txid: child_outpoint.txid,
                vout: 0,
                value: 41_000,
                confirmed: true,
                block_height: None,
            });
        }
        let observed = vec![
            spent(deposit, root_txid),
            spent(leaf_parent_out, leaf_cpfp_txid),
            scan,
            spent(refund_outpoint, child_outpoint.txid),
        ];
        (prepared, leaf_id, observed, child_outpoint, child_tx)
    }

    #[test]
    fn interpret_adopts_the_output_of_a_child_paying_the_address_again() {
        let (prepared, leaf_id, mut observed, child_outpoint, child_tx) = refund_with_child();
        observed.push(unspent(child_outpoint));
        observed.push(Observation {
            query: ChainQuery::Transaction(child_outpoint.txid),
            result: ChainResult::Transaction(child_tx),
        });
        let interp = interpret_chain(&prepared, &observed);

        match interp.resolved.refunds.get(&leaf_id) {
            Some(RefundState::Adopted(adopted)) => {
                assert_eq!(adopted.outpoint, child_outpoint);
                assert_eq!(adopted.value, 41_000);
            }
            other => panic!("expected the child's output adopted, got {other:?}"),
        }
    }

    #[test]
    fn interpret_marks_swept_once_the_child_output_is_spent() {
        let (prepared, leaf_id, mut observed, child_outpoint, _) = refund_with_child();
        observed.push(spent(child_outpoint, Txid::from_byte_array([7u8; 32])));
        let interp = interpret_chain(&prepared, &observed);

        assert!(matches!(
            interp.resolved.refunds.get(&leaf_id),
            Some(RefundState::Swept)
        ));
    }

    #[test]
    fn a_scan_names_the_sweep_of_a_swept_refund() {
        let (prepared, leaf_id, mut observed, child_outpoint, _) = refund_with_child();
        let sweep = Txid::from_byte_array([7u8; 32]);
        observed.push(Observation {
            query: ChainQuery::Outspend(child_outpoint),
            result: ChainResult::Spend(Some(SpendInfo {
                spender_txid: sweep,
                confirmed: true,
                block_height: Some(120),
            })),
        });

        let scan = scan_of(&prepared, &observed);

        assert_eq!(
            scan.sweeps,
            vec![RefundSweep {
                leaf_id,
                txid: sweep,
                block_height: Some(120),
            }]
        );
    }

    fn sweep_lookup(
        leaf_id: &TreeNodeId,
        observed: &[Observation],
    ) -> (RefundSweepLookup, Vec<ChainQuery>) {
        let addresses = HashMap::from([(leaf_id.clone(), leaf_addr())]);
        let mut scan = scan_refund_sweeps(&addresses, observed);
        (scan.lookups.remove(leaf_id).unwrap(), scan.pending)
    }

    fn spent_in_block(outpoint: OutPoint, spender: Txid, block_height: u32) -> Observation {
        Observation {
            query: ChainQuery::Outspend(outpoint),
            result: ChainResult::Spend(Some(SpendInfo {
                spender_txid: spender,
                confirmed: true,
                block_height: Some(block_height),
            })),
        }
    }

    #[test]
    fn a_refund_sweep_takes_the_address_and_one_outspend() {
        let leaf_id = TreeNodeId::from_str("leaf").unwrap();
        let refund = OutPoint {
            txid: Txid::from_byte_array([5u8; 32]),
            vout: 0,
        };
        let sweep = Txid::from_byte_array([7u8; 32]);
        let address = refund_scan(&leaf_id, refund.txid, 42_000);

        let (lookup, pending) = sweep_lookup(&leaf_id, &[]);
        assert_eq!(lookup, RefundSweepLookup::Pending);
        assert_eq!(pending, vec![address.query.clone()]);

        let (lookup, pending) = sweep_lookup(&leaf_id, std::slice::from_ref(&address));
        assert_eq!(lookup, RefundSweepLookup::Pending);
        assert_eq!(pending, vec![ChainQuery::Outspend(refund)]);

        let swept = [address.clone(), spent_in_block(refund, sweep, 120)];
        let (lookup, pending) = sweep_lookup(&leaf_id, &swept);
        assert!(pending.is_empty());
        assert_eq!(
            lookup,
            RefundSweepLookup::Swept(RefundSweep {
                leaf_id: leaf_id.clone(),
                txid: sweep,
                block_height: Some(120),
            })
        );
    }

    #[test]
    fn a_refund_without_a_sweep_in_a_block_is_not_swept() {
        let leaf_id = TreeNodeId::from_str("leaf").unwrap();
        let refund = OutPoint {
            txid: Txid::from_byte_array([5u8; 32]),
            vout: 0,
        };
        let sweep = Txid::from_byte_array([7u8; 32]);
        let address = refund_scan(&leaf_id, refund.txid, 42_000);

        // The refund itself is not fetched: its spend settles the lookup.
        for outspend in [unspent(refund), spent_unconfirmed(refund, sweep)] {
            let (lookup, pending) = sweep_lookup(&leaf_id, &[address.clone(), outspend]);
            assert!(pending.is_empty());
            assert_eq!(lookup, RefundSweepLookup::NotSwept);
        }
        let (lookup, pending) = sweep_lookup(&leaf_id, &[no_refund(&leaf_id)]);
        assert!(pending.is_empty());
        assert_eq!(lookup, RefundSweepLookup::NotSwept);
    }

    #[test]
    fn a_refund_sweep_is_followed_through_the_child_paying_the_refunds_fee() {
        let (_, leaf_id, observed, child_outpoint, _) = refund_with_child();
        let sweep = Txid::from_byte_array([7u8; 32]);
        // The address listing and the spend of the refund by its child.
        let followed = &observed[2..];

        let (lookup, pending) = sweep_lookup(&leaf_id, followed);
        assert_eq!(lookup, RefundSweepLookup::Pending);
        assert_eq!(pending, vec![ChainQuery::Outspend(child_outpoint)]);

        let swept = [followed, &[spent_in_block(child_outpoint, sweep, 120)]].concat();
        let (lookup, pending) = sweep_lookup(&leaf_id, &swept);
        assert!(pending.is_empty());
        assert_eq!(
            lookup,
            RefundSweepLookup::Swept(RefundSweep {
                leaf_id,
                txid: sweep,
                block_height: Some(120),
            })
        );
    }

    #[test]
    fn a_refund_query_without_a_result_leaves_the_sweep_lookup_pending() {
        let leaf_id = TreeNodeId::from_str("leaf").unwrap();
        let unavailable = Observation {
            query: ChainQuery::RefundAddress {
                leaf_id: leaf_id.clone(),
                address: leaf_addr(),
            },
            result: ChainResult::Unavailable,
        };

        let (lookup, pending) = sweep_lookup(&leaf_id, &[unavailable]);

        assert_eq!(lookup, RefundSweepLookup::Pending);
        assert!(pending.is_empty());
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
        let interp = interpret_chain(&prepared, &observed);

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
        let interp = interpret_chain(&prepared, &observed);

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
            // The chain cannot be read at all, the resume probe included, so the
            // walk starts at the deposit and its own fallback applies.
            Observation {
                query: ChainQuery::TxConfirmed(root_txid),
                result: ChainResult::Unavailable,
            },
            Observation {
                query: ChainQuery::Outspend(deposit),
                result: ChainResult::Unavailable,
            },
            spent(leaf_parent_out, leaf_cpfp_txid),
            no_refund(&leaf_id),
        ];
        let interp = interpret_chain(&prepared, &observed);

        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp { block_height: None })
        );
        assert_eq!(
            interp.resolved.nodes.get(&leaf_id),
            Some(&NodeState::ConfirmedCpfp { block_height: None })
        );
    }

    #[test]
    fn interpret_flags_driven_child_below_operator_confirmed_node() {
        // The root's chain lookup is unavailable, so its confirmation rests on the
        // operators' OnChain flag: scan_funding can't see the spend, so the leaf
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
            // The chain cannot be read at all, the resume probe included, so the
            // walk starts at the deposit and its own fallback applies.
            Observation {
                query: ChainQuery::TxConfirmed(root_txid),
                result: ChainResult::Unavailable,
            },
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
        let interp = interpret_chain(&prepared, &observed);

        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp { block_height: None }),
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
        // its CPFP change can't be resolved. scan_funding still protects any reused
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
        let interp = interpret_chain(&prepared, &observed);

        assert_eq!(
            interp.resolved.nodes.get(&id("root")),
            Some(&NodeState::ConfirmedCpfp { block_height: None }),
            "root is chain-confirmed but its change is unresolved"
        );
        assert!(
            !interp.unverified.contains(&leaf_id),
            "a chain-verified confirmation does not flag the driven child"
        );
    }
}
