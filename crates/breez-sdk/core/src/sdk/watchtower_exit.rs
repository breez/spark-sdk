use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    str::FromStr,
    sync::Arc,
};

use bitcoin::{
    Address, Amount, ScriptBuf, Transaction, TxOut, Txid,
    consensus::encode::{deserialize_hex, serialize_hex},
    hex::{DisplayHex, FromHex},
};
use spark_wallet::{
    ChainResult, Observation, ResolvedWatchtowerExits, TreeNode, TreeNodeId, TreeNodeStatus,
    UnsignedWatchtowerExitRecovery, WatchtowerExitedOutput, build_watchtower_exit_recovery,
    scan_watchtower_exits,
};
use tracing::{error, info, warn};

use crate::{
    CooperativeRecoveryError,
    chain::{BitcoinChainService, Outspend},
    error::SdkError,
    persist::{
        CachedWatchtowerExit, CachedWatchtowerExitOutput, CachedWatchtowerExitRecovery,
        ObjectCacheRepository, SeenAt,
    },
    utils::replacement::Replaced,
};

use super::{BreezSdk, recover_funds::run_checks, unilateral_exit::execute_chain_query};

impl BreezSdk {
    /// Adds up the watchtower-exited leaves not recovered yet. Returns that
    /// value, how many of the leaves are new, and the leaves tracked.
    ///
    /// A leaf gets its record the first time a sync sees it. The operators
    /// report a recovered leaf the same whether or not its recovery confirmed,
    /// so the chain is asked about that leaf first, and it stays out of the
    /// value until the chain answered.
    pub(super) async fn sync_watchtower_exits(
        &self,
        leaves: &[TreeNode],
    ) -> (u64, usize, HashSet<String>) {
        let repository = ObjectCacheRepository::new(self.storage.clone());
        let mut sats: u64 = 0;
        let mut found: usize = 0;
        let mut tracked = HashSet::new();
        let mut unchecked: Vec<TreeNode> = Vec::new();
        for leaf in leaves {
            let leaf_id = leaf.id.to_string();
            let exit = match repository.fetch_watchtower_exit(&leaf_id).await {
                Ok(exit) => exit,
                Err(e) => {
                    error!("Failed to read the watchtower exit of leaf {leaf_id}: {e}");
                    continue;
                }
            };
            let exit = match (exit, leaf.status) {
                (Some(exit), _) => exit,
                (None, TreeNodeStatus::WatchtowerExited) => {
                    match self.track_watchtower_exit(leaf, None, None).await {
                        Ok(exit) => {
                            if !self.exit_reported(&leaf_id).await {
                                found = found.saturating_add(1);
                            }
                            exit
                        }
                        Err(e) => {
                            error!("Failed to store the watchtower exit of leaf {leaf_id}: {e}");
                            continue;
                        }
                    }
                }
                (None, TreeNodeStatus::WatchtowerExitRecovered) => {
                    unchecked.push(leaf.clone());
                    continue;
                }
                // A quote stores the output of an on-chain leaf it recovers
                // cooperatively.
                (None, _) => continue,
            };
            tracked.insert(leaf_id);
            if exit.recovered.is_none() {
                sats = sats.saturating_add(leaf.value);
            }
        }
        for (leaf, exit) in self.check_recovered_leaves(unchecked).await {
            let leaf_id = leaf.id.to_string();
            if exit.recovered.is_none() {
                sats = sats.saturating_add(leaf.value);
                if !self.exit_reported(&leaf_id).await {
                    found = found.saturating_add(1);
                }
            }
            tracked.insert(leaf_id);
        }
        if found > 0 {
            info!("Found {found} leaves a watchtower exit took on-chain");
        }
        (sats, found, tracked)
    }

    /// Whether an earlier sync reported the leaf as one exiting unilaterally.
    async fn exit_reported(&self, leaf_id: &str) -> bool {
        matches!(
            ObjectCacheRepository::new(self.storage.clone())
                .fetch_exiting_leaf(leaf_id)
                .await,
            Ok(Some(_))
        )
    }

    /// Asks the chain whether the recovery of each of `leaves` confirmed, and
    /// records the leaves it got an answer for. The others stay without a
    /// record, so the next sync asks again.
    async fn check_recovered_leaves(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Vec<(TreeNode, CachedWatchtowerExit)> {
        if leaves.is_empty() {
            return Vec::new();
        }
        let leaf_ids: Vec<TreeNodeId> = leaves.iter().map(|leaf| leaf.id.clone()).collect();
        let nodes = match self
            .spark_wallet
            .fetch_nodes_with_ancestors(&leaf_ids)
            .await
        {
            Ok(nodes) => Arc::new(nodes),
            Err(e) => {
                warn!("Failed to fetch the ancestors of recovered leaves: {e}");
                return Vec::new();
            }
        };
        let Some(tip) = self.tip_for_checks().await else {
            return Vec::new();
        };
        let checks = leaves
            .into_iter()
            .map(|leaf| {
                let sdk = self.clone();
                let nodes = nodes.clone();
                async move {
                    let exit = sdk.check_recovered_leaf(&leaf, &nodes, tip).await?;
                    Some((leaf, exit))
                }
            })
            .collect();
        run_checks(checks).await
    }

    /// `None` when the chain did not answer.
    async fn check_recovered_leaf(
        &self,
        leaf: &TreeNode,
        nodes: &HashMap<TreeNodeId, TreeNode>,
        tip: u32,
    ) -> Option<CachedWatchtowerExit> {
        let (resolved, unread) = resolve_watchtower_exit_outputs(
            self.chain_service.as_ref(),
            std::slice::from_ref(leaf),
            nodes,
        )
        .await;
        if unread {
            return None;
        }
        let shown = resolved.outputs.into_iter().map(|output| (output, true));
        let assumed = resolved.assumed.into_iter().map(|output| (output, false));
        let found = shown
            .chain(assumed)
            .map(|(output, on_chain)| LookedUpOutput { output, on_chain })
            .next();
        // Only a recovery spends the output.
        let recovered = match &found {
            Some(found) => {
                let outpoint = found.output.outpoint;
                match self
                    .chain_service
                    .get_outspend(outpoint.txid.to_string(), outpoint.vout)
                    .await
                {
                    Ok(Outspend::Spent { status, .. }) => status.confirmed,
                    Ok(Outspend::Unspent) => false,
                    Err(e) => {
                        warn!(
                            "Outspend lookup failed for watchtower-exited output {outpoint}: {e}"
                        );
                        return None;
                    }
                }
            }
            None => false,
        };
        let recorded = self
            .update_watchtower_exit(&leaf.id.to_string(), |stored| {
                let mut exit = stored.unwrap_or_else(|| CachedWatchtowerExit::tracking(leaf));
                if let Some(found) = &found {
                    let seen = found.on_chain.then_some(SeenAt { tip: Some(tip) });
                    exit.take_output(&found.output, seen);
                }
                exit.take_spend(recovered, Some(tip));
                Some(exit)
            })
            .await;
        match recorded {
            Ok(exit) => exit,
            Err(e) => {
                error!(
                    "Failed to store the watchtower exit of leaf {}: {e}",
                    leaf.id
                );
                None
            }
        }
    }

    /// Records that a recovery of each of these leaves confirmed. Failures are
    /// only logged.
    pub(super) async fn record_recovered(&self, leaf_ids: &[&str], tip: Option<u32>) {
        for leaf_id in leaf_ids {
            let recorded = self
                .update_watchtower_exit(leaf_id, |stored| {
                    stored.map(|mut stored| {
                        stored.take_spend(true, tip);
                        stored
                    })
                })
                .await;
            if let Err(e) = recorded {
                error!("Failed to record the recovery of leaf {leaf_id}: {e}");
            }
        }
    }

    pub(super) async fn fetch_watchtower_exit(
        &self,
        leaf_id: &str,
    ) -> Result<Option<CachedWatchtowerExit>, SdkError> {
        Ok(ObjectCacheRepository::new(self.storage.clone())
            .fetch_watchtower_exit(leaf_id)
            .await?)
    }

    /// The stored exits of `leaves`, after a lookup of the outputs not settled
    /// at `tip`.
    pub(super) async fn lookup_watchtower_exits(
        &self,
        leaves: &[TreeNode],
        tip: Option<u32>,
    ) -> Result<Vec<CachedWatchtowerExit>, SdkError> {
        let mut stored = Vec::with_capacity(leaves.len());
        let mut due = Vec::new();
        for leaf in leaves {
            let exit = self.fetch_watchtower_exit(&leaf.id.to_string()).await?;
            if !output_settled(exit.as_ref(), tip) {
                due.push(leaf.clone());
            }
            stored.push((leaf, exit));
        }
        let mut outputs = self.lookup_outputs(&due).await;
        let mut exits = Vec::with_capacity(stored.len());
        for (leaf, exit) in stored {
            let exit = match outputs.remove(&leaf.id.to_string()) {
                Some(output) => Some(self.track_watchtower_exit(leaf, Some(&output), tip).await?),
                None => exit,
            };
            exits.extend(exit);
        }
        Ok(exits)
    }

    async fn lookup_outputs(&self, leaves: &[TreeNode]) -> HashMap<String, LookedUpOutput> {
        let leaves: Vec<TreeNode> = {
            let checks = self.recovery_checks.lock().await;
            leaves
                .iter()
                .filter(|leaf| {
                    let leaf_id = leaf.id.to_string();
                    !checks.unrecoverable.contains(&leaf_id)
                        && !checks.unilateral.contains(&leaf_id)
                })
                .cloned()
                .collect()
        };
        if leaves.is_empty() {
            return HashMap::new();
        }
        // The output of an on-chain leaf is in its own direct tx, so only the
        // others need their ancestors.
        let exited: Vec<TreeNodeId> = leaves
            .iter()
            .filter(|leaf| leaf.status != TreeNodeStatus::OnChain)
            .map(|leaf| leaf.id.clone())
            .collect();
        let nodes = if exited.is_empty() {
            HashMap::new()
        } else {
            match self.spark_wallet.fetch_nodes_with_ancestors(&exited).await {
                Ok(nodes) => nodes,
                Err(e) => {
                    warn!("Failed to fetch the ancestors of watchtower-exited leaves: {e}");
                    HashMap::new()
                }
            }
        };
        let (resolved, _) =
            resolve_watchtower_exit_outputs(self.chain_service.as_ref(), &leaves, &nodes).await;
        if !resolved.unrecoverable.is_empty() || !resolved.unilateral.is_empty() {
            let mut checks = self.recovery_checks.lock().await;
            for leaf_id in resolved.unrecoverable {
                warn!(
                    "No cooperative recovery reaches watchtower-exited leaf {leaf_id}: the direct \
                     tx that confirmed above it pays no output to its key"
                );
                checks.unrecoverable.insert(leaf_id.to_string());
            }
            checks
                .unilateral
                .extend(resolved.unilateral.iter().map(ToString::to_string));
        }
        let shown = resolved.outputs.into_iter().map(|output| (output, true));
        let assumed = resolved.assumed.into_iter().map(|output| (output, false));
        shown
            .chain(assumed)
            .map(|(output, on_chain)| {
                (
                    output.leaf_id.to_string(),
                    LookedUpOutput { output, on_chain },
                )
            })
            .collect()
    }

    async fn track_watchtower_exit(
        &self,
        leaf: &TreeNode,
        found: Option<&LookedUpOutput>,
        tip: Option<u32>,
    ) -> Result<CachedWatchtowerExit, SdkError> {
        let tracked = self
            .update_watchtower_exit(&leaf.id.to_string(), |stored| {
                let mut exit = stored.unwrap_or_else(|| CachedWatchtowerExit::tracking(leaf));
                if let Some(found) = found {
                    exit.take_output(&found.output, found.on_chain.then_some(SeenAt { tip }));
                }
                Some(exit)
            })
            .await?;
        tracked
            .ok_or_else(|| SdkError::Generic("The funds of the leaf were not recorded".to_string()))
    }

    async fn update_watchtower_exit(
        &self,
        leaf_id: &str,
        update: impl FnOnce(Option<CachedWatchtowerExit>) -> Option<CachedWatchtowerExit>,
    ) -> Result<Option<CachedWatchtowerExit>, SdkError> {
        let _lock = self.recovery_state_lock.lock().await;
        let repository = ObjectCacheRepository::new(self.storage.clone());
        let stored = repository.fetch_watchtower_exit(leaf_id).await?;
        let updated = update(stored.clone());
        if updated != stored
            && let Some(exit) = &updated
        {
            repository.save_watchtower_exit(exit).await?;
        }
        Ok(updated)
    }

    /// The signed recovery of `exit` and its fee: one stored before, or else one
    /// the operators co-sign, unless they were found `unreachable`. `None` once
    /// a recovery of the output confirmed.
    pub(super) async fn cooperative_recovery(
        &self,
        exit: &CachedWatchtowerExit,
        destination: &Address,
        fee_rate_sat_per_vbyte: u64,
        unreachable: Option<&str>,
    ) -> Result<Option<(Transaction, u64)>, CooperativeRecoveryError> {
        if exit.recovered.is_some() {
            return Ok(None);
        }
        let output = exit.output().map_err(generic)?;
        let unsigned = build_recovery(&output, destination, fee_rate_sat_per_vbyte)
            .ok_or_else(|| generic("The output is too small to pay this fee"))?;
        match recovery_on_network(self.chain_service.as_ref(), exit).await? {
            RecoveryOnNetwork::Confirmed => {
                let tip = self.chain_service.tip_height().await.ok();
                if let Err(e) = self
                    .update_watchtower_exit(&exit.leaf_id, |stored| {
                        stored.map(|mut stored| {
                            stored.take_spend(true, tip);
                            stored
                        })
                    })
                    .await
                {
                    error!("Failed to store the recovery of leaf {}: {e}", exit.leaf_id);
                }
                return Ok(None);
            }
            RecoveryOnNetwork::Unconfirmed { txid, replaced } => {
                outbid(&unsigned, txid, &replaced)?;
            }
            RecoveryOnNetwork::None => {}
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
            .cosign_watchtower_exit_recovery(&output, unsigned.tx)
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
        if let Err(e) = self
            .update_watchtower_exit(&exit.leaf_id, |stored| {
                stored.map(|mut stored| {
                    stored.add_recovery(&signed, fee_rate_sat_per_vbyte);
                    stored
                })
            })
            .await
        {
            error!("Failed to store the recovery of leaf {}: {e}", exit.leaf_id);
        }
        Ok(Some((signed, unsigned.fee_sat)))
    }
}

/// Also returns whether a lookup went unanswered.
async fn resolve_watchtower_exit_outputs(
    chain: &dyn BitcoinChainService,
    leaves: &[TreeNode],
    nodes: &HashMap<TreeNodeId, TreeNode>,
) -> (ResolvedWatchtowerExits, bool) {
    let mut observed: Vec<Observation> = Vec::new();
    let mut unread = false;
    loop {
        let scan = scan_watchtower_exits(leaves, nodes, &observed);
        if scan.pending.is_empty() {
            return (scan.resolved, unread);
        }
        // A failed lookup records `Unavailable`, so each pass answers its queries
        // and the loop ends.
        for query in scan.pending {
            let result = execute_chain_query(chain, &query).await;
            unread |= matches!(result, ChainResult::Unavailable);
            observed.push(Observation { query, result });
        }
    }
}

fn output_settled(exit: Option<&CachedWatchtowerExit>, tip: Option<u32>) -> bool {
    exit.and_then(|exit| exit.output.as_ref())
        .and_then(|output| output.found)
        .is_some_and(|seen| seen.settled(tip))
}

struct LookedUpOutput {
    output: WatchtowerExitedOutput,
    /// False for the output of the direct tx a node holds, when the chain does
    /// not show which direct tx confirmed.
    on_chain: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum RecoveryOnNetwork {
    None,
    Unconfirmed { txid: Txid, replaced: Replaced },
    Confirmed,
}

impl RecoveryOnNetwork {
    fn unconfirmed(tx: &Transaction, amount_sats: u64) -> Option<Self> {
        Replaced::spending(tx, amount_sats).map(|replaced| Self::Unconfirmed {
            txid: tx.compute_txid(),
            replaced,
        })
    }
}

/// When the chain cannot be read, the stored recovery with the highest fee rate
/// is taken to be the one on the network.
async fn recovery_on_network(
    chain: &dyn BitcoinChainService,
    exit: &CachedWatchtowerExit,
) -> Result<RecoveryOnNetwork, CooperativeRecoveryError> {
    let Some(output) = exit.output.as_ref() else {
        return Ok(RecoveryOnNetwork::None);
    };
    match chain.get_outspend(output.txid.clone(), output.vout).await {
        Ok(Outspend::Unspent) => Ok(RecoveryOnNetwork::None),
        Ok(Outspend::Spent { status, .. }) if status.confirmed => Ok(RecoveryOnNetwork::Confirmed),
        Ok(Outspend::Spent { txid, .. }) => {
            let stored = exit
                .recoveries()
                .find(|stored| stored.compute_txid().to_string() == txid);
            let tx = if let Some(stored) = stored {
                stored
            } else {
                let tx_hex = chain
                    .get_transaction_hex(txid.clone())
                    .await
                    .map_err(generic)?;
                deserialize_hex(&tx_hex).map_err(generic)?
            };
            RecoveryOnNetwork::unconfirmed(&tx, output.amount_sats).ok_or_else(|| {
                generic(format!(
                    "Recovery {txid} pays out more than the output holds"
                ))
            })
        }
        Err(e) => {
            warn!(
                "Outspend lookup failed for watchtower-exited output {}:{}: {e}",
                output.txid, output.vout
            );
            Ok(exit
                .highest_fee_recovery()
                .and_then(|tx| RecoveryOnNetwork::unconfirmed(&tx, output.amount_sats))
                .unwrap_or(RecoveryOnNetwork::None))
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
    output: &WatchtowerExitedOutput,
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

impl CachedWatchtowerExit {
    fn tracking(leaf: &TreeNode) -> Self {
        Self {
            leaf_id: leaf.id.to_string(),
            value_sats: leaf.value,
            output: None,
            recoveries: Vec::new(),
            recovered: None,
        }
    }

    /// `seen` is unset for the output of a direct tx the chain has not shown,
    /// which never replaces an output the chain showed.
    fn take_output(&mut self, found: &WatchtowerExitedOutput, seen: Option<SeenAt>) {
        let txid = found.outpoint.txid.to_string();
        if let Some(stored) = self.output.as_mut() {
            if stored.txid == txid && stored.vout == found.outpoint.vout {
                stored.found = match (stored.found, seen) {
                    (Some(first), Some(now)) => Some(SeenAt {
                        tip: first.tip.or(now.tip),
                    }),
                    (first, now) => first.or(now),
                };
                return;
            }
            if stored.found.is_some() && seen.is_none() {
                return;
            }
        }
        self.output = Some(CachedWatchtowerExitOutput {
            txid,
            vout: found.outpoint.vout,
            amount_sats: found.tx_out.value.to_sat(),
            script_pubkey: found.tx_out.script_pubkey.as_bytes().to_lower_hex_string(),
            found: seen,
        });
        self.recoveries.clear();
        self.recovered = None;
    }

    fn take_spend(&mut self, confirmed: bool, tip: Option<u32>) {
        self.recovered = confirmed.then(|| SeenAt {
            tip: self.recovered.and_then(|seen| seen.tip).or(tip),
        });
    }

    fn add_recovery(&mut self, tx: &Transaction, fee_rate_sat_per_vbyte: u64) {
        let spends_output = self.output().is_ok_and(|output| {
            tx.input
                .first()
                .is_some_and(|input| input.previous_output == output.outpoint)
        });
        let txid = tx.compute_txid();
        if !spends_output
            || self
                .recoveries()
                .any(|stored| stored.compute_txid() == txid)
        {
            return;
        }
        self.recoveries.push(CachedWatchtowerExitRecovery {
            tx_hex: serialize_hex(tx),
            fee_rate_sat_per_vbyte,
        });
    }

    /// The fee of the transaction that took the funds on-chain.
    pub(super) fn exit_fee_sats(&self) -> u64 {
        self.output.as_ref().map_or(0, |output| {
            self.value_sats.saturating_sub(output.amount_sats)
        })
    }

    pub(super) fn output(&self) -> Result<WatchtowerExitedOutput, SdkError> {
        let stored = self.output.as_ref().ok_or_else(|| {
            SdkError::Generic(format!(
                "The funds of leaf {} were not found on-chain yet",
                self.leaf_id
            ))
        })?;
        let txid = Txid::from_str(&stored.txid)
            .map_err(|e| SdkError::Generic(format!("invalid stored txid: {e}")))?;
        let script_pubkey = Vec::<u8>::from_hex(&stored.script_pubkey)
            .map_err(|e| SdkError::Generic(format!("invalid stored script: {e}")))?;
        Ok(WatchtowerExitedOutput {
            leaf_id: self
                .leaf_id
                .parse()
                .map_err(|e| SdkError::Generic(format!("invalid stored leaf id: {e}")))?,
            outpoint: bitcoin::OutPoint {
                txid,
                vout: stored.vout,
            },
            tx_out: TxOut {
                value: Amount::from_sat(stored.amount_sats),
                script_pubkey: ScriptBuf::from_bytes(script_pubkey),
            },
        })
    }

    fn recoveries(&self) -> impl Iterator<Item = Transaction> + '_ {
        self.recoveries
            .iter()
            .filter_map(|recovery| deserialize_hex(&recovery.tx_hex).ok())
    }

    /// The stored recovery with the same txid as `unsigned`: a txid does not
    /// cover the signature.
    fn signed_recovery(&self, unsigned: &Transaction) -> Option<Transaction> {
        let txid = unsigned.compute_txid();
        self.recoveries()
            .find(|stored| stored.compute_txid() == txid)
    }

    fn highest_fee_recovery(&self) -> Option<Transaction> {
        let highest = self
            .recoveries
            .iter()
            .max_by_key(|recovery| recovery.fee_rate_sat_per_vbyte)?;
        deserialize_hex(&highest.tx_hex).ok()
    }
}

#[cfg(test)]
mod tests {
    use spark_wallet::tree_store_tests::create_test_node_with_parent;

    use crate::chain::stub::{ChainStub, tx_paying};

    use super::*;

    fn exit_with(recoveries: Vec<CachedWatchtowerExitRecovery>) -> CachedWatchtowerExit {
        CachedWatchtowerExit {
            leaf_id: "leaf".to_string(),
            value_sats: 10_955,
            output: Some(CachedWatchtowerExitOutput {
                txid: "00".repeat(32),
                vout: 0,
                amount_sats: 10_000,
                script_pubkey: String::new(),
                found: Some(SeenAt::default()),
            }),
            recoveries,
            recovered: None,
        }
    }

    fn paying(value: u64) -> Transaction {
        tx_paying(bitcoin::OutPoint::null(), value)
    }

    fn stored(tx: &Transaction, fee_rate_sat_per_vbyte: u64) -> CachedWatchtowerExitRecovery {
        CachedWatchtowerExitRecovery {
            tx_hex: serialize_hex(tx),
            fee_rate_sat_per_vbyte,
        }
    }

    #[test]
    fn a_stored_recovery_is_the_same_transaction_signed() {
        let unsigned = paying(9_500);
        let mut signed = unsigned.clone();
        signed.input[0].witness = bitcoin::Witness::from_slice(&[[1u8; 64]]);
        let exit = exit_with(vec![stored(&paying(9_000), 5), stored(&signed, 2)]);

        assert_eq!(exit.signed_recovery(&unsigned), Some(signed));
        assert_eq!(exit.signed_recovery(&paying(9_400)), None);
    }

    #[test]
    fn the_highest_fee_rate_recovery_is_the_one_assumed_out_there() {
        let exit = exit_with(vec![
            stored(&paying(9_800), 2),
            stored(&paying(9_000), 8),
            stored(&paying(9_500), 5),
        ]);

        assert_eq!(exit.highest_fee_recovery(), Some(paying(9_000)));
        assert!(exit_with(Vec::new()).highest_fee_recovery().is_none());
    }

    fn exit_spent_by(outspend: Option<Outspend>) -> ChainStub {
        match outspend {
            Some(outspend) => ChainStub::spending(
                bitcoin::OutPoint {
                    txid: Txid::from_str(&"00".repeat(32)).unwrap(),
                    vout: 0,
                },
                outspend,
            ),
            None => ChainStub::default(),
        }
    }

    #[macros::async_test_all]
    async fn an_unspent_output_needs_no_outbidding() {
        let chain = exit_spent_by(Some(Outspend::Unspent));

        let found = recovery_on_network(&chain, &exit_with(Vec::new())).await;

        assert_eq!(found, Ok(RecoveryOnNetwork::None));
    }

    #[macros::async_test_all]
    async fn a_confirmed_spend_is_a_confirmed_recovery() {
        let chain = exit_spent_by(Some(ChainStub::spent("theirs", true, Some(100))));

        let found = recovery_on_network(&chain, &exit_with(Vec::new())).await;

        assert_eq!(found, Ok(RecoveryOnNetwork::Confirmed));
    }

    #[macros::async_test_all]
    async fn a_stored_recovery_in_the_mempool_is_outbid_on_what_it_pays() {
        let ours = paying(9_700);
        let chain = exit_spent_by(Some(ChainStub::spent(
            &ours.compute_txid().to_string(),
            false,
            None,
        )));

        let found = recovery_on_network(&chain, &exit_with(vec![stored(&ours, 2)])).await;

        assert_eq!(
            found,
            Ok(RecoveryOnNetwork::Unconfirmed {
                txid: ours.compute_txid(),
                replaced: Replaced {
                    fee_sats: 300,
                    vsize: ours.vsize() as u64,
                },
            })
        );
    }

    #[macros::async_test_all]
    async fn a_recovery_from_elsewhere_is_read_from_the_chain() {
        let theirs = paying(9_000);
        let txid = theirs.compute_txid().to_string();
        let mut chain = exit_spent_by(Some(ChainStub::spent(&txid, false, None)));
        chain.transactions.insert(txid, serialize_hex(&theirs));

        let found = recovery_on_network(&chain, &exit_with(Vec::new())).await;

        assert_eq!(
            found,
            Ok(RecoveryOnNetwork::Unconfirmed {
                txid: theirs.compute_txid(),
                replaced: Replaced {
                    fee_sats: 1_000,
                    vsize: theirs.vsize() as u64,
                },
            })
        );
    }

    #[macros::async_test_all]
    async fn an_unreadable_chain_assumes_the_highest_fee_stored_recovery() {
        let chain = exit_spent_by(None);
        let exit = exit_with(vec![stored(&paying(9_800), 2), stored(&paying(9_400), 6)]);

        let found = recovery_on_network(&chain, &exit).await;

        assert_eq!(
            found,
            Ok(RecoveryOnNetwork::Unconfirmed {
                txid: paying(9_400).compute_txid(),
                replaced: Replaced {
                    fee_sats: 600,
                    vsize: paying(9_400).vsize() as u64,
                },
            })
        );
        assert_eq!(
            recovery_on_network(&chain, &exit_with(Vec::new())).await,
            Ok(RecoveryOnNetwork::None)
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

    #[macros::async_test_all]
    async fn an_on_chain_leaf_is_found_in_the_direct_tx_the_chain_shows() {
        let mut leaf = create_test_node_with_parent(
            "00000000-0000-0000-0000-00000000000a",
            None,
            TreeNodeStatus::OnChain,
        );
        let leaf_script = ScriptBuf::new_p2tr(
            &bitcoin::secp256k1::Secp256k1::verification_only(),
            leaf.verifying_public_key.x_only_public_key().0,
            None,
        );
        let parent_output = bitcoin::OutPoint {
            txid: Txid::from_str(&"03".repeat(32)).unwrap(),
            vout: 0,
        };
        let mut held = tx_paying(parent_output, 9_800);
        held.output[0].script_pubkey = leaf_script.clone();
        leaf.direct_tx = Some(held);
        let mut confirmed = tx_paying(parent_output, 9_500);
        confirmed.output[0].script_pubkey = leaf_script;
        let txid = confirmed.compute_txid().to_string();
        let mut chain =
            ChainStub::spending(parent_output, ChainStub::spent(&txid, true, Some(100)));
        chain.transactions.insert(txid, serialize_hex(&confirmed));
        let leaves = [leaf.clone()];

        let (resolved, unread) =
            resolve_watchtower_exit_outputs(&chain, &leaves, &HashMap::new()).await;
        assert!(!unread);
        assert_eq!(
            resolved.outputs,
            vec![WatchtowerExitedOutput {
                leaf_id: leaf.id.clone(),
                outpoint: bitcoin::OutPoint {
                    txid: confirmed.compute_txid(),
                    vout: 0,
                },
                tx_out: confirmed.output[0].clone(),
            }]
        );

        let unreachable = ChainStub::default();
        assert_eq!(
            resolve_watchtower_exit_outputs(&unreachable, &leaves, &HashMap::new()).await,
            (ResolvedWatchtowerExits::default(), true)
        );
    }

    fn seen_at(tip: u32) -> SeenAt {
        SeenAt { tip: Some(tip) }
    }

    fn exited_leaf() -> TreeNode {
        create_test_node_with_parent(
            "00000000-0000-0000-0000-00000000000a",
            None,
            TreeNodeStatus::WatchtowerExited,
        )
    }

    fn output_of(leaf: &TreeNode, txid: &str, value: u64) -> WatchtowerExitedOutput {
        WatchtowerExitedOutput {
            leaf_id: leaf.id.clone(),
            outpoint: bitcoin::OutPoint {
                txid: Txid::from_str(&txid.repeat(32)).unwrap(),
                vout: 0,
            },
            tx_out: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51, 0x20]),
            },
        }
    }

    #[test]
    fn a_tracked_exit_gives_back_the_output_a_lookup_found() {
        let leaf = exited_leaf();
        let output = output_of(&leaf, "01", 800);
        let mut exit = CachedWatchtowerExit::tracking(&leaf);
        assert!(exit.output().is_err());
        assert_eq!(exit.exit_fee_sats(), 0);

        exit.take_output(&output, Some(seen_at(100)));

        assert_eq!(exit.output().unwrap(), output);
        assert_eq!(exit.value_sats, leaf.value);
        assert_eq!(exit.exit_fee_sats(), leaf.value - 800);
    }

    #[test]
    fn another_output_replaces_the_stored_one_and_what_was_built_on_it() {
        let leaf = exited_leaf();
        let first = output_of(&leaf, "01", 800);
        let second = output_of(&leaf, "02", 700);
        let recovery = tx_paying(first.outpoint, 600);
        let mut exit = CachedWatchtowerExit::tracking(&leaf);
        exit.take_output(&first, Some(seen_at(100)));
        exit.add_recovery(&recovery, 2);
        exit.take_spend(true, Some(101));

        exit.take_output(&first, Some(seen_at(103)));
        assert_eq!(exit.output.as_ref().unwrap().found, Some(seen_at(100)));
        assert_eq!(exit.recoveries.len(), 1);
        assert!(exit.recovered.is_some());

        exit.take_output(&second, Some(seen_at(104)));
        assert_eq!(exit.output().unwrap(), second);
        assert_eq!(exit.output.as_ref().unwrap().found, Some(seen_at(104)));
        assert!(exit.recoveries.is_empty());
        assert_eq!(exit.recovered, None);

        exit.add_recovery(&recovery, 2);
        assert!(
            exit.recoveries.is_empty(),
            "a recovery of the replaced output is not kept"
        );
    }

    #[test]
    fn an_assumed_output_stands_until_the_chain_shows_one() {
        let leaf = exited_leaf();
        let held = output_of(&leaf, "01", 800);
        let confirmed = output_of(&leaf, "02", 700);
        let mut exit = CachedWatchtowerExit::tracking(&leaf);

        exit.take_output(&held, None);
        assert_eq!(exit.output().unwrap(), held);
        assert_eq!(exit.output.as_ref().unwrap().found, None);
        assert!(!output_settled(Some(&exit), Some(1_000)));
        exit.add_recovery(&tx_paying(held.outpoint, 600), 2);

        exit.take_output(&held, Some(seen_at(100)));
        assert_eq!(exit.output.as_ref().unwrap().found, Some(seen_at(100)));
        assert_eq!(exit.recoveries.len(), 1);

        exit.take_output(&confirmed, Some(seen_at(101)));
        assert_eq!(exit.output().unwrap(), confirmed);
        assert!(exit.recoveries.is_empty());

        exit.take_output(&held, None);
        assert_eq!(exit.output().unwrap(), confirmed);
    }

    #[test]
    fn a_confirmed_recovery_is_undone_until_it_is_settled() {
        let mut exit = exit_with(Vec::new());
        exit.take_spend(true, Some(100));
        exit.take_spend(true, Some(103));
        let seen = exit.recovered.unwrap();
        assert_eq!(seen, SeenAt { tip: Some(100) });
        assert!(!seen.settled(Some(104)));
        assert!(seen.settled(Some(105)));
        assert!(!seen.settled(None));
        assert!(!SeenAt::default().settled(Some(105)));

        exit.take_spend(false, Some(104));
        assert_eq!(exit.recovered, None);
    }
}
