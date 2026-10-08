use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bitcoin::{
    Address, Amount, FeeRate, OutPoint, ScriptBuf, Transaction, WPubkeyHash,
    address::NetworkUnchecked,
    consensus::encode::{deserialize_hex, serialize_hex},
    hashes::Hash,
};
use spark_wallet::{
    ChainQuery, EXITING_STATUSES, Fee, TreeNode, TreeNodeStatus, WATCHTOWER_EXITED_STATUSES,
    WatchtowerExitLookup, refund_sweep_input_fee, watchtower_exit_recovery_payout,
};
use tracing::{debug, error, info};

use crate::{
    ChainTransaction, CheckRecoverFundsRequest, CheckRecoverFundsResponse,
    CooperativeRecoveryError, CooperativeRecoveryFailure, ExitLeafSelection, ExitTransactionStatus,
    LeafRecovery, Network, PrepareRecoverFundsRequest, PrepareRecoverFundsResponse,
    RecoverFundsLeaf, RecoverFundsRequest, RecoverFundsResponse, RecoveryMethod,
    RecoveryRedoReason, RecoveryTransaction, RecoveryTxKind, RecoveryVerdict, SdkEvent,
    SkippedLeaf, SkippedLeafReason, UpdateLeafRecovery, error::SdkError,
    persist::ObjectCacheRepository, signer::CpfpSigner, utils::time::now_secs,
};

use super::{
    BreezSdk,
    chain_queries::{ChainQueries, without_result},
    unilateral_exit::{UnilateralBuild, UnilateralQuote, check_recovery_transactions, node_ids},
    watchtower_exit::{OutputSpend, WatchtowerExit, build_recovery, output_spend},
};

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Quotes a recovery of the selected leaves: how each is recovered, the exact
    /// fees, and how much to fund. It records a recovery or sweep it finds in a
    /// block, so the SDK leaves those funds out of the total to recover at the
    /// next sync.
    pub async fn prepare_recover_funds(
        &self,
        request: PrepareRecoverFundsRequest,
    ) -> Result<PrepareRecoverFundsResponse, SdkError> {
        let fee_rate = request.fee_rate_sat_per_vbyte;
        let destination = parse_destination(&request.destination, self.config.network)?;
        self.spark_wallet.refresh_before_exit().await;
        let stored = self.leaf_recoveries().await?;
        let mut queries = ChainQueries::new(self.chain_service.clone());
        let selection = self
            .split_selection(request.selection, &stored, &mut queries)
            .await?;

        let mut leaves = Vec::new();
        let mut skipped = selection.skipped;
        let mut cooperative_fee_sats = 0u64;
        for exit in selection.cooperative {
            let Some(recovery) = build_recovery(&exit.output, &destination, fee_rate) else {
                skipped.push(SkippedLeaf {
                    leaf_id: exit.leaf_id,
                    value_sats: exit.value_sats,
                    reason: SkippedLeafReason::FeeExceedsValue,
                });
                continue;
            };
            cooperative_fee_sats = cooperative_fee_sats
                .saturating_add(exit.exit_fee_sats())
                .saturating_add(recovery.fee_sat);
            leaves.push(RecoverFundsLeaf {
                leaf_id: exit.leaf_id,
                value_sats: exit.value_sats,
                method: RecoveryMethod::Cooperative,
            });
        }
        let cooperative_value_sats = total_value(&leaves);
        let unilateral = match selection.unilateral {
            Some(unilateral) => {
                self.quote_unilateral_exit(
                    &destination,
                    fee_rate,
                    request.funding_kind.as_ref(),
                    unilateral,
                    &selection.not_unilateral,
                    &stored,
                    &mut queries,
                )
                .await?
            }
            None => UnilateralQuote::default(),
        };
        leaves.extend(unilateral.leaves);
        skipped.extend(unilateral.skipped);

        let response = PrepareRecoverFundsResponse {
            leaves,
            skipped,
            recoverable_value_sats: cooperative_value_sats
                .saturating_add(unilateral.recoverable_value_sats),
            total_fee_sats: cooperative_fee_sats.saturating_add(unilateral.total_fee_sats),
            cooperative_fee_sats,
            cpfp_fee_sats: unilateral.cpfp_fee_sats,
            fanout_fee_sats: unilateral.fanout_fee_sats,
            sweep_fee_sats: unilateral.sweep_fee_sats,
            funding: unilateral.funding,
            fee_rate_sat_per_vbyte: fee_rate,
            destination: request.destination,
            exit_chain_state: unilateral.exit_chain_state,
        };
        debug!(
            leaves = response.leaves.len(),
            recoverable_value_sats = response.recoverable_value_sats,
            total_fee_sats = response.total_fee_sats,
            "prepare_recover_funds: quote ready"
        );
        Ok(response)
    }

    /// Builds and signs the recovery a `prepare_recover_funds` quote describes:
    /// the operators co-sign each cooperative recovery, and `signer` signs the
    /// spends of `funding_inputs` for the unilateral exit. Returns every
    /// transaction in broadcast order, without broadcasting.
    pub async fn recover_funds(
        &self,
        request: RecoverFundsRequest,
        signer: Option<Arc<dyn CpfpSigner>>,
    ) -> Result<RecoverFundsResponse, SdkError> {
        let RecoverFundsRequest {
            prepared,
            funding_inputs,
        } = request;
        let fee_rate = prepared.fee_rate_sat_per_vbyte;
        let destination = parse_destination(&prepared.destination, self.config.network)?;
        let (cooperative, unilateral): (Vec<&RecoverFundsLeaf>, Vec<&RecoverFundsLeaf>) = prepared
            .leaves
            .iter()
            .partition(|leaf| leaf.method == RecoveryMethod::Cooperative);

        // Looked up first, so an unknown leaf fails the call before anything is
        // signed.
        let mut queries = ChainQueries::new(self.chain_service.clone());
        let exits = self
            .quoted_watchtower_exits(&cooperative, &mut queries)
            .await?;

        let unilateral = if unilateral.is_empty() {
            UnilateralBuild::default()
        } else {
            let leaf_ids: Vec<String> =
                unilateral.iter().map(|leaf| leaf.leaf_id.clone()).collect();
            self.build_unilateral_exit(
                &destination,
                node_ids(&leaf_ids)?,
                fee_rate,
                funding_inputs.clone(),
                prepared
                    .funding
                    .as_ref()
                    .map_or(0, |funding| funding.single_utxo_sats),
                &prepared.exit_chain_state,
                signer.as_deref(),
                &mut queries,
            )
            .await?
        };

        let cooperative = self
            .cosign_cooperative_recoveries(exits, &destination, fee_rate, &mut queries)
            .await;
        let mut leaves = cooperative.leaves;
        let recoverable_value_sats =
            total_value(&leaves).saturating_add(unilateral.recoverable_value_sats);
        leaves.extend(unilateral.leaves);
        let mut transactions = cooperative.transactions;
        transactions.extend(unilateral.transactions);
        let response = RecoverFundsResponse {
            recoverable_value_sats,
            total_fee_sats: cooperative
                .fee_sats
                .saturating_add(unilateral.total_fee_sats),
            cooperative_fee_sats: cooperative.fee_sats,
            cpfp_fee_sats: unilateral.cpfp_fee_sats,
            fanout_fee_sats: unilateral.fanout_fee_sats,
            sweep_fee_sats: unilateral.sweep_fee_sats,
            leaves,
            failed: cooperative.failed,
            transactions,
            funding_inputs,
            fee_rate_sat_per_vbyte: fee_rate,
            destination: prepared.destination,
        };
        debug!(
            transactions = response.transactions.len(),
            failed = response.failed.len(),
            total_fee_sats = response.total_fee_sats,
            "recover_funds: complete"
        );
        Ok(response)
    }

    /// Reads a recovery you kept back against the chain: which of its
    /// transactions are now in a block, and whether it can still be finished as
    /// it stands.
    pub async fn check_recover_funds(
        &self,
        request: CheckRecoverFundsRequest,
    ) -> Result<CheckRecoverFundsResponse, SdkError> {
        let mut recovery = request.recovery;
        let (mut cooperative, mut unilateral): (Vec<_>, Vec<_>) =
            std::mem::take(&mut recovery.transactions)
                .into_iter()
                .partition(|tx| tx.kind == RecoveryTxKind::Cooperative);
        let outputs = cooperative
            .iter()
            .map(recovered_output)
            .collect::<Result<Vec<OutPoint>, SdkError>>()?;
        let mut queries = ChainQueries::new(self.chain_service.clone());
        queries
            .resolve(|observed| {
                let outspends = outputs.iter().copied().map(ChainQuery::Outspend).collect();
                ((), without_result(outspends, observed))
            })
            .await;
        let mut recovered: Vec<(String, ChainTransaction)> = Vec::new();
        for (tx, output) in cooperative.iter_mut().zip(&outputs) {
            let spend = output_spend(queries.observed(), *output);
            tx.status = cooperative_recovery_status(&spend, tx.status);
            if let (Some(leaf_id), Some(spend)) = (&tx.node_id, spend.in_block()) {
                recovered.push((leaf_id.clone(), spend));
            }
        }
        let diverged = check_recovery_transactions(&mut queries, &mut unilateral).await?;
        recovery.transactions = cooperative.into_iter().chain(unilateral).collect();
        self.record_finished(&recovery.transactions, recovered)
            .await;

        let all_confirmed = recovery
            .transactions
            .iter()
            .all(|tx| matches!(tx.status, ExitTransactionStatus::Confirmed { .. }));
        let verdict = if diverged {
            RecoveryVerdict::Redo {
                reason: RecoveryRedoReason::OnChainStateDiverged,
            }
        } else if all_confirmed && !recovery.transactions.is_empty() {
            RecoveryVerdict::Done
        } else {
            RecoveryVerdict::Valid
        };
        debug!(?verdict, "check_recover_funds: read back");
        Ok(CheckRecoverFundsResponse { recovery, verdict })
    }
}

#[derive(Default)]
struct CooperativeRecoveries {
    leaves: Vec<RecoverFundsLeaf>,
    transactions: Vec<RecoveryTransaction>,
    failed: Vec<CooperativeRecoveryFailure>,
    fee_sats: u64,
}

struct RecoverySelection {
    cooperative: Vec<WatchtowerExit>,
    /// Leaves only a cooperative recovery reaches, and that have none to quote.
    skipped: Vec<SkippedLeaf>,
    unilateral: Option<spark_wallet::ExitLeafSelection>,
    /// Leaves that no unilateral exit recovers.
    not_unilateral: HashSet<String>,
}

/// The leaves without a stored recovery, by what the SDK does with each in a sync.
#[derive(Default)]
struct NewLeaves {
    /// The SDK stores these as they are, without a chain check.
    to_recover: Vec<UpdateLeafRecovery>,
    /// The SDK checks these with the chain service first: the operators report
    /// a recovered leaf the same whether or not its recovery is in a block.
    recovered: Vec<TreeNode>,
    /// The SDK checks these with the chain service first: the operators report
    /// an exited leaf the same whether or not a sweep of its refund is in a
    /// block.
    exited: Vec<TreeNode>,
}

impl BreezSdk {
    pub(crate) async fn recoverable_funds_sats(&self) -> Result<u64, SdkError> {
        Ok(ObjectCacheRepository::new(self.storage.clone())
            .fetch_recoverable_funds()
            .await?
            .unwrap_or_default())
    }

    pub(super) async fn leaf_recoveries(&self) -> Result<HashMap<String, LeafRecovery>, SdkError> {
        Ok(self
            .storage
            .list_leaf_recoveries()
            .await?
            .into_iter()
            .map(|leaf| (leaf.leaf_id.clone(), leaf))
            .collect())
    }

    /// Returns whether storing `update` succeeded. The SDK only logs a failure:
    /// it can get what the update holds again, from the chain service or the
    /// operators.
    pub(super) async fn store_leaf_recovery(&self, update: UpdateLeafRecovery) -> bool {
        let leaf_id = update.leaf_id.clone();
        match self.storage.update_leaf_recovery(update).await {
            Ok(()) => true,
            Err(e) => {
                error!("Failed to store the recovery of leaf {leaf_id}: {e}");
                false
            }
        }
    }

    /// Stores the recovery of each leaf that has none stored, then the total of the
    /// funds to recover, and emits `RecoverableFunds` when it stored a leaf with
    /// funds still to recover. It stores a recovered or exited leaf once it knows
    /// from the chain service whether a recovery or sweep of the leaf's funds is
    /// in a block, or at once when the leaf is too small to send out.
    pub(super) async fn sync_recoverable_funds(&self) {
        let statuses = [EXITING_STATUSES.as_slice(), &WATCHTOWER_EXITED_STATUSES].concat();
        let leaves = match self.spark_wallet.list_leaves_with_status(&statuses).await {
            Ok(leaves) => leaves,
            Err(e) => {
                error!("Failed to list the leaves to recover: {e}");
                return;
            }
        };
        let mut stored = if leaves.is_empty() {
            HashMap::new()
        } else {
            match self.leaf_recoveries().await {
                Ok(stored) => stored,
                Err(e) => {
                    error!("Failed to read the stored leaf recoveries: {e}");
                    return;
                }
            }
        };
        let new = new_leaves(&leaves, &stored);
        let mut queries = ChainQueries::new(self.chain_service.clone());
        let recovered = self
            .check_recovered_leaves(&new.recovered, &mut queries)
            .await;
        let exited = self.check_exited_leaves(&new.exited, &mut queries).await;

        let mut new_leaf_ids: HashSet<String> = HashSet::new();
        let updates = new.to_recover.into_iter().chain(recovered).chain(exited);
        for update in updates {
            let leaf_id = update.leaf_id.clone();
            if self.store_leaf_recovery(update).await {
                new_leaf_ids.insert(leaf_id);
            }
        }
        if !new_leaf_ids.is_empty() {
            stored = match self.leaf_recoveries().await {
                Ok(stored) => stored,
                Err(e) => {
                    error!("Failed to read the stored leaf recoveries: {e}");
                    return;
                }
            };
        }
        let (funds, found) = recoverable_funds(&leaves, &stored, &new_leaf_ids);

        let repository = ObjectCacheRepository::new(self.storage.clone());
        let stored_funds = match repository.fetch_recoverable_funds().await {
            Ok(stored_funds) => stored_funds.unwrap_or_default(),
            Err(e) => {
                error!("Failed to read the recoverable funds: {e}");
                0
            }
        };
        if funds != stored_funds
            && let Err(e) = repository.save_recoverable_funds(funds).await
        {
            error!("Failed to store the recoverable funds: {e}");
        }
        if found > 0 {
            info!("Found {found} leaves with funds to recover");
            self.event_emitter
                .emit(&SdkEvent::RecoverableFunds {
                    recoverable_funds_sats: funds,
                })
                .await;
        }
    }

    /// Stores the spends in `recovered` and the sweeps in a block among
    /// `transactions`, so the SDK leaves their leaves out of the recoverable
    /// funds at the next sync.
    async fn record_finished(
        &self,
        transactions: &[RecoveryTransaction],
        recovered: Vec<(String, ChainTransaction)>,
    ) {
        let swept = swept_leaves(transactions);
        if recovered.is_empty() && swept.is_empty() {
            return;
        }
        let stored = match self.leaf_recoveries().await {
            Ok(stored) => stored,
            Err(e) => {
                error!("Failed to read the stored leaf recoveries: {e}");
                return;
            }
        };
        let now = now_secs();
        // The SDK stores a spend only for a leaf that has a stored recovery.
        let recovered = recovered
            .into_iter()
            .filter(|(leaf_id, spend)| {
                stored
                    .get(leaf_id)
                    .is_some_and(|leaf| leaf.watchtower_exit_spend.as_ref() != Some(spend))
            })
            .map(|(leaf_id, spend)| UpdateLeafRecovery {
                leaf_id,
                chain_checked_at: Some(now),
                watchtower_exit_spend: Some(spend),
                ..Default::default()
            });
        let swept = swept
            .into_iter()
            .filter(|(leaf_id, sweep)| {
                stored
                    .get(leaf_id)
                    .is_none_or(|leaf| leaf.unilateral_exit_sweep.as_ref() != Some(sweep))
            })
            .map(|(leaf_id, sweep)| UpdateLeafRecovery {
                leaf_id,
                chain_checked_at: Some(now),
                unilateral_exit_sweep: Some(sweep),
                ..Default::default()
            });
        for update in recovered.chain(swept).collect::<Vec<_>>() {
            self.store_leaf_recovery(update).await;
        }
    }

    async fn cosign_cooperative_recoveries(
        &self,
        exits: Vec<WatchtowerExit>,
        destination: &Address,
        fee_rate: u64,
        queries: &mut ChainQueries,
    ) -> CooperativeRecoveries {
        let mut recoveries = CooperativeRecoveries::default();
        let mut unreachable: Option<String> = None;
        for exit in exits {
            match self
                .cooperative_recovery(
                    &exit,
                    destination,
                    fee_rate,
                    unreachable.as_deref(),
                    queries,
                )
                .await
            {
                Ok(Some((tx, fee_sats))) => {
                    recoveries.fee_sats = recoveries
                        .fee_sats
                        .saturating_add(exit.exit_fee_sats())
                        .saturating_add(fee_sats);
                    recoveries.leaves.push(RecoverFundsLeaf {
                        leaf_id: exit.leaf_id.clone(),
                        value_sats: exit.value_sats,
                        method: RecoveryMethod::Cooperative,
                    });
                    recoveries.transactions.push(RecoveryTransaction {
                        kind: RecoveryTxKind::Cooperative,
                        node_id: Some(exit.leaf_id),
                        txid: tx.compute_txid().to_string(),
                        tx_hex: serialize_hex(&tx),
                        cpfp_tx_hex: None,
                        csv_timelock_blocks: None,
                        depends_on: Vec::new(),
                        status: ExitTransactionStatus::Ready,
                    });
                }
                Ok(None) => debug!(
                    leaf_id = exit.leaf_id,
                    "recover_funds: a recovery of the leaf is in a block"
                ),
                Err(error) => {
                    info!(
                        "Leaf {} was not recovered cooperatively: {error}",
                        exit.leaf_id
                    );
                    if let CooperativeRecoveryError::OperatorsUnavailable { message } = &error {
                        unreachable.get_or_insert_with(|| message.clone());
                    }
                    recoveries.failed.push(CooperativeRecoveryFailure {
                        leaf_id: exit.leaf_id,
                        output_txid: exit.output.outpoint.txid.to_string(),
                        output_vout: exit.output.outpoint.vout,
                        error,
                    });
                }
            }
        }
        recoveries
    }

    async fn watchtower_exit_candidates(&self) -> Result<Vec<TreeNode>, SdkError> {
        let statuses = [
            WATCHTOWER_EXITED_STATUSES.as_slice(),
            // Its direct tx can confirm at a fee its refunds do not spend.
            &[TreeNodeStatus::OnChain],
        ]
        .concat();
        Ok(self.spark_wallet.list_leaves_with_status(&statuses).await?)
    }

    /// The watchtower exit of each cooperative leaf in a quote. Fails when no
    /// output was found for one.
    async fn quoted_watchtower_exits(
        &self,
        leaves: &[&RecoverFundsLeaf],
        queries: &mut ChainQueries,
    ) -> Result<Vec<WatchtowerExit>, SdkError> {
        if leaves.is_empty() {
            return Ok(Vec::new());
        }
        let stored = self.leaf_recoveries().await?;
        let listed: Vec<TreeNode> = self
            .watchtower_exit_candidates()
            .await?
            .into_iter()
            .filter(|leaf| {
                let leaf_id = leaf.id.to_string();
                leaves.iter().any(|quoted| quoted.leaf_id == leaf_id)
            })
            .collect();
        let mut found = self
            .lookup_watchtower_exits(&listed, &stored, queries)
            .await?;
        let mut exits = Vec::with_capacity(leaves.len());
        for leaf in leaves {
            let exit = match found.exits.remove(&leaf.leaf_id) {
                Some(exit) => Some(exit),
                // A leaf the wallet no longer lists still has its stored output.
                None => stored
                    .get(&leaf.leaf_id)
                    .map(|stored| WatchtowerExit::from_stored(leaf.value_sats, stored))
                    .transpose()?
                    .flatten(),
            };
            exits.push(exit.ok_or_else(|| {
                SdkError::InvalidInput(format!(
                    "Leaf {} has no funds this wallet can recover cooperatively",
                    leaf.leaf_id
                ))
            })?);
        }
        Ok(exits)
    }

    async fn split_selection(
        &self,
        selection: ExitLeafSelection,
        stored: &HashMap<String, LeafRecovery>,
        queries: &mut ChainQueries,
    ) -> Result<RecoverySelection, SdkError> {
        let named: Option<HashSet<String>> = match &selection {
            ExitLeafSelection::Specific { leaf_ids } if leaf_ids.is_empty() => {
                return Err(SdkError::InvalidInput("No leaves to recover".to_string()));
            }
            ExitLeafSelection::Specific { leaf_ids } => Some(leaf_ids.iter().cloned().collect()),
            _ => None,
        };
        let selected: Vec<TreeNode> = self
            .watchtower_exit_candidates()
            .await?
            .into_iter()
            .filter(|leaf| {
                named
                    .as_ref()
                    .is_none_or(|named| named.contains(&leaf.id.to_string()))
            })
            .collect();
        let mut found = self
            .lookup_watchtower_exits(&selected, stored, queries)
            .await?;

        let mut not_unilateral: HashSet<String> = selected
            .iter()
            .filter(|leaf| leaf.status.is_watchtower_exited())
            .map(|leaf| leaf.id.to_string())
            .collect();
        not_unilateral.extend(found.exits.keys().cloned());
        let skipped = selected
            .iter()
            .filter_map(|leaf| {
                let leaf_id = leaf.id.to_string();
                skipped_cooperative_leaf(leaf, stored.get(&leaf_id), found.missing.get(&leaf_id))
            })
            .collect();
        let cooperative = selected
            .iter()
            .filter_map(|leaf| found.exits.remove(&leaf.id.to_string()))
            .filter(|exit| !exit.recovered)
            .collect();

        let unilateral = match selection {
            ExitLeafSelection::Specific { leaf_ids } => {
                let leaf_ids: Vec<String> = leaf_ids
                    .into_iter()
                    .filter(|leaf_id| !not_unilateral.contains(leaf_id))
                    .collect();
                if leaf_ids.is_empty() {
                    None
                } else {
                    Some(spark_wallet::ExitLeafSelection::Specific(node_ids(
                        &leaf_ids,
                    )?))
                }
            }
            ExitLeafSelection::All => Some(spark_wallet::ExitLeafSelection::Auto),
            ExitLeafSelection::RecoverableOnly => Some(spark_wallet::ExitLeafSelection::Exiting),
        };
        Ok(RecoverySelection {
            cooperative,
            skipped,
            unilateral,
            not_unilateral,
        })
    }
}

fn new_leaves(leaves: &[TreeNode], stored: &HashMap<String, LeafRecovery>) -> NewLeaves {
    let mut new = NewLeaves::default();
    for leaf in leaves {
        let leaf_id = leaf.id.to_string();
        if stored.contains_key(&leaf_id) {
            continue;
        }
        match leaf.status {
            // The SDK sends no chain request for funds too small to send out.
            _ if !can_be_sent_out(leaf) => new.to_recover.push(UpdateLeafRecovery {
                leaf_id,
                ..Default::default()
            }),
            TreeNodeStatus::WatchtowerExitRecovered => new.recovered.push(leaf.clone()),
            TreeNodeStatus::Exited => new.exited.push(leaf.clone()),
            _ => new.to_recover.push(UpdateLeafRecovery {
                leaf_id,
                ..Default::default()
            }),
        }
    }
    new
}

/// Whether the funds of a recovered or exited `leaf` can be sent out at the
/// lowest fee rate nodes relay. The leaf's value is the most its on-chain
/// output holds.
fn can_be_sent_out(leaf: &TreeNode) -> bool {
    let value = Amount::from_sat(leaf.value);
    let fee_rate = FeeRate::BROADCAST_MIN;
    match leaf.status {
        // A recovery is a transaction of its own. Of the outputs wallets
        // receive on, P2WPKH is the smallest and has the lowest dust limit.
        TreeNodeStatus::WatchtowerExitRecovered => {
            let output = ScriptBuf::new_p2wpkh(&WPubkeyHash::all_zeros());
            let fee = Fee::Rate {
                sat_per_vbyte: fee_rate.to_sat_per_vb_ceil(),
            };
            watchtower_exit_recovery_payout(value, &output, fee).is_some()
        }
        // One sweep has the refunds of several leaves as inputs, so a refund
        // only has to hold more than the fee of its own input.
        TreeNodeStatus::Exited => refund_sweep_input_fee(fee_rate).is_some_and(|fee| value > fee),
        _ => true,
    }
}

/// The value of the leaves whose stored recovery holds neither a watchtower exit
/// spend nor a sweep, and how many of them are among `new_leaf_ids`.
fn recoverable_funds(
    leaves: &[TreeNode],
    stored: &HashMap<String, LeafRecovery>,
    new_leaf_ids: &HashSet<String>,
) -> (u64, usize) {
    let mut funds: u64 = 0;
    let mut found: usize = 0;
    for leaf in leaves {
        let leaf_id = leaf.id.to_string();
        let unfinished = stored.get(&leaf_id).is_some_and(|leaf| {
            leaf.watchtower_exit_spend.is_none() && leaf.unilateral_exit_sweep.is_none()
        });
        if !unfinished {
            continue;
        }
        funds = funds.saturating_add(leaf.value);
        if new_leaf_ids.contains(&leaf_id) {
            found = found.saturating_add(1);
        }
    }
    (funds, found)
}

/// The sweep of each leaf that has one in a block among `transactions`. A
/// sweep lists the refunds it depends on, and each refund has its leaf's id.
fn swept_leaves(transactions: &[RecoveryTransaction]) -> Vec<(String, ChainTransaction)> {
    let mut swept = Vec::new();
    for sweep in transactions {
        let ExitTransactionStatus::Confirmed {
            block_height: Some(block_height),
        } = sweep.status
        else {
            continue;
        };
        if sweep.kind != RecoveryTxKind::Sweep {
            continue;
        }
        let leaf_ids = transactions
            .iter()
            .filter(|tx| tx.kind == RecoveryTxKind::Refund && sweep.depends_on.contains(&tx.txid))
            .filter_map(|tx| tx.node_id.clone());
        for leaf_id in leaf_ids {
            swept.push((
                leaf_id,
                ChainTransaction {
                    txid: sweep.txid.clone(),
                    block_height,
                },
            ));
        }
    }
    swept
}

/// The reason `prepare_recover_funds` gives for leaving out a watchtower-exited
/// `leaf` with no output to recover, from the `lookup` of that output. `None`
/// for a leaf with a recovery in a block, and for a leaf that is not
/// watchtower-exited.
fn skipped_cooperative_leaf(
    leaf: &TreeNode,
    stored: Option<&LeafRecovery>,
    lookup: Option<&WatchtowerExitLookup>,
) -> Option<SkippedLeaf> {
    let lookup = lookup?;
    if !leaf.status.is_watchtower_exited()
        || stored.is_some_and(|stored| stored.watchtower_exit_spend.is_some())
    {
        return None;
    }
    let reason = match lookup {
        WatchtowerExitLookup::Unrecoverable => SkippedLeafReason::NotRecoverable {
            message: "The transaction that took the leaf's funds on-chain pays none of them to \
                      the leaf's key"
                .to_string(),
        },
        WatchtowerExitLookup::Pending => SkippedLeafReason::Unverified,
        _ => SkippedLeafReason::FundsNotFound,
    };
    Some(SkippedLeaf {
        leaf_id: leaf.id.to_string(),
        value_sats: leaf.value,
        reason,
    })
}

fn total_value(leaves: &[RecoverFundsLeaf]) -> u64 {
    leaves
        .iter()
        .map(|leaf| leaf.value_sats)
        .fold(0, u64::saturating_add)
}

/// The output a cooperative recovery has as input.
fn recovered_output(tx: &RecoveryTransaction) -> Result<OutPoint, SdkError> {
    let decoded: Transaction = deserialize_hex(&tx.tx_hex)
        .map_err(|e| SdkError::InvalidInput(format!("Invalid transaction: {e}")))?;
    decoded
        .input
        .first()
        .map(|input| input.previous_output)
        .ok_or_else(|| {
            SdkError::InvalidInput(format!("Cooperative recovery {} spends nothing", tx.txid))
        })
}

/// The status of a cooperative recovery, from the `spend` of its input:
/// confirmed once any recovery of that output is in a block, this one or a
/// replacement. Without a result from the chain service it stays `status`.
fn cooperative_recovery_status(
    spend: &OutputSpend,
    status: ExitTransactionStatus,
) -> ExitTransactionStatus {
    match spend {
        OutputSpend::InBlock { block_height, .. } => ExitTransactionStatus::Confirmed {
            block_height: *block_height,
        },
        OutputSpend::Unspent | OutputSpend::InMempool(_) => ExitTransactionStatus::Ready,
        OutputSpend::Unknown => status,
    }
}

fn parse_destination(destination: &str, network: Network) -> Result<Address, SdkError> {
    destination
        .parse::<Address<NetworkUnchecked>>()
        .map_err(|e| SdkError::InvalidInput(format!("Invalid destination address: {e}")))?
        .require_network(network.into())
        .map_err(|e| SdkError::InvalidInput(format!("Address network mismatch: {e}")))
}

#[cfg(test)]
mod tests {
    use bitcoin::{Txid, hashes::Hash};
    use spark_wallet::tree_store_tests::create_test_node_with_parent;

    use crate::chain::stub::tx_paying;

    use super::*;

    const LEAF_ID: &str = "00000000-0000-0000-0000-00000000000a";
    const OTHER_LEAF_ID: &str = "00000000-0000-0000-0000-00000000000b";

    fn stored_leaf(leaf_id: &str) -> LeafRecovery {
        LeafRecovery {
            leaf_id: leaf_id.to_string(),
            chain_checked_at: None,
            watchtower_exit_output: None,
            watchtower_exit_recoveries: Vec::new(),
            watchtower_exit_spend: None,
            unilateral_exit_sweep: None,
        }
    }

    fn in_block(txid: &str) -> ChainTransaction {
        ChainTransaction {
            txid: txid.to_string(),
            block_height: 100,
        }
    }

    fn by_leaf_id(exits: Vec<LeafRecovery>) -> HashMap<String, LeafRecovery> {
        exits
            .into_iter()
            .map(|leaf| (leaf.leaf_id.clone(), leaf))
            .collect()
    }

    #[test]
    fn a_watchtower_exited_leaf_without_an_output_is_skipped() {
        let leaf = create_test_node_with_parent(LEAF_ID, None, TreeNodeStatus::WatchtowerExited);
        let skipped = |stored: Option<&LeafRecovery>, lookup: Option<&WatchtowerExitLookup>| {
            skipped_cooperative_leaf(&leaf, stored, lookup).map(|skipped| skipped.reason)
        };

        assert_eq!(
            skipped(None, Some(&WatchtowerExitLookup::NotFound)),
            Some(SkippedLeafReason::FundsNotFound)
        );
        assert_eq!(
            skipped(
                Some(&stored_leaf(LEAF_ID)),
                Some(&WatchtowerExitLookup::NotFound)
            ),
            Some(SkippedLeafReason::FundsNotFound)
        );
        assert_eq!(
            skipped(None, Some(&WatchtowerExitLookup::Pending)),
            Some(SkippedLeafReason::Unverified)
        );
        assert!(matches!(
            skipped(None, Some(&WatchtowerExitLookup::Unrecoverable)),
            Some(SkippedLeafReason::NotRecoverable { .. })
        ));
        assert_eq!(skipped(None, None), None, "an output was found");
        let recovered = LeafRecovery {
            watchtower_exit_spend: Some(in_block("recovery")),
            ..stored_leaf(LEAF_ID)
        };
        assert_eq!(
            skipped(Some(&recovered), Some(&WatchtowerExitLookup::NotFound)),
            None,
            "a finished recovery is not listed"
        );

        let on_chain = create_test_node_with_parent(LEAF_ID, None, TreeNodeStatus::OnChain);
        assert_eq!(
            skipped_cooperative_leaf(&on_chain, None, Some(&WatchtowerExitLookup::NotFound)),
            None,
            "its unilateral exit is quoted"
        );
    }

    #[test]
    fn a_new_leaf_is_stored_unless_its_status_leaves_its_funds_open() {
        let id = |n: u8| format!("00000000-0000-0000-0000-0000000000{n:02}");
        let leaves: Vec<TreeNode> = [
            TreeNodeStatus::WatchtowerExited,
            TreeNodeStatus::OnChain,
            TreeNodeStatus::ParentExited,
            TreeNodeStatus::WatchtowerExitRecovered,
            TreeNodeStatus::Exited,
            TreeNodeStatus::WatchtowerExitRecovered,
        ]
        .into_iter()
        .zip(1u8..)
        .map(|(status, n)| create_test_node_with_parent(&id(n), None, status))
        .collect();
        // The sixth leaf has a stored recovery.
        let stored = by_leaf_id(vec![stored_leaf(&id(6))]);

        let new = new_leaves(&leaves, &stored);

        let ids = |leaves: &[TreeNode]| -> Vec<String> {
            leaves.iter().map(|leaf| leaf.id.to_string()).collect()
        };
        assert_eq!(
            new.to_recover
                .iter()
                .map(|update| update.leaf_id.clone())
                .collect::<Vec<_>>(),
            vec![id(1), id(2), id(3)]
        );
        assert!(
            new.to_recover
                .iter()
                .all(|update| update.chain_checked_at.is_none())
        );
        assert_eq!(ids(&new.recovered), vec![id(4)]);
        assert_eq!(ids(&new.exited), vec![id(5)]);
    }

    #[test]
    fn a_leaf_too_small_to_send_out_is_stored_without_a_chain_check() {
        let leaf = |id: &str, status, value| {
            let mut leaf = create_test_node_with_parent(id, None, status);
            leaf.value = value;
            leaf
        };
        let recovered = |value| leaf(LEAF_ID, TreeNodeStatus::WatchtowerExitRecovered, value);
        let exited = |value| leaf(OTHER_LEAF_ID, TreeNodeStatus::Exited, value);

        // A recovery to a P2WPKH output is 99 vbytes, and the dust limit of
        // that output is 294 sats.
        assert!(can_be_sent_out(&recovered(393)));
        assert!(!can_be_sent_out(&recovered(392)));
        // A taproot key-path input is 57.5 vbytes.
        assert!(can_be_sent_out(&exited(59)));
        assert!(!can_be_sent_out(&exited(58)));
        // A leaf in another status gets no chain check to begin with.
        assert!(can_be_sent_out(&leaf(
            LEAF_ID,
            TreeNodeStatus::WatchtowerExited,
            1
        )));

        let new = new_leaves(&[recovered(392), exited(58)], &HashMap::new());
        assert!(new.recovered.is_empty() && new.exited.is_empty());
        assert_eq!(
            new.to_recover
                .iter()
                .map(|update| update.leaf_id.as_str())
                .collect::<Vec<_>>(),
            vec![LEAF_ID, OTHER_LEAF_ID]
        );
        let new = new_leaves(&[recovered(393), exited(59)], &HashMap::new());
        assert!(new.to_recover.is_empty());
        assert_eq!((new.recovered.len(), new.exited.len()), (1, 1));
    }

    #[test]
    fn the_funds_to_recover_are_those_of_stored_leaves_not_taken_out() {
        let leaf = |leaf_id: &str| {
            create_test_node_with_parent(leaf_id, None, TreeNodeStatus::WatchtowerExited)
        };
        let leaves = [leaf(LEAF_ID), leaf(OTHER_LEAF_ID)];
        let value = leaves[0].value;
        let new_leaf_ids = HashSet::from([LEAF_ID.to_string(), OTHER_LEAF_ID.to_string()]);
        let funds = |other: Option<LeafRecovery>, new_leaf_ids: &HashSet<String>| {
            let stored = by_leaf_id(std::iter::once(stored_leaf(LEAF_ID)).chain(other).collect());
            recoverable_funds(&leaves, &stored, new_leaf_ids)
        };

        assert_eq!(
            funds(Some(stored_leaf(OTHER_LEAF_ID)), &new_leaf_ids),
            (value.saturating_mul(2), 2)
        );
        assert_eq!(
            funds(Some(stored_leaf(OTHER_LEAF_ID)), &HashSet::new()),
            (value.saturating_mul(2), 0),
            "a leaf stored before is not found again"
        );
        assert_eq!(
            funds(None, &new_leaf_ids),
            (value, 1),
            "a leaf without a result from the chain service is left out"
        );
        let recovered = LeafRecovery {
            watchtower_exit_spend: Some(in_block("recovery")),
            ..stored_leaf(OTHER_LEAF_ID)
        };
        assert_eq!(funds(Some(recovered), &new_leaf_ids), (value, 1));
        let swept = LeafRecovery {
            unilateral_exit_sweep: Some(in_block("sweep")),
            ..stored_leaf(OTHER_LEAF_ID)
        };
        assert_eq!(funds(Some(swept), &new_leaf_ids), (value, 1));
    }

    fn unilateral(
        kind: RecoveryTxKind,
        txid: &str,
        node_id: Option<&str>,
        depends_on: &[&str],
        status: ExitTransactionStatus,
    ) -> RecoveryTransaction {
        RecoveryTransaction {
            kind,
            node_id: node_id.map(ToString::to_string),
            txid: txid.to_string(),
            tx_hex: String::new(),
            cpfp_tx_hex: None,
            csv_timelock_blocks: None,
            depends_on: depends_on.iter().map(ToString::to_string).collect(),
            status,
        }
    }

    #[test]
    fn a_sweep_in_a_block_swept_the_leaves_of_the_refunds_it_spends() {
        let confirmed = ExitTransactionStatus::Confirmed {
            block_height: Some(100),
        };
        let refunds = [
            unilateral(
                RecoveryTxKind::Refund,
                "refund_a",
                Some(LEAF_ID),
                &[],
                confirmed,
            ),
            unilateral(
                RecoveryTxKind::Refund,
                "refund_b",
                Some(OTHER_LEAF_ID),
                &[],
                confirmed,
            ),
        ];
        let with_sweep = |sweep: RecoveryTransaction| -> Vec<RecoveryTransaction> {
            refunds.iter().cloned().chain([sweep]).collect()
        };
        let sweep =
            |status| unilateral(RecoveryTxKind::Sweep, "sweep", None, &["refund_a"], status);

        assert_eq!(
            swept_leaves(&with_sweep(sweep(confirmed))),
            vec![(LEAF_ID.to_string(), in_block("sweep"))]
        );
        // Not in a block, or in one the chain service did not name.
        for status in [
            ExitTransactionStatus::Ready,
            ExitTransactionStatus::Confirmed { block_height: None },
        ] {
            assert!(swept_leaves(&with_sweep(sweep(status))).is_empty());
        }
        assert!(swept_leaves(&refunds).is_empty());
    }

    fn watchtower_output() -> OutPoint {
        OutPoint {
            txid: Txid::from_byte_array([7; 32]),
            vout: 1,
        }
    }

    #[test]
    fn a_cooperative_recovery_spends_the_output_of_its_first_input() {
        let tx = tx_paying(watchtower_output(), 9_500);
        let recovery = |tx_hex: String| RecoveryTransaction {
            kind: RecoveryTxKind::Cooperative,
            node_id: Some(LEAF_ID.to_string()),
            txid: tx.compute_txid().to_string(),
            tx_hex,
            cpfp_tx_hex: None,
            csv_timelock_blocks: None,
            depends_on: Vec::new(),
            status: ExitTransactionStatus::Ready,
        };

        assert_eq!(
            recovered_output(&recovery(serialize_hex(&tx))).unwrap(),
            watchtower_output()
        );
        assert!(recovered_output(&recovery("00".to_string())).is_err());
    }

    #[test]
    fn a_cooperative_recovery_is_confirmed_once_any_recovery_of_its_output_is() {
        let confirmed = ExitTransactionStatus::Confirmed {
            block_height: Some(120),
        };
        let in_block = OutputSpend::InBlock {
            txid: Txid::from_byte_array([8; 32]),
            block_height: Some(120),
        };

        assert_eq!(
            cooperative_recovery_status(&in_block, ExitTransactionStatus::Ready),
            confirmed
        );
        // Not in a block, whatever the status was before.
        for spend in [
            OutputSpend::Unspent,
            OutputSpend::InMempool(Txid::from_byte_array([8; 32])),
        ] {
            assert_eq!(
                cooperative_recovery_status(&spend, confirmed),
                ExitTransactionStatus::Ready
            );
        }
        // Without a result the status stays as it was.
        assert_eq!(
            cooperative_recovery_status(&OutputSpend::Unknown, confirmed),
            confirmed
        );
    }
}
