use std::{collections::HashMap, fmt::Display, str::FromStr};

use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Transaction, TxOut, Txid,
    consensus::encode::{deserialize_hex, serialize_hex},
    hex::{DisplayHex, FromHex},
};
use spark_wallet::{
    ChainQuery, ChainResult, Observation, TreeNode, TreeNodeId, TreeNodeStatus,
    UnsignedWatchtowerExitRecovery, WatchtowerExitLookup, WatchtowerExitOutput,
    build_watchtower_exit_recovery, scan_watchtower_exits,
};
use tracing::warn;

use crate::{
    ChainTransaction, CooperativeRecoveryError, LeafRecovery, Storage, StoredWatchtowerExitOutput,
    UpdateLeafRecovery, WatchtowerExitRecovery,
    error::SdkError,
    utils::{replacement::Replaced, time::now_secs},
};

use super::{
    BreezSdk,
    chain_queries::{ChainQueries, batches, result_of, without_result},
    recover_funds::{store_checks, store_update},
};

/// A leaf whose funds are in an on-chain output, with the recoveries stored
/// for that output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WatchtowerExit {
    pub(super) leaf_id: String,
    pub(super) value_sats: u64,
    pub(super) output: WatchtowerExitOutput,
    recoveries: Vec<Transaction>,
    /// Whether a recovery of `output` is in a block.
    pub(super) recovered: bool,
    /// Whether the operators co-signed a recovery of `output` before. Only then
    /// can a transaction have `output` as an input.
    recovery_cosigned: bool,
}

/// The result of looking up watchtower exits, by leaf id.
#[derive(Default)]
pub(super) struct WatchtowerExits {
    pub(super) exits: HashMap<String, WatchtowerExit>,
    /// The lookup of each leaf that has no output to recover.
    pub(super) missing: HashMap<String, WatchtowerExitLookup>,
}

/// What the chain service says spent an output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum OutputSpend {
    /// The chain service returned no result.
    Unknown,
    Unspent,
    InMempool(Txid),
    InBlock {
        txid: Txid,
        /// Unset when the chain service did not report the block.
        block_height: Option<u32>,
    },
}

impl OutputSpend {
    /// The spender, once the chain service reported its block.
    pub(super) fn in_block(&self) -> Option<ChainTransaction> {
        match self {
            Self::InBlock {
                txid,
                block_height: Some(block_height),
            } => Some(ChainTransaction {
                txid: txid.to_string(),
                block_height: *block_height,
            }),
            _ => None,
        }
    }

    /// Whether this completes the check of the output: the chain service
    /// returned a result, and for a spend in a block also the block.
    pub(super) fn is_known(&self) -> bool {
        !matches!(
            self,
            Self::Unknown
                | Self::InBlock {
                    block_height: None,
                    ..
                }
        )
    }
}

/// What spent `outpoint`, read from `observed`.
pub(super) fn output_spend(observed: &[Observation], outpoint: OutPoint) -> OutputSpend {
    match result_of(observed, &ChainQuery::Outspend(outpoint)) {
        Some(ChainResult::Spend(None)) => OutputSpend::Unspent,
        Some(ChainResult::Spend(Some(spend))) if spend.confirmed => OutputSpend::InBlock {
            txid: spend.spender_txid,
            block_height: spend.block_height,
        },
        Some(ChainResult::Spend(Some(spend))) => OutputSpend::InMempool(spend.spender_txid),
        _ => OutputSpend::Unknown,
    }
}

impl BreezSdk {
    /// Checks with the chain service where the funds of each recovered leaf are,
    /// and whether a recovery of them is in a block. The operators report a
    /// recovered leaf the same either way. Stores what it finds, and returns
    /// the leaves whose check is complete.
    pub(super) async fn check_recovered_leaves(
        &self,
        leaves: &[TreeNode],
        stored: &HashMap<String, LeafRecovery>,
        queries: &mut ChainQueries,
    ) -> Vec<String> {
        // A leaf with a stored output needs no lookup, so no ancestors either.
        let (with_output, without_output): (Vec<TreeNode>, Vec<TreeNode>) = leaves
            .iter()
            .cloned()
            .partition(|leaf| stored_outpoint(stored, leaf).is_some());
        let mut checkable = with_output;
        let mut nodes = HashMap::new();
        if !without_output.is_empty() {
            let leaf_ids: Vec<TreeNodeId> =
                without_output.iter().map(|leaf| leaf.id.clone()).collect();
            match self
                .spark_wallet
                .fetch_nodes_with_ancestors(&leaf_ids)
                .await
            {
                Ok(ancestors) => {
                    nodes = ancestors;
                    checkable.extend(without_output);
                }
                Err(e) => warn!("Failed to fetch the ancestors of recovered leaves: {e}"),
            }
        }
        store_recovered_leaf_checks(&checkable, &nodes, stored, queries, self.storage.as_ref())
            .await
    }

    /// Looks up the output each of `leaves` is recovered from and whether a
    /// recovery of it is in a block, and stores what the chain service showed
    /// in a block.
    pub(super) async fn lookup_watchtower_exits(
        &self,
        leaves: &[TreeNode],
        stored: &HashMap<String, LeafRecovery>,
        queries: &mut ChainQueries,
    ) -> Result<WatchtowerExits, SdkError> {
        if leaves.is_empty() {
            return Ok(WatchtowerExits::default());
        }
        // The output of an on-chain leaf is in its own direct tx, so only the
        // others need their ancestors.
        let exited: Vec<TreeNodeId> = leaves
            .iter()
            .filter(|leaf| leaf.status != TreeNodeStatus::OnChain)
            .map(|leaf| leaf.id.clone())
            .collect();
        let (nodes, ancestors_known) = if exited.is_empty() {
            (HashMap::new(), true)
        } else {
            match self.spark_wallet.fetch_nodes_with_ancestors(&exited).await {
                Ok(nodes) => (nodes, true),
                Err(e) => {
                    warn!("Failed to fetch the ancestors of watchtower-exited leaves: {e}");
                    (HashMap::new(), false)
                }
            }
        };
        store_watchtower_exit_lookups(
            leaves,
            &nodes,
            ancestors_known,
            stored,
            queries,
            self.storage.as_ref(),
        )
        .await
    }

    /// The signed recovery of `exit` and its fee: one stored before, or else one
    /// the operators co-sign, unless they were found `unreachable`. `None` once
    /// a recovery of the output is in a block.
    pub(super) async fn cooperative_recovery(
        &self,
        exit: &WatchtowerExit,
        destination: &Address,
        fee_rate_sat_per_vbyte: u64,
        unreachable: Option<&str>,
        queries: &mut ChainQueries,
    ) -> Result<Option<(Transaction, u64)>, CooperativeRecoveryError> {
        if exit.recovered {
            return Ok(None);
        }
        let unsigned = build_recovery(&exit.output, destination, fee_rate_sat_per_vbyte)
            .ok_or_else(|| generic("The output is too small to pay this fee"))?;
        let spend = exit_output_spend(exit, queries).await;
        if let OutputSpend::InBlock { .. } = spend {
            if let Some(spend) = spend.in_block() {
                self.store_leaf_recovery(UpdateLeafRecovery {
                    leaf_id: exit.leaf_id.clone(),
                    chain_checked_at: Some(now_secs()),
                    watchtower_exit_spend: Some(spend),
                    ..Default::default()
                })
                .await;
            }
            return Ok(None);
        }
        let in_mempool = match &spend {
            OutputSpend::InMempool(txid) if exit.stored_recovery(*txid).is_none() => {
                let transaction = ChainQuery::Transaction(*txid);
                queries
                    .resolve(|observed| ((), without_result(vec![transaction.clone()], observed)))
                    .await;
                match result_of(queries.observed(), &transaction) {
                    Some(ChainResult::Transaction(tx)) => Some(tx.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((txid, replaced)) = recovery_to_outbid(exit, &spend, in_mempool.as_ref())? {
            outbid(&unsigned, txid, &replaced)?;
        }
        if let Some(signed) = exit.signed_recovery(&unsigned.tx) {
            return Ok(Some((signed, unsigned.fee_sat)));
        }
        if let Some(message) = unreachable {
            return Err(CooperativeRecoveryError::OperatorsUnavailable {
                message: message.to_string(),
            });
        }

        let signed = self
            .spark_wallet
            .cosign_watchtower_exit_recovery(&exit.output, unsigned.tx)
            .await
            .map_err(|e| {
                if e.is_operator_unavailable() {
                    CooperativeRecoveryError::OperatorsUnavailable {
                        message: e.to_string(),
                    }
                } else {
                    generic(e)
                }
            })?;
        // The transaction is signed either way: losing it here only costs asking
        // the operators again next time.
        self.store_leaf_recovery(UpdateLeafRecovery {
            leaf_id: exit.leaf_id.clone(),
            watchtower_exit_recovery: Some(WatchtowerExitRecovery {
                txid: signed.compute_txid().to_string(),
                transaction_hex: serialize_hex(&signed),
                output_amount_sats: paid_out(&signed),
            }),
            ..Default::default()
        })
        .await;
        Ok(Some((signed, unsigned.fee_sat)))
    }
}

impl WatchtowerExit {
    /// Takes from `stored` the recoveries with `output` as input. The exit is
    /// recovered when `spend` or a stored spend of `output` is in a block.
    pub(super) fn new(
        leaf_id: String,
        value_sats: u64,
        output: WatchtowerExitOutput,
        stored: Option<&LeafRecovery>,
        spend: &OutputSpend,
        recovery_cosigned: bool,
    ) -> Self {
        let recoveries = stored
            .into_iter()
            .flat_map(|stored| &stored.watchtower_exit_recoveries)
            .filter_map(|recovery| deserialize_hex::<Transaction>(&recovery.transaction_hex).ok())
            .filter(|tx| {
                tx.input
                    .first()
                    .is_some_and(|input| input.previous_output == output.outpoint)
            })
            .collect();
        // A stored spend belongs to the stored output.
        let stored_spend = stored.is_some_and(|stored| {
            stored.watchtower_exit_spend.is_some()
                && stored
                    .watchtower_exit_output
                    .as_ref()
                    .is_none_or(|stored| same_outpoint(stored, &output))
        });
        Self {
            leaf_id,
            value_sats,
            output,
            recoveries,
            recovered: stored_spend || matches!(spend, OutputSpend::InBlock { .. }),
            recovery_cosigned,
        }
    }

    /// The exit of a leaf from its stored output alone. `None` when none is
    /// stored. Without the leaf the SDK does not know whether the operators
    /// co-signed a recovery, so it takes that they did.
    pub(super) fn from_stored(
        value_sats: u64,
        stored: &LeafRecovery,
    ) -> Result<Option<Self>, SdkError> {
        let Some(output) = &stored.watchtower_exit_output else {
            return Ok(None);
        };
        let output = exited_output(&stored.leaf_id, output)?;
        Ok(Some(Self::new(
            stored.leaf_id.clone(),
            value_sats,
            output,
            Some(stored),
            &OutputSpend::Unknown,
            true,
        )))
    }

    /// The leaf's value less what `output` holds.
    pub(super) fn exit_fee_sats(&self) -> u64 {
        self.value_sats
            .saturating_sub(self.output.tx_out.value.to_sat())
    }

    /// The stored recovery with the same txid as `unsigned`: a txid does not
    /// cover the signature.
    fn signed_recovery(&self, unsigned: &Transaction) -> Option<Transaction> {
        self.stored_recovery(unsigned.compute_txid()).cloned()
    }

    fn stored_recovery(&self, txid: Txid) -> Option<&Transaction> {
        self.recoveries
            .iter()
            .find(|stored| stored.compute_txid() == txid)
    }

    /// The stored recovery with the smallest output, so with the highest fee.
    fn highest_fee_recovery(&self) -> Option<&Transaction> {
        self.recoveries.iter().min_by_key(|tx| paid_out(tx))
    }
}

/// What spent the output of `exit`. No transaction has it as an input before
/// the operators co-sign a recovery, so the SDK sends no request until then.
async fn exit_output_spend(exit: &WatchtowerExit, queries: &mut ChainQueries) -> OutputSpend {
    if !exit.recovery_cosigned {
        return OutputSpend::Unspent;
    }
    let outspend = ChainQuery::Outspend(exit.output.outpoint);
    queries
        .resolve(|observed| ((), without_result(vec![outspend.clone()], observed)))
        .await;
    output_spend(queries.observed(), exit.output.outpoint)
}

/// Whether the operators report `leaf` as recovered: they co-signed a recovery
/// of its output. `nodes` holds the leaf as the operators last returned it.
fn is_recovered(leaf: &TreeNode, nodes: &HashMap<TreeNodeId, TreeNode>) -> bool {
    nodes.get(&leaf.id).unwrap_or(leaf).status == TreeNodeStatus::WatchtowerExitRecovered
}

/// The lookup of each leaf's output over `observed`, and the queries the caller
/// has yet to execute: those of the scan, and what spent the output it found
/// for a recovered leaf. Such an output is an input of recoveries only, and the
/// operators report a leaf as recovered from the moment they co-sign one.
fn scan_outputs(
    leaves: &[TreeNode],
    nodes: &HashMap<TreeNodeId, TreeNode>,
    observed: &[Observation],
) -> (HashMap<TreeNodeId, WatchtowerExitLookup>, Vec<ChainQuery>) {
    let scan = scan_watchtower_exits(leaves, nodes, observed);
    let outspends = leaves
        .iter()
        .filter(|leaf| is_recovered(leaf, nodes))
        .filter_map(|leaf| match scan.lookups.get(&leaf.id) {
            Some(WatchtowerExitLookup::Found { output, .. }) => {
                Some(ChainQuery::Outspend(output.outpoint))
            }
            _ => None,
        })
        .collect();
    let mut pending = scan.pending;
    pending.extend(without_result(outspends, observed));
    (scan.lookups, pending)
}

/// The lookup of `leaf`. Without the leaf's ancestors `scan_watchtower_exits`
/// cannot look for its output, unless the leaf is on-chain: its output is in
/// its own direct tx.
fn lookup_with(
    lookup: Option<WatchtowerExitLookup>,
    leaf: &TreeNode,
    ancestors_known: bool,
) -> WatchtowerExitLookup {
    match lookup {
        Some(WatchtowerExitLookup::NotFound)
            if !ancestors_known && leaf.status != TreeNodeStatus::OnChain =>
        {
            WatchtowerExitLookup::Pending
        }
        Some(lookup) => lookup,
        None => WatchtowerExitLookup::Pending,
    }
}

/// The result of a lookup for one leaf.
struct LookedUpExit {
    /// The lookup instead, when the leaf has no output to recover.
    exit: Result<WatchtowerExit, WatchtowerExitLookup>,
    /// What the chain service showed in a block that `stored` does not hold.
    update: Option<UpdateLeafRecovery>,
}

/// `recovered` is whether the operators report `leaf` as recovered.
fn looked_up_exit(
    leaf: &TreeNode,
    recovered: bool,
    lookup: WatchtowerExitLookup,
    stored: Option<&LeafRecovery>,
    observed: &[Observation],
    now: u64,
) -> Result<LookedUpExit, SdkError> {
    let leaf_id = leaf.id.to_string();
    let stored_output = stored.and_then(|stored| stored.watchtower_exit_output.as_ref());
    let (output, spend, update) = match (lookup, stored_output) {
        (
            WatchtowerExitLookup::Found {
                output,
                block_height,
            },
            _,
        ) => {
            let spend = output_spend(observed, output.outpoint);
            let in_block = block_height.map(|height| stored_output_of(&output, height));
            // The SDK stores a spend only when storage holds its output.
            let spent_output_stored = in_block.is_some()
                || stored_output.is_none_or(|stored| same_outpoint(stored, &output));
            // Only a recovered leaf has a spend to check.
            let complete = !recovered || spend.is_known();
            // The SDK stores the output as soon as it finds it, and the check
            // time once the check is complete.
            let update = UpdateLeafRecovery {
                leaf_id: leaf_id.clone(),
                chain_checked_at: complete.then_some(now),
                watchtower_exit_output: in_block,
                watchtower_exit_spend: spend.in_block().filter(|_| spent_output_stored),
                ..Default::default()
            };
            let stored_spend = stored.and_then(|stored| stored.watchtower_exit_spend.as_ref());
            let new = update
                .watchtower_exit_output
                .as_ref()
                .is_some_and(|output| stored_output != Some(output))
                || update
                    .watchtower_exit_spend
                    .as_ref()
                    .is_some_and(|spend| stored_spend != Some(spend))
                || (complete && unchecked_output(stored, &output));
            (output, spend, new.then_some(update))
        }
        // An output the chain service reported earlier takes precedence over
        // the one of the direct tx the node holds.
        (_, Some(stored_output)) => (
            exited_output(&leaf_id, stored_output)?,
            OutputSpend::Unknown,
            None,
        ),
        (WatchtowerExitLookup::Unconfirmed(output), None) => (output, OutputSpend::Unknown, None),
        (lookup, None) => {
            return Ok(LookedUpExit {
                exit: Err(lookup),
                update: None,
            });
        }
    };
    Ok(LookedUpExit {
        exit: Ok(WatchtowerExit::new(
            leaf_id, leaf.value, output, stored, &spend, recovered,
        )),
        update,
    })
}

/// Looks up the watchtower exit of each of `leaves` and stores what the chain
/// service showed in a block. `nodes` holds the ancestors of the leaves when
/// `ancestors_known`.
async fn store_watchtower_exit_lookups(
    leaves: &[TreeNode],
    nodes: &HashMap<TreeNodeId, TreeNode>,
    ancestors_known: bool,
    stored: &HashMap<String, LeafRecovery>,
    queries: &mut ChainQueries,
    storage: &dyn Storage,
) -> Result<WatchtowerExits, SdkError> {
    queries
        .resolve(|observed| ((), scan_outputs(leaves, nodes, observed).1))
        .await;
    let (mut lookups, _) = scan_outputs(leaves, nodes, queries.observed());

    let mut found = WatchtowerExits::default();
    let now = now_secs();
    for leaf in leaves {
        let leaf_id = leaf.id.to_string();
        let lookup = lookup_with(lookups.remove(&leaf.id), leaf, ancestors_known);
        if lookup == WatchtowerExitLookup::Unrecoverable {
            warn!(
                "Watchtower-exited leaf {leaf_id} has no cooperative recovery: the direct \
                 tx in a block above it has no output for the leaf's key"
            );
        }
        let looked_up = looked_up_exit(
            leaf,
            is_recovered(leaf, nodes),
            lookup,
            stored.get(&leaf_id),
            queries.observed(),
            now,
        )?;
        if let Some(update) = looked_up.update {
            store_update(storage, update).await;
        }
        match looked_up.exit {
            Ok(exit) => {
                found.exits.insert(leaf_id, exit);
            }
            Err(lookup) => {
                found.missing.insert(leaf_id, lookup);
            }
        }
    }
    Ok(found)
}

/// Checks `leaves` batch by batch. It stores what a batch established before
/// it starts the next one, and starts none after a failed request. Returns the
/// leaves whose check is complete.
pub(super) async fn store_recovered_leaf_checks(
    leaves: &[TreeNode],
    nodes: &HashMap<TreeNodeId, TreeNode>,
    stored: &HashMap<String, LeafRecovery>,
    queries: &mut ChainQueries,
    storage: &dyn Storage,
) -> Vec<String> {
    let mut complete = Vec::new();
    for batch in batches(leaves) {
        if queries.failed() {
            break;
        }
        queries
            .resolve(|observed| ((), recovered_leaf_queries(batch, nodes, stored, observed)))
            .await;
        let checks = recovered_leaf_checks(batch, nodes, stored, &queries.fetched(), now_secs());
        complete.extend(store_checks(storage, checks).await);
    }
    complete
}

/// The outpoint of the output stored for `leaf`.
fn stored_outpoint(stored: &HashMap<String, LeafRecovery>, leaf: &TreeNode) -> Option<OutPoint> {
    let leaf_id = leaf.id.to_string();
    let output = stored.get(&leaf_id)?.watchtower_exit_output.as_ref()?;
    Some(exited_output(&leaf_id, output).ok()?.outpoint)
}

/// The queries the check of `leaves` has yet to execute. For a leaf with a
/// stored output that is only whether the output is spent.
fn recovered_leaf_queries(
    leaves: &[TreeNode],
    nodes: &HashMap<TreeNodeId, TreeNode>,
    stored: &HashMap<String, LeafRecovery>,
    observed: &[Observation],
) -> Vec<ChainQuery> {
    let (with_output, without_output): (Vec<TreeNode>, Vec<TreeNode>) = leaves
        .iter()
        .cloned()
        .partition(|leaf| stored_outpoint(stored, leaf).is_some());
    let outspends = with_output
        .iter()
        .filter_map(|leaf| stored_outpoint(stored, leaf))
        .map(ChainQuery::Outspend)
        .collect();
    let mut pending = scan_outputs(&without_output, nodes, observed).1;
    pending.extend(without_result(outspends, observed));
    pending
}

/// What to store for each recovered leaf, from the queries with a result. An
/// output found in a block is stored as soon as it is found, also while the
/// query for its spend has no result, so that the next sync starts from it. The
/// check time is stored once the check is complete: the SDK knows whether the
/// output is spent, or the results show the leaf's funds in no block.
fn recovered_leaf_checks(
    leaves: &[TreeNode],
    nodes: &HashMap<TreeNodeId, TreeNode>,
    stored: &HashMap<String, LeafRecovery>,
    fetched: &[Observation],
    now: u64,
) -> Vec<UpdateLeafRecovery> {
    let without_output: Vec<TreeNode> = leaves
        .iter()
        .filter(|leaf| stored_outpoint(stored, leaf).is_none())
        .cloned()
        .collect();
    let (lookups, _) = scan_outputs(&without_output, nodes, fetched);
    let mut checks = Vec::new();
    for leaf in leaves {
        let checked = UpdateLeafRecovery {
            leaf_id: leaf.id.to_string(),
            chain_checked_at: Some(now),
            ..Default::default()
        };
        if let Some(outpoint) = stored_outpoint(stored, leaf) {
            let spend = output_spend(fetched, outpoint);
            if spend.is_known() {
                checks.push(UpdateLeafRecovery {
                    watchtower_exit_spend: spend.in_block(),
                    ..checked
                });
            }
            continue;
        }
        match lookups.get(&leaf.id) {
            None | Some(WatchtowerExitLookup::Pending) => {}
            Some(WatchtowerExitLookup::Found {
                output,
                block_height,
            }) => {
                // The SDK stores a transaction only with the height of its block.
                let Some(block_height) = block_height else {
                    continue;
                };
                let found = Some(stored_output_of(output, *block_height));
                let spend = output_spend(fetched, output.outpoint);
                checks.push(if spend.is_known() {
                    UpdateLeafRecovery {
                        watchtower_exit_output: found,
                        watchtower_exit_spend: spend.in_block(),
                        ..checked
                    }
                } else {
                    UpdateLeafRecovery {
                        leaf_id: leaf.id.to_string(),
                        watchtower_exit_output: found,
                        ..Default::default()
                    }
                });
            }
            Some(_) => checks.push(checked),
        }
    }
    checks
}

/// Whether storage holds `output` for the leaf without a check time: the SDK
/// found the output and has yet to learn whether it is spent.
fn unchecked_output(stored: Option<&LeafRecovery>, output: &WatchtowerExitOutput) -> bool {
    stored.is_some_and(|stored| {
        stored.chain_checked_at.is_none()
            && stored
                .watchtower_exit_output
                .as_ref()
                .is_some_and(|stored| same_outpoint(stored, output))
    })
}

fn same_outpoint(stored: &StoredWatchtowerExitOutput, output: &WatchtowerExitOutput) -> bool {
    stored.vout == output.outpoint.vout && stored.txid == output.outpoint.txid.to_string()
}

fn stored_output_of(
    output: &WatchtowerExitOutput,
    block_height: u32,
) -> StoredWatchtowerExitOutput {
    StoredWatchtowerExitOutput {
        txid: output.outpoint.txid.to_string(),
        vout: output.outpoint.vout,
        amount_sats: output.tx_out.value.to_sat(),
        script_pubkey: output.tx_out.script_pubkey.as_bytes().to_lower_hex_string(),
        block_height,
    }
}

fn exited_output(
    leaf_id: &str,
    stored: &StoredWatchtowerExitOutput,
) -> Result<WatchtowerExitOutput, SdkError> {
    let txid = Txid::from_str(&stored.txid)
        .map_err(|e| SdkError::Generic(format!("invalid stored txid: {e}")))?;
    let script_pubkey = Vec::<u8>::from_hex(&stored.script_pubkey)
        .map_err(|e| SdkError::Generic(format!("invalid stored script: {e}")))?;
    Ok(WatchtowerExitOutput {
        leaf_id: leaf_id
            .parse()
            .map_err(|e| SdkError::Generic(format!("invalid stored leaf id: {e}")))?,
        outpoint: OutPoint {
            txid,
            vout: stored.vout,
        },
        tx_out: TxOut {
            value: Amount::from_sat(stored.amount_sats),
            script_pubkey: ScriptBuf::from_bytes(script_pubkey),
        },
    })
}

fn paid_out(tx: &Transaction) -> u64 {
    tx.output
        .iter()
        .map(|output| output.value.to_sat())
        .fold(0, u64::saturating_add)
}

/// The recovery of `exit` that is on the network and in no block, with what
/// replacing it takes. `in_mempool` is the spender the chain service returned,
/// when it is not a stored recovery. Without a result from the chain service,
/// the SDK takes the stored recovery with the highest fee to be on the network.
fn recovery_to_outbid(
    exit: &WatchtowerExit,
    spend: &OutputSpend,
    in_mempool: Option<&Transaction>,
) -> Result<Option<(Txid, Replaced)>, CooperativeRecoveryError> {
    let amount_sats = exit.output.tx_out.value.to_sat();
    match spend {
        OutputSpend::Unspent | OutputSpend::InBlock { .. } => Ok(None),
        OutputSpend::Unknown => Ok(exit.highest_fee_recovery().and_then(|tx| {
            Replaced::spending(tx, amount_sats).map(|replaced| (tx.compute_txid(), replaced))
        })),
        OutputSpend::InMempool(txid) => {
            let tx = exit
                .stored_recovery(*txid)
                .or(in_mempool.filter(|tx| tx.compute_txid() == *txid))
                .ok_or_else(|| {
                    generic(format!(
                        "The chain service did not return recovery {txid} from its mempool"
                    ))
                })?;
            Replaced::spending(tx, amount_sats)
                .map(|replaced| Some((*txid, replaced)))
                .ok_or_else(|| {
                    generic(format!(
                        "Recovery {txid} pays out more than the output holds"
                    ))
                })
        }
    }
}

fn outbid(
    unsigned: &UnsignedWatchtowerExitRecovery,
    txid: Txid,
    replaced: &Replaced,
) -> Result<(), CooperativeRecoveryError> {
    if unsigned.tx.compute_txid() == txid {
        return Ok(());
    }
    let required_fee_sats = replaced.min_replacement_fee_sats(unsigned.vsize);
    if unsigned.fee_sat < required_fee_sats {
        return Err(CooperativeRecoveryError::ReplacementFeeTooLow {
            required_fee_sats,
            required_fee_rate_sat_per_vbyte: required_fee_sats.div_ceil(unsigned.vsize),
        });
    }
    Ok(())
}

pub(super) fn build_recovery(
    output: &WatchtowerExitOutput,
    destination: &Address,
    fee_rate_sat_per_vbyte: u64,
) -> Option<UnsignedWatchtowerExitRecovery> {
    build_watchtower_exit_recovery(
        output,
        destination,
        spark_wallet::Fee::Rate {
            sat_per_vbyte: fee_rate_sat_per_vbyte,
        },
    )
}

fn generic(error: impl Display) -> CooperativeRecoveryError {
    CooperativeRecoveryError::Generic {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use spark_wallet::{SpendInfo, tree_store_tests::create_test_node_with_parent};

    use crate::chain::stub::tx_paying;

    use super::*;

    const LEAF_ID: &str = "00000000-0000-0000-0000-00000000000a";

    fn output_at(txid: &str, value_sats: u64) -> WatchtowerExitOutput {
        WatchtowerExitOutput {
            leaf_id: LEAF_ID.parse().unwrap(),
            outpoint: OutPoint {
                txid: Txid::from_str(&txid.repeat(32)).unwrap(),
                vout: 0,
            },
            tx_out: TxOut {
                value: Amount::from_sat(value_sats),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51, 0x20]),
            },
        }
    }

    fn output() -> WatchtowerExitOutput {
        output_at("00", 10_000)
    }

    fn paying(value: u64) -> Transaction {
        tx_paying(output().outpoint, value)
    }

    fn stored_recovery(tx: &Transaction) -> WatchtowerExitRecovery {
        WatchtowerExitRecovery {
            txid: tx.compute_txid().to_string(),
            transaction_hex: serialize_hex(tx),
            output_amount_sats: paid_out(tx),
        }
    }

    fn stored_leaf(recoveries: &[Transaction]) -> LeafRecovery {
        LeafRecovery {
            leaf_id: LEAF_ID.to_string(),
            chain_checked_at: None,
            watchtower_exit_output: Some(stored_output_of(&output(), 100)),
            watchtower_exit_recoveries: recoveries.iter().map(stored_recovery).collect(),
            watchtower_exit_spend: None,
            unilateral_exit_sweep: None,
        }
    }

    fn exit_of(output: WatchtowerExitOutput, stored: Option<&LeafRecovery>) -> WatchtowerExit {
        WatchtowerExit::new(
            LEAF_ID.to_string(),
            10_955,
            output,
            stored,
            &OutputSpend::Unspent,
            true,
        )
    }

    fn exit_with(recoveries: &[Transaction]) -> WatchtowerExit {
        exit_of(output(), Some(&stored_leaf(recoveries)))
    }

    fn spender() -> Txid {
        Txid::from_str(&"07".repeat(32)).unwrap()
    }

    fn in_block(block_height: Option<u32>) -> OutputSpend {
        OutputSpend::InBlock {
            txid: spender(),
            block_height,
        }
    }

    fn spent(
        output: &WatchtowerExitOutput,
        confirmed: bool,
        block_height: Option<u32>,
    ) -> Observation {
        Observation {
            query: ChainQuery::Outspend(output.outpoint),
            result: ChainResult::Spend(Some(SpendInfo {
                spender_txid: spender(),
                confirmed,
                block_height,
            })),
        }
    }

    fn unspent(output: &WatchtowerExitOutput) -> Observation {
        Observation {
            query: ChainQuery::Outspend(output.outpoint),
            result: ChainResult::Spend(None),
        }
    }

    #[test]
    fn a_stored_output_reads_back_as_it_was_found() {
        let stored = stored_output_of(&output(), 100);

        assert_eq!(exited_output(LEAF_ID, &stored).unwrap(), output());
        assert_eq!(exit_with(&[]).exit_fee_sats(), 955);
    }

    #[test]
    fn a_stored_recovery_is_the_same_transaction_signed() {
        let unsigned = paying(9_500);
        let mut signed = unsigned.clone();
        signed.input[0].witness = bitcoin::Witness::from_slice(&[[1u8; 64]]);
        let exit = exit_with(&[paying(9_000), signed.clone()]);

        assert_eq!(exit.signed_recovery(&unsigned), Some(signed));
        assert_eq!(exit.signed_recovery(&paying(9_400)), None);
    }

    #[test]
    fn the_recovery_paying_out_the_least_pays_the_highest_fee() {
        let exit = exit_with(&[paying(9_800), paying(9_000), paying(9_500)]);

        assert_eq!(exit.highest_fee_recovery(), Some(&paying(9_000)));
        assert!(exit_with(&[]).highest_fee_recovery().is_none());
    }

    #[test]
    fn a_recovery_or_spend_of_another_output_is_not_this_exits() {
        let other = output_at("02", 9_000);
        let mut stored = stored_leaf(&[paying(9_500), tx_paying(other.outpoint, 8_000)]);
        stored.watchtower_exit_spend = Some(ChainTransaction {
            txid: "spender".to_string(),
            block_height: 101,
        });

        let same = exit_of(output(), Some(&stored));
        assert_eq!(same.recoveries, vec![paying(9_500)]);
        assert!(same.recovered);

        let replaced = exit_of(other.clone(), Some(&stored));
        assert_eq!(replaced.recoveries, vec![tx_paying(other.outpoint, 8_000)]);
        assert!(!replaced.recovered);

        // A spend stored without an output is the spend of the recovery's input.
        stored.watchtower_exit_output = None;
        assert!(exit_of(output(), Some(&stored)).recovered);
        assert!(!exit_of(output(), None).recovered);
    }

    #[test]
    fn a_spend_the_chain_service_shows_in_a_block_recovered_the_exit() {
        let recovered = |spend: &OutputSpend| {
            WatchtowerExit::new(LEAF_ID.to_string(), 10_955, output(), None, spend, true).recovered
        };

        assert!(recovered(&in_block(Some(101))));
        assert!(recovered(&in_block(None)));
        for spend in [
            OutputSpend::Unspent,
            OutputSpend::InMempool(spender()),
            OutputSpend::Unknown,
        ] {
            assert!(!recovered(&spend));
        }
    }

    #[test]
    fn the_spend_of_an_output_is_read_from_its_outspend_result() {
        let output = output();
        let spend = |observed: &[Observation]| output_spend(observed, output.outpoint);

        assert_eq!(spend(&[]), OutputSpend::Unknown);
        assert_eq!(
            spend(&[Observation {
                query: ChainQuery::Outspend(output.outpoint),
                result: ChainResult::Unavailable,
            }]),
            OutputSpend::Unknown
        );
        assert_eq!(spend(&[unspent(&output)]), OutputSpend::Unspent);
        assert_eq!(
            spend(&[spent(&output, false, None)]),
            OutputSpend::InMempool(spender())
        );
        assert_eq!(
            spend(&[spent(&output, true, Some(101))]),
            in_block(Some(101))
        );
        assert_eq!(
            in_block(Some(101)).in_block(),
            Some(ChainTransaction {
                txid: spender().to_string(),
                block_height: 101,
            })
        );
        // The SDK stores a transaction only with the height of its block.
        assert_eq!(in_block(None).in_block(), None);
    }

    #[test]
    fn an_unspent_output_needs_no_outbidding() {
        let found = recovery_to_outbid(&exit_with(&[paying(9_700)]), &OutputSpend::Unspent, None);

        assert_eq!(found, Ok(None));
    }

    #[test]
    fn a_stored_recovery_in_the_mempool_is_outbid_on_what_it_pays() {
        let ours = paying(9_700);
        let exit = exit_with(std::slice::from_ref(&ours));

        let found = recovery_to_outbid(&exit, &OutputSpend::InMempool(ours.compute_txid()), None);

        assert_eq!(
            found,
            Ok(Some((
                ours.compute_txid(),
                Replaced {
                    fee_sats: 300,
                    vsize: ours.vsize() as u64,
                }
            )))
        );
    }

    #[test]
    fn a_recovery_from_elsewhere_is_read_from_the_chain() {
        let theirs = paying(9_000);
        let spend = OutputSpend::InMempool(theirs.compute_txid());

        assert_eq!(
            recovery_to_outbid(&exit_with(&[]), &spend, Some(&theirs)),
            Ok(Some((
                theirs.compute_txid(),
                Replaced {
                    fee_sats: 1_000,
                    vsize: theirs.vsize() as u64,
                }
            )))
        );
        // The chain service did not return it, or returned another transaction.
        for returned in [None, Some(&paying(9_100))] {
            assert!(matches!(
                recovery_to_outbid(&exit_with(&[]), &spend, returned),
                Err(CooperativeRecoveryError::Generic { .. })
            ));
        }
    }

    #[test]
    fn an_unknown_spend_assumes_the_highest_fee_stored_recovery() {
        let exit = exit_with(&[paying(9_800), paying(9_400)]);

        assert_eq!(
            recovery_to_outbid(&exit, &OutputSpend::Unknown, None),
            Ok(Some((
                paying(9_400).compute_txid(),
                Replaced {
                    fee_sats: 600,
                    vsize: paying(9_400).vsize() as u64,
                }
            )))
        );
        assert_eq!(
            recovery_to_outbid(&exit_with(&[]), &OutputSpend::Unknown, None),
            Ok(None)
        );
    }

    fn unsigned_with_fee(fee_sat: u64) -> UnsignedWatchtowerExitRecovery {
        let tx = paying(10_000u64.saturating_sub(fee_sat));
        UnsignedWatchtowerExitRecovery {
            fee_sat,
            vsize: tx.vsize() as u64,
            tx,
        }
    }

    #[test]
    fn the_recovery_on_the_network_goes_out_again() {
        let ours = unsigned_with_fee(300);
        let replaced = Replaced::spending(&ours.tx, 10_000).unwrap();

        assert_eq!(outbid(&ours, ours.tx.compute_txid(), &replaced), Ok(()));
    }

    #[test]
    fn another_recovery_has_to_outbid_the_one_on_the_network() {
        let theirs = paying(9_700);
        let replaced = Replaced::spending(&theirs, 10_000).unwrap();
        let required_fee_sats = replaced.min_replacement_fee_sats(theirs.vsize() as u64);

        let too_low = unsigned_with_fee(required_fee_sats.saturating_sub(1));
        assert_eq!(
            outbid(&too_low, theirs.compute_txid(), &replaced),
            Err(CooperativeRecoveryError::ReplacementFeeTooLow {
                required_fee_sats,
                required_fee_rate_sat_per_vbyte: required_fee_sats.div_ceil(too_low.vsize),
            })
        );
        let enough = unsigned_with_fee(required_fee_sats);
        assert_eq!(outbid(&enough, theirs.compute_txid(), &replaced), Ok(()));
    }

    /// A leaf whose direct tx is in a block at another fee than the one it
    /// holds, with the two queries that show it.
    struct ExitedLeaf {
        leaf: TreeNode,
        found: WatchtowerExitOutput,
        parent_spent: Observation,
        direct_tx: Observation,
    }

    fn exited_leaf(status: TreeNodeStatus) -> ExitedLeaf {
        exited_leaf_at(LEAF_ID, "03", status)
    }

    /// The leaf `leaf_id`, with the output of `parent_txid` as its input.
    fn exited_leaf_at(leaf_id: &str, parent_txid: &str, status: TreeNodeStatus) -> ExitedLeaf {
        let mut leaf = create_test_node_with_parent(leaf_id, None, status);
        let leaf_script = ScriptBuf::new_p2tr(
            &bitcoin::secp256k1::Secp256k1::verification_only(),
            leaf.verifying_public_key.x_only_public_key().0,
            None,
        );
        let parent_output = OutPoint {
            txid: Txid::from_str(&parent_txid.repeat(32)).unwrap(),
            vout: 0,
        };
        let mut held = tx_paying(parent_output, 9_800);
        held.output[0].script_pubkey = leaf_script.clone();
        let mut confirmed = tx_paying(parent_output, 9_500);
        confirmed.output[0].script_pubkey = leaf_script;
        let found = WatchtowerExitOutput {
            leaf_id: leaf.id.clone(),
            outpoint: OutPoint {
                txid: confirmed.compute_txid(),
                vout: 0,
            },
            tx_out: confirmed.output[0].clone(),
        };
        leaf.direct_tx = Some(held);
        ExitedLeaf {
            leaf,
            found,
            parent_spent: Observation {
                query: ChainQuery::Outspend(parent_output),
                result: ChainResult::Spend(Some(SpendInfo {
                    spender_txid: confirmed.compute_txid(),
                    confirmed: true,
                    block_height: Some(100),
                })),
            },
            direct_tx: Observation {
                query: ChainQuery::Transaction(confirmed.compute_txid()),
                result: ChainResult::Transaction(confirmed),
            },
        }
    }

    #[test]
    fn a_found_output_of_a_recovered_leaf_is_checked_for_a_spend() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
        let leaves = [exited.leaf.clone()];
        let mut observed = vec![exited.parent_spent.clone(), exited.direct_tx.clone()];

        let (lookups, pending) = scan_outputs(&leaves, &HashMap::new(), &observed);
        assert_eq!(
            lookups[&exited.leaf.id],
            WatchtowerExitLookup::Found {
                output: exited.found.clone(),
                block_height: Some(100),
            }
        );
        assert_eq!(pending, vec![ChainQuery::Outspend(exited.found.outpoint)]);

        observed.push(unspent(&exited.found));
        assert!(
            scan_outputs(&leaves, &HashMap::new(), &observed)
                .1
                .is_empty()
        );
    }

    #[test]
    fn a_found_output_of_a_leaf_that_is_not_recovered_is_not_checked_for_a_spend() {
        let exited = exited_leaf(TreeNodeStatus::OnChain);
        let leaves = [exited.leaf.clone()];
        let observed = vec![exited.parent_spent.clone(), exited.direct_tx.clone()];

        let (lookups, pending) = scan_outputs(&leaves, &HashMap::new(), &observed);
        assert_eq!(lookups[&exited.leaf.id], found(&exited, Some(100)));
        assert!(pending.is_empty());

        // The operators report the leaf as recovered since the wallet last
        // listed it.
        let mut recovered = exited.leaf.clone();
        recovered.status = TreeNodeStatus::WatchtowerExitRecovered;
        let nodes = HashMap::from([(recovered.id.clone(), recovered)]);
        assert_eq!(
            scan_outputs(&leaves, &nodes, &observed).1,
            vec![ChainQuery::Outspend(exited.found.outpoint)]
        );
    }

    fn found(exited: &ExitedLeaf, block_height: Option<u32>) -> WatchtowerExitLookup {
        WatchtowerExitLookup::Found {
            output: exited.found.clone(),
            block_height,
        }
    }

    fn stored_with(output: Option<StoredWatchtowerExitOutput>) -> LeafRecovery {
        LeafRecovery {
            watchtower_exit_output: output,
            ..stored_leaf(&[])
        }
    }

    #[test]
    fn an_output_found_in_a_block_is_stored_with_the_recovery_in_a_block() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
        let in_block_output = stored_output_of(&exited.found, 100);
        let looked_up = |stored: Option<&LeafRecovery>, observed: &[Observation]| {
            looked_up_exit(
                &exited.leaf,
                true,
                found(&exited, Some(100)),
                stored,
                observed,
                1_000,
            )
            .unwrap()
        };
        let update = |spend| UpdateLeafRecovery {
            leaf_id: LEAF_ID.to_string(),
            chain_checked_at: Some(1_000),
            watchtower_exit_output: Some(in_block_output.clone()),
            watchtower_exit_spend: spend,
            ..Default::default()
        };
        let recovery = ChainTransaction {
            txid: spender().to_string(),
            block_height: 101,
        };

        let new = looked_up(None, &[unspent(&exited.found)]);
        assert_eq!(new.update, Some(update(None)));
        let exit = new.exit.unwrap();
        assert_eq!(exit.output, exited.found);
        assert!(!exit.recovered);

        let recovered = looked_up(None, &[spent(&exited.found, true, Some(101))]);
        assert_eq!(recovered.update, Some(update(Some(recovery.clone()))));
        assert!(recovered.exit.unwrap().recovered);

        // The SDK does not store again what storage already holds.
        let stored = LeafRecovery {
            chain_checked_at: Some(900),
            watchtower_exit_spend: Some(recovery),
            ..stored_with(Some(in_block_output.clone()))
        };
        let same = looked_up(Some(&stored), &[spent(&exited.found, true, Some(101))]);
        assert_eq!(same.update, None);
        assert!(same.exit.unwrap().recovered);

        // An output stored without a check time gets its check time.
        let unchecked = stored_with(Some(in_block_output.clone()));
        let checked = looked_up(Some(&unchecked), &[unspent(&exited.found)]);
        assert_eq!(checked.update, Some(update(None)));

        // The SDK replaces the stored output with another one it finds.
        let other = stored_with(Some(stored_output_of(&output(), 90)));
        let replaced = looked_up(Some(&other), &[unspent(&exited.found)]);
        assert_eq!(replaced.update, Some(update(None)));
        assert_eq!(replaced.exit.unwrap().output, exited.found);
    }

    #[test]
    fn a_lookup_stores_a_found_output_before_its_spend_is_known() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
        let in_block_output = stored_output_of(&exited.found, 100);
        // No query for the spend of the output has a result.
        let looked_up = |stored: Option<&LeafRecovery>| {
            looked_up_exit(
                &exited.leaf,
                true,
                found(&exited, Some(100)),
                stored,
                &[],
                1_000,
            )
            .unwrap()
        };
        let output_alone = Some(UpdateLeafRecovery {
            leaf_id: LEAF_ID.to_string(),
            watchtower_exit_output: Some(in_block_output.clone()),
            ..Default::default()
        });

        let new = looked_up(None);
        assert_eq!(new.update, output_alone);
        assert!(!new.exit.unwrap().recovered);
        assert_eq!(looked_up(Some(&stored_with(None))).update, output_alone);
        let other = stored_with(Some(stored_output_of(&output(), 90)));
        assert_eq!(looked_up(Some(&other)).update, output_alone);

        // Storage holds the output already, with or without a check time.
        let unchecked = stored_with(Some(in_block_output.clone()));
        assert_eq!(looked_up(Some(&unchecked)).update, None);
        let checked = LeafRecovery {
            chain_checked_at: Some(900),
            ..unchecked
        };
        assert_eq!(looked_up(Some(&checked)).update, None);
    }

    #[test]
    fn the_check_of_a_leaf_that_is_not_recovered_is_complete_once_its_output_is_found() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExited);

        let looked_up = looked_up_exit(
            &exited.leaf,
            false,
            found(&exited, Some(100)),
            None,
            &[],
            1_000,
        )
        .unwrap();

        assert_eq!(
            looked_up.update,
            Some(UpdateLeafRecovery {
                leaf_id: LEAF_ID.to_string(),
                chain_checked_at: Some(1_000),
                watchtower_exit_output: Some(stored_output_of(&exited.found, 100)),
                ..Default::default()
            })
        );
        assert!(!looked_up.exit.unwrap().recovered);
    }

    #[test]
    fn a_spend_is_only_stored_with_the_output_it_spends() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
        let spend_in_block = [spent(&exited.found, true, Some(101))];
        // The chain service did not name the block of the output's transaction.
        let looked_up = |stored: Option<&LeafRecovery>| {
            looked_up_exit(
                &exited.leaf,
                true,
                found(&exited, None),
                stored,
                &spend_in_block,
                1_000,
            )
            .unwrap()
        };
        let spend = Some(ChainTransaction {
            txid: spender().to_string(),
            block_height: 101,
        });

        let update = looked_up(None).update.unwrap();
        assert_eq!(
            (update.watchtower_exit_output, update.watchtower_exit_spend),
            (None, spend)
        );

        let other = stored_with(Some(stored_output_of(&output(), 90)));
        let on_other = looked_up(Some(&other));
        assert_eq!(on_other.update, None);
        assert!(on_other.exit.unwrap().recovered);
    }

    #[test]
    fn a_stored_output_takes_precedence_over_the_direct_tx_the_node_holds() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExited);
        let held = output_at("05", 9_800);
        let stored = stored_with(Some(stored_output_of(&exited.found, 100)));
        let looked_up = |lookup: WatchtowerExitLookup, stored: Option<&LeafRecovery>| {
            looked_up_exit(&exited.leaf, false, lookup, stored, &[], 1_000).unwrap()
        };

        for lookup in [
            WatchtowerExitLookup::Unconfirmed(held.clone()),
            WatchtowerExitLookup::NotFound,
            WatchtowerExitLookup::Pending,
        ] {
            let found = looked_up(lookup, Some(&stored));
            assert_eq!(found.update, None);
            assert_eq!(found.exit.unwrap().output, exited.found);
        }

        let unconfirmed = looked_up(WatchtowerExitLookup::Unconfirmed(held.clone()), None);
        assert_eq!(unconfirmed.update, None);
        assert_eq!(unconfirmed.exit.unwrap().output, held);
    }

    #[test]
    fn a_lookup_without_an_output_is_what_the_leaf_is_left_with() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExited);

        for lookup in [
            WatchtowerExitLookup::Unrecoverable,
            WatchtowerExitLookup::Unilateral,
            WatchtowerExitLookup::NotFound,
            WatchtowerExitLookup::Pending,
        ] {
            for stored in [None, Some(stored_with(None))] {
                let found = looked_up_exit(
                    &exited.leaf,
                    false,
                    lookup.clone(),
                    stored.as_ref(),
                    &[],
                    1_000,
                )
                .unwrap();
                assert_eq!(found.update, None);
                assert_eq!(found.exit, Err(lookup.clone()));
            }
        }
    }

    #[test]
    fn a_leaf_without_its_ancestors_is_not_looked_for() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExited);
        let on_chain = exited_leaf(TreeNodeStatus::OnChain);
        let not_found = Some(WatchtowerExitLookup::NotFound);

        assert_eq!(
            lookup_with(not_found.clone(), &exited.leaf, false),
            WatchtowerExitLookup::Pending
        );
        assert_eq!(
            lookup_with(not_found.clone(), &exited.leaf, true),
            WatchtowerExitLookup::NotFound
        );
        assert_eq!(
            lookup_with(not_found, &on_chain.leaf, false),
            WatchtowerExitLookup::NotFound
        );
        assert_eq!(
            lookup_with(Some(found(&exited, Some(100))), &exited.leaf, false),
            found(&exited, Some(100))
        );
        assert_eq!(
            lookup_with(None, &exited.leaf, true),
            WatchtowerExitLookup::Pending
        );
    }

    fn check_of(exited: &ExitedLeaf, fetched: &[Observation]) -> Vec<UpdateLeafRecovery> {
        recovered_leaf_checks(
            std::slice::from_ref(&exited.leaf),
            &HashMap::new(),
            &HashMap::new(),
            fetched,
            1_000,
        )
    }

    #[test]
    fn a_recovered_leaf_is_stored_with_its_output_and_the_recovery_in_a_block() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
        let found = vec![exited.parent_spent.clone(), exited.direct_tx.clone()];
        let stored = |spend| UpdateLeafRecovery {
            leaf_id: LEAF_ID.to_string(),
            chain_checked_at: Some(1_000),
            watchtower_exit_output: Some(stored_output_of(&exited.found, 100)),
            watchtower_exit_spend: spend,
            ..Default::default()
        };
        let checked = |outspend: Observation| {
            let mut fetched = found.clone();
            fetched.push(outspend);
            check_of(&exited, &fetched)
        };

        assert_eq!(
            checked(spent(&exited.found, true, Some(101))),
            vec![stored(Some(ChainTransaction {
                txid: spender().to_string(),
                block_height: 101,
            }))]
        );
        // With a recovery in the mempool, or none, the funds stay in the total.
        assert_eq!(
            checked(spent(&exited.found, false, None)),
            vec![stored(None)]
        );
        assert_eq!(checked(unspent(&exited.found)), vec![stored(None)]);
    }

    #[test]
    fn a_recovered_leaf_without_a_found_output_is_left_for_the_next_sync() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);

        for fetched in [Vec::new(), vec![exited.parent_spent.clone()]] {
            assert!(check_of(&exited, &fetched).is_empty());
        }
    }

    #[test]
    fn a_found_output_is_stored_before_its_spend_is_known() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
        let found = vec![exited.parent_spent.clone(), exited.direct_tx.clone()];
        let output_alone = vec![UpdateLeafRecovery {
            leaf_id: LEAF_ID.to_string(),
            watchtower_exit_output: Some(stored_output_of(&exited.found, 100)),
            ..Default::default()
        }];

        assert_eq!(check_of(&exited, &found), output_alone);
        // The SDK stores a transaction only with the height of its block.
        let unnamed_block = [found, vec![spent(&exited.found, true, None)]].concat();
        assert_eq!(check_of(&exited, &unnamed_block), output_alone);
    }

    #[test]
    fn a_leaf_with_a_stored_output_needs_only_the_spend_of_that_output() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
        let leaves = [exited.leaf.clone()];
        let nodes = HashMap::new();
        let stored = HashMap::from([(
            LEAF_ID.to_string(),
            LeafRecovery {
                watchtower_exit_output: Some(stored_output_of(&exited.found, 100)),
                ..stored_leaf(&[])
            },
        )]);
        let checked = |spend| UpdateLeafRecovery {
            leaf_id: LEAF_ID.to_string(),
            chain_checked_at: Some(1_000),
            watchtower_exit_spend: spend,
            ..Default::default()
        };
        let check = |fetched: &[Observation]| {
            recovered_leaf_checks(&leaves, &nodes, &stored, fetched, 1_000)
        };

        assert_eq!(
            recovered_leaf_queries(&leaves, &nodes, &stored, &[]),
            vec![ChainQuery::Outspend(exited.found.outpoint)]
        );
        assert!(check(&[]).is_empty());
        assert_eq!(check(&[unspent(&exited.found)]), vec![checked(None)]);
        assert_eq!(
            check(&[spent(&exited.found, true, Some(101))]),
            vec![checked(Some(ChainTransaction {
                txid: spender().to_string(),
                block_height: 101,
            }))]
        );
    }

    #[cfg(not(target_family = "wasm"))]
    mod stored_checks {
        use std::sync::{Arc, atomic::Ordering};

        use crate::{
            chain::{Outspend, stub::ChainStub},
            persist::sqlite::SqliteStorage,
        };

        use super::*;

        fn temp_storage() -> SqliteStorage {
            let mut dir = std::env::temp_dir();
            dir.push(format!("breez-recovered-leaves-{}", uuid::Uuid::new_v4()));
            SqliteStorage::new(&dir).expect("create sqlite storage")
        }

        /// A chain service that returns `known` and fails every other request.
        fn chain_knowing(known: &[Observation]) -> Arc<ChainStub> {
            let mut chain = ChainStub::default();
            for observation in known {
                match (&observation.query, &observation.result) {
                    (ChainQuery::Outspend(outpoint), ChainResult::Spend(spend)) => {
                        let outspend = spend.as_ref().map_or(Outspend::Unspent, |spend| {
                            ChainStub::spent(
                                &spend.spender_txid.to_string(),
                                spend.confirmed,
                                spend.block_height,
                            )
                        });
                        chain
                            .outspends
                            .insert((outpoint.txid.to_string(), outpoint.vout), outspend);
                    }
                    (ChainQuery::Transaction(txid), ChainResult::Transaction(tx)) => {
                        chain
                            .transactions
                            .insert(txid.to_string(), serialize_hex(tx));
                    }
                    other => panic!("the stub cannot return {other:?}"),
                }
            }
            Arc::new(chain)
        }

        /// The observations that complete the check of `leaf`, with its output
        /// unspent.
        fn all_of(leaf: &ExitedLeaf) -> Vec<Observation> {
            vec![
                leaf.parent_spent.clone(),
                leaf.direct_tx.clone(),
                unspent(&leaf.found),
            ]
        }

        async fn stored_in(storage: &SqliteStorage) -> HashMap<String, LeafRecovery> {
            storage
                .list_leaf_recoveries()
                .await
                .unwrap()
                .into_iter()
                .map(|leaf| (leaf.leaf_id.clone(), leaf))
                .collect()
        }

        fn leaf_number(number: u8) -> ExitedLeaf {
            exited_leaf_at(
                &format!("00000000-0000-0000-0000-0000000000{number:02}"),
                &format!("{:02x}", number.saturating_add(0x10)),
                TreeNodeStatus::WatchtowerExitRecovered,
            )
        }

        #[tokio::test]
        async fn a_failed_request_keeps_the_found_output_and_the_next_sync_continues_from_it() {
            let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
            let leaves = [exited.leaf.clone()];
            let storage = temp_storage();

            // The chain service fails the request for the spend of the output.
            let chain = chain_knowing(&[exited.parent_spent.clone(), exited.direct_tx.clone()]);
            let mut queries = ChainQueries::for_sync(chain.clone());
            let complete = store_recovered_leaf_checks(
                &leaves,
                &HashMap::new(),
                &HashMap::new(),
                &mut queries,
                &storage,
            )
            .await;

            let stored = stored_in(&storage).await;
            assert!(complete.is_empty());
            assert_eq!(chain.requests.load(Ordering::SeqCst), 3);
            assert_eq!(
                stored[LEAF_ID].watchtower_exit_output,
                Some(stored_output_of(&exited.found, 100))
            );
            assert_eq!(stored[LEAF_ID].chain_checked_at, None);

            let chain = chain_knowing(&[spent(&exited.found, true, Some(101))]);
            let mut queries = ChainQueries::for_sync(chain.clone());
            let complete = store_recovered_leaf_checks(
                &leaves,
                &HashMap::new(),
                &stored,
                &mut queries,
                &storage,
            )
            .await;

            let stored = stored_in(&storage).await;
            assert_eq!(complete, vec![LEAF_ID.to_string()]);
            assert_eq!(chain.requests.load(Ordering::SeqCst), 1);
            assert!(stored[LEAF_ID].chain_checked_at.is_some());
            assert_eq!(
                stored[LEAF_ID].watchtower_exit_spend,
                Some(ChainTransaction {
                    txid: spender().to_string(),
                    block_height: 101,
                })
            );
        }

        #[tokio::test]
        async fn a_lookup_keeps_the_found_output_and_a_later_one_falls_back_on_it() {
            let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
            let leaves = [exited.leaf.clone()];
            let storage = temp_storage();

            // The chain service fails the request for the spend of the output.
            let chain = chain_knowing(&[exited.parent_spent.clone(), exited.direct_tx.clone()]);
            let mut queries = ChainQueries::new(chain.clone());
            let found = store_watchtower_exit_lookups(
                &leaves,
                &HashMap::new(),
                true,
                &HashMap::new(),
                &mut queries,
                &storage,
            )
            .await
            .unwrap();

            let stored = stored_in(&storage).await;
            assert_eq!(chain.requests.load(Ordering::SeqCst), 3);
            assert_eq!(found.exits[LEAF_ID].output, exited.found);
            assert_eq!(
                stored[LEAF_ID].watchtower_exit_output,
                Some(stored_output_of(&exited.found, 100))
            );
            assert_eq!(stored[LEAF_ID].chain_checked_at, None);

            // The chain service fails every request.
            let chain = chain_knowing(&[]);
            let mut queries = ChainQueries::new(chain.clone());
            let found = store_watchtower_exit_lookups(
                &leaves,
                &HashMap::new(),
                true,
                &stored,
                &mut queries,
                &storage,
            )
            .await
            .unwrap();

            assert_eq!(chain.requests.load(Ordering::SeqCst), 1);
            assert_eq!(found.exits[LEAF_ID].output, exited.found);
            assert!(found.missing.is_empty());
        }

        #[tokio::test]
        async fn the_spend_of_an_output_is_requested_only_once_a_recovery_is_cosigned() {
            let exited = exited_leaf(TreeNodeStatus::WatchtowerExited);
            let exit = |recovery_cosigned| {
                WatchtowerExit::new(
                    LEAF_ID.to_string(),
                    exited.leaf.value,
                    exited.found.clone(),
                    None,
                    &OutputSpend::Unknown,
                    recovery_cosigned,
                )
            };
            let chain = chain_knowing(&[spent(&exited.found, true, Some(101))]);
            let mut queries = ChainQueries::new(chain.clone());

            let spend = exit_output_spend(&exit(false), &mut queries).await;
            assert_eq!(spend, OutputSpend::Unspent);
            assert_eq!(chain.requests.load(Ordering::SeqCst), 0);

            let spend = exit_output_spend(&exit(true), &mut queries).await;
            assert!(matches!(spend, OutputSpend::InBlock { .. }));
            assert_eq!(chain.requests.load(Ordering::SeqCst), 1);
        }

        #[tokio::test]
        async fn a_recovered_leaf_below_an_on_chain_split_node_takes_two_requests() {
            let split_id = "00000000-0000-0000-0000-00000000000c";
            let outpoint = |byte: &str| OutPoint {
                txid: Txid::from_str(&byte.repeat(32)).unwrap(),
                vout: 0,
            };
            let mut leaf = create_test_node_with_parent(
                LEAF_ID,
                Some(split_id),
                TreeNodeStatus::WatchtowerExitRecovered,
            );
            let leaf_script = ScriptBuf::new_p2tr(
                &bitcoin::secp256k1::Secp256k1::verification_only(),
                leaf.verifying_public_key.x_only_public_key().0,
                None,
            );
            let mut own = tx_paying(outpoint("05"), 9_800);
            own.output[0].script_pubkey = leaf_script.clone();
            leaf.direct_tx = Some(own);
            let mut held = tx_paying(outpoint("06"), 9_900);
            held.output[0].script_pubkey = leaf_script;
            let mut split = create_test_node_with_parent(split_id, None, TreeNodeStatus::OnChain);
            split.direct_tx = Some(held.clone());
            let found = WatchtowerExitOutput {
                leaf_id: leaf.id.clone(),
                outpoint: OutPoint {
                    txid: held.compute_txid(),
                    vout: 0,
                },
                tx_out: held.output[0].clone(),
            };
            let nodes = HashMap::from([(leaf.id.clone(), leaf.clone()), (split.id.clone(), split)]);
            // The split node's direct tx is in a block, and nothing spent its
            // output.
            let chain = chain_knowing(&[
                Observation {
                    query: ChainQuery::Outspend(outpoint("06")),
                    result: ChainResult::Spend(Some(SpendInfo {
                        spender_txid: held.compute_txid(),
                        confirmed: true,
                        block_height: Some(100),
                    })),
                },
                unspent(&found),
            ]);
            let storage = temp_storage();
            let mut queries = ChainQueries::for_sync(chain.clone());

            let complete = store_recovered_leaf_checks(
                &[leaf],
                &nodes,
                &HashMap::new(),
                &mut queries,
                &storage,
            )
            .await;

            assert_eq!(complete, vec![LEAF_ID.to_string()]);
            // The input of the split node, and the spend of the output found.
            assert_eq!(chain.requests.load(Ordering::SeqCst), 2);
            assert_eq!(
                stored_in(&storage).await[LEAF_ID].watchtower_exit_output,
                Some(stored_output_of(&found, 100))
            );
        }

        #[tokio::test]
        async fn a_sync_stores_the_batches_it_finished_and_the_next_one_checks_the_rest() {
            let exited: Vec<ExitedLeaf> = (0..12).map(leaf_number).collect();
            let leaves: Vec<TreeNode> = exited.iter().map(|exited| exited.leaf.clone()).collect();
            let storage = temp_storage();

            // The chain service fails every request for the last two leaves, which
            // are in the second batch.
            let known: Vec<Observation> = exited.iter().take(10).flat_map(all_of).collect();
            let chain = chain_knowing(&known);
            let mut queries = ChainQueries::for_sync(chain.clone());
            let complete = store_recovered_leaf_checks(
                &leaves,
                &HashMap::new(),
                &HashMap::new(),
                &mut queries,
                &storage,
            )
            .await;

            let stored = stored_in(&storage).await;
            assert_eq!(complete.len(), 10);
            assert_eq!(stored.len(), 10);
            assert!(stored.values().all(|leaf| leaf.chain_checked_at.is_some()));
            // Three requests for each leaf of the first batch, and the one that
            // failed.
            assert_eq!(chain.requests.load(Ordering::SeqCst), 31);

            let known: Vec<Observation> = exited.iter().skip(10).flat_map(all_of).collect();
            let chain = chain_knowing(&known);
            let mut queries = ChainQueries::for_sync(chain.clone());
            let complete = store_recovered_leaf_checks(
                &leaves[10..],
                &HashMap::new(),
                &stored,
                &mut queries,
                &storage,
            )
            .await;

            assert_eq!(complete.len(), 2);
            assert_eq!(stored_in(&storage).await.len(), 12);
            assert_eq!(chain.requests.load(Ordering::SeqCst), 6);
        }
    }

    #[test]
    fn a_recovered_leaf_the_chain_shows_no_output_for_stays_to_recover() {
        let exited = exited_leaf(TreeNodeStatus::WatchtowerExitRecovered);
        let fetched = [Observation {
            query: exited.parent_spent.query.clone(),
            result: ChainResult::Spend(None),
        }];

        assert_eq!(
            check_of(&exited, &fetched),
            vec![UpdateLeafRecovery {
                leaf_id: LEAF_ID.to_string(),
                chain_checked_at: Some(1_000),
                ..Default::default()
            }]
        );
    }
}
