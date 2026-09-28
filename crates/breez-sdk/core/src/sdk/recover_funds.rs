use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use bitcoin::{
    Address, Transaction,
    address::NetworkUnchecked,
    consensus::encode::{deserialize_hex, serialize_hex},
};
use futures::StreamExt;
use spark_wallet::{
    EXITING_STATUSES, TreeNode, TreeNodeStatus, WATCHTOWER_EXITED_STATUSES, is_watchtower_exited,
};
use tracing::{debug, error, info, warn};

use crate::{
    CheckRecoverFundsRequest, CheckRecoverFundsResponse, CooperativeRecoveryError,
    CooperativeRecoveryFailure, ExitLeafSelection, ExitTransactionStatus, Network,
    PrepareRecoverFundsRequest, PrepareRecoverFundsResponse, RecoverFundsLeaf, RecoverFundsRequest,
    RecoverFundsResponse, RecoveryMethod, RecoveryRedoReason, RecoveryTransaction, RecoveryTxKind,
    RecoveryVerdict, SdkEvent,
    chain::{BitcoinChainService, Outspend},
    error::SdkError,
    persist::{CachedWatchtowerExit, ObjectCacheRepository},
    signer::CpfpSigner,
};

use super::{
    BreezSdk,
    unilateral_exit::{
        UnilateralBuild, UnilateralQuote, check_recovery_transactions, node_ids, wallet_selection,
    },
    watchtower_exit::build_recovery,
};

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Quotes a recovery of the selected leaves: how each is recovered, the exact
    /// fees, and how much to fund.
    pub async fn prepare_recover_funds(
        &self,
        request: PrepareRecoverFundsRequest,
    ) -> Result<PrepareRecoverFundsResponse, SdkError> {
        let fee_rate = request.fee_rate_sat_per_vbyte;
        let destination = parse_destination(&request.destination, self.config.network)?;
        self.spark_wallet.refresh_before_exit().await;
        let selection = self.split_selection(request.selection).await?;

        let mut leaves = Vec::new();
        let mut cooperative_fee_sats = 0u64;
        for exit in selection.cooperative {
            let Some(recovery) = build_recovery(&exit.output()?, &destination, fee_rate) else {
                debug!(
                    leaf_id = exit.leaf_id,
                    "prepare_recover_funds: too small to pay the fee"
                );
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
                )
                .await?
            }
            None => UnilateralQuote::default(),
        };
        leaves.extend(unilateral.leaves);

        let response = PrepareRecoverFundsResponse {
            leaves,
            recoverable_value_sats: cooperative_value_sats
                .saturating_add(unilateral.recoverable_value_sat),
            total_fee_sats: cooperative_fee_sats.saturating_add(unilateral.total_fee_sat),
            cooperative_fee_sats,
            cpfp_fee_sats: unilateral.cpfp_fee_sat,
            fanout_fee_sats: unilateral.fanout_fee_sat,
            sweep_fee_sats: unilateral.sweep_fee_sat,
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
        let exits = self.quoted_watchtower_exits(&cooperative).await?;

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
            )
            .await?
        };

        let cooperative = self
            .cosign_cooperative_recoveries(exits, &destination, fee_rate)
            .await;
        let mut leaves = cooperative.leaves;
        let recoverable_value_sats =
            total_value(&leaves).saturating_add(unilateral.recoverable_value_sat);
        leaves.extend(unilateral.leaves);
        let mut transactions = cooperative.transactions;
        transactions.extend(unilateral.transactions);
        let response = RecoverFundsResponse {
            recoverable_value_sats,
            total_fee_sats: cooperative.fee_sat.saturating_add(unilateral.total_fee_sat),
            cooperative_fee_sats: cooperative.fee_sat,
            cpfp_fee_sats: unilateral.cpfp_fee_sat,
            fanout_fee_sats: unilateral.fanout_fee_sat,
            sweep_fee_sats: unilateral.sweep_fee_sat,
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
        for tx in &mut cooperative {
            tx.status = cooperative_recovery_status(self.chain_service.as_ref(), tx).await?;
        }
        let diverged =
            check_recovery_transactions(self.chain_service.as_ref(), &mut unilateral).await?;
        recovery.transactions = cooperative.into_iter().chain(unilateral).collect();
        self.record_finished(&recovery.transactions).await;

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
    fee_sat: u64,
}

struct RecoverySelection {
    cooperative: Vec<CachedWatchtowerExit>,
    unilateral: Option<spark_wallet::ExitLeafSelection>,
    /// Leaves that no unilateral exit recovers.
    not_unilateral: HashSet<String>,
}

/// What the lookups of a quote found, kept for as long as the SDK runs.
#[derive(Default)]
pub(crate) struct RecoveryChecks {
    /// Leaves not looked up again: no cooperative recovery reaches them.
    pub(super) unrecoverable: HashSet<String>,
    /// Leaves not looked up again: their own refunds recover them.
    pub(super) unilateral: HashSet<String>,
}

/// How many leaves a sync checks against the chain at the same time.
const CONCURRENT_CHECKS: usize = 4;

/// Runs `checks` a few at a time and returns the ones that answered. Once a
/// check could not be made, the ones not started yet are left for the next
/// sync: a chain service that limits requests is not asked for more.
pub(super) async fn run_checks<T, F>(checks: Vec<F>) -> Vec<T>
where
    F: Future<Output = Option<T>>,
{
    let failed = Arc::new(AtomicBool::new(false));
    let started = failed.clone();
    futures::stream::iter(checks)
        .take_while(move |_| futures::future::ready(!started.load(Ordering::Relaxed)))
        .buffer_unordered(CONCURRENT_CHECKS)
        .filter_map(move |answer| {
            if answer.is_none() {
                failed.store(true, Ordering::Relaxed);
            }
            futures::future::ready(answer)
        })
        .collect()
        .await
}

impl BreezSdk {
    pub(crate) async fn recoverable_funds_sats(&self) -> Result<u64, SdkError> {
        Ok(ObjectCacheRepository::new(self.storage.clone())
            .fetch_recoverable_funds()
            .await?
            .unwrap_or_default())
    }

    /// Stores the total of the funds to recover, and emits `RecoverableFunds`
    /// when it found new ones.
    pub(super) async fn sync_recoverable_funds(&self) {
        let statuses = [EXITING_STATUSES.as_slice(), &WATCHTOWER_EXITED_STATUSES].concat();
        let leaves = match self.spark_wallet.list_leaves_with_status(&statuses).await {
            Ok(leaves) => leaves,
            Err(e) => {
                error!("Failed to list the leaves to recover: {e}");
                return;
            }
        };
        let (watchtower_exited, exiting): (Vec<TreeNode>, Vec<TreeNode>) = leaves
            .into_iter()
            .partition(|leaf| is_watchtower_exited(leaf.status));
        // A quote stores the output of an on-chain leaf it recovers
        // cooperatively.
        let cooperative: Vec<TreeNode> = watchtower_exited
            .into_iter()
            .chain(
                exiting
                    .iter()
                    .filter(|leaf| leaf.status == TreeNodeStatus::OnChain)
                    .cloned(),
            )
            .collect();
        let repository = ObjectCacheRepository::new(self.storage.clone());
        let stored = match repository.fetch_recoverable_funds().await {
            Ok(stored) => stored.unwrap_or_default(),
            Err(e) => {
                error!("Failed to read the recoverable funds: {e}");
                0
            }
        };
        let (cooperative_sats, found_cooperative, cooperative_leaves) =
            self.sync_watchtower_exits(&cooperative).await;
        let exiting: Vec<TreeNode> = exiting
            .into_iter()
            .filter(|leaf| !cooperative_leaves.contains(&leaf.id.to_string()))
            .collect();
        let (unilateral_sats, found_unilateral) = self.sync_exiting_leaves(&exiting).await;
        let funds = cooperative_sats.saturating_add(unilateral_sats);

        if funds != stored
            && let Err(e) = repository.save_recoverable_funds(funds).await
        {
            error!("Failed to store the recoverable funds: {e}");
        }
        if found_cooperative.saturating_add(found_unilateral) > 0 {
            self.event_emitter
                .emit(&SdkEvent::RecoverableFunds {
                    recoverable_funds_sats: funds,
                })
                .await;
        }
    }

    /// Records the leaves whose recovery these transactions finished, so the
    /// next sync leaves them out of the recoverable funds.
    async fn record_finished(&self, transactions: &[RecoveryTransaction]) {
        let confirmed = |kind: RecoveryTxKind| {
            transactions.iter().filter(move |tx| {
                tx.kind == kind && matches!(tx.status, ExitTransactionStatus::Confirmed { .. })
            })
        };
        let recovered: Vec<&str> = confirmed(RecoveryTxKind::Cooperative)
            .filter_map(|tx| tx.node_id.as_deref())
            .collect();
        // A sweep names the refunds it spends, and each refund names its leaf.
        let swept_refunds: HashSet<&str> = confirmed(RecoveryTxKind::Sweep)
            .flat_map(|tx| tx.depends_on.iter().map(String::as_str))
            .collect();
        let swept: Vec<&str> = transactions
            .iter()
            .filter(|tx| {
                tx.kind == RecoveryTxKind::Refund && swept_refunds.contains(tx.txid.as_str())
            })
            .filter_map(|tx| tx.node_id.as_deref())
            .collect();
        if recovered.is_empty() && swept.is_empty() {
            return;
        }
        let tip = self.chain_service.tip_height().await.ok();
        self.record_recovered(&recovered, tip).await;
        self.record_swept(&swept, tip).await;
    }

    /// The chain tip a round of checks is recorded with. Unreadable when the
    /// chain service is, and then no check is started.
    pub(super) async fn tip_for_checks(&self) -> Option<u32> {
        match self.chain_service.tip_height().await {
            Ok(tip) => Some(tip),
            Err(e) => {
                warn!("Failed to read the chain tip, leaving the checks for the next sync: {e}");
                None
            }
        }
    }

    async fn cosign_cooperative_recoveries(
        &self,
        exits: Vec<CachedWatchtowerExit>,
        destination: &Address,
        fee_rate: u64,
    ) -> CooperativeRecoveries {
        let mut recoveries = CooperativeRecoveries::default();
        let mut unreachable: Option<String> = None;
        for exit in exits {
            match self
                .cooperative_recovery(&exit, destination, fee_rate, unreachable.as_deref())
                .await
            {
                Ok(Some((tx, fee_sat))) => {
                    recoveries.fee_sat = recoveries
                        .fee_sat
                        .saturating_add(exit.exit_fee_sats())
                        .saturating_add(fee_sat);
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
                    "recover_funds: a recovery of the leaf already confirmed"
                ),
                Err(error) => {
                    info!(
                        "Leaf {} was not recovered cooperatively: {error}",
                        exit.leaf_id
                    );
                    if let CooperativeRecoveryError::OperatorsUnavailable { message } = &error {
                        unreachable.get_or_insert_with(|| message.clone());
                    }
                    let (output_txid, output_vout) = exit
                        .output
                        .map(|output| (output.txid, output.vout))
                        .unwrap_or_default();
                    recoveries.failed.push(CooperativeRecoveryFailure {
                        leaf_id: exit.leaf_id,
                        output_txid,
                        output_vout,
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
    ) -> Result<Vec<CachedWatchtowerExit>, SdkError> {
        if leaves.is_empty() {
            return Ok(Vec::new());
        }
        let stored: Vec<TreeNode> = self
            .watchtower_exit_candidates()
            .await?
            .into_iter()
            .filter(|leaf| {
                let leaf_id = leaf.id.to_string();
                leaves.iter().any(|quoted| quoted.leaf_id == leaf_id)
            })
            .collect();
        let tip = self.chain_service.tip_height().await.ok();
        let mut looked_up: HashMap<String, CachedWatchtowerExit> = self
            .lookup_watchtower_exits(&stored, tip)
            .await?
            .into_iter()
            .map(|exit| (exit.leaf_id.clone(), exit))
            .collect();
        let mut exits = Vec::with_capacity(leaves.len());
        for leaf in leaves {
            let exit = match looked_up.remove(&leaf.leaf_id) {
                Some(exit) => Some(exit),
                None => self.fetch_watchtower_exit(&leaf.leaf_id).await?,
            };
            exits.push(exit.filter(|exit| exit.output.is_some()).ok_or_else(|| {
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
        let tip = self.chain_service.tip_height().await.ok();
        let exits = self.lookup_watchtower_exits(&selected, tip).await?;

        let mut not_unilateral: HashSet<String> = selected
            .iter()
            .filter(|leaf| is_watchtower_exited(leaf.status))
            .map(|leaf| leaf.id.to_string())
            .collect();
        not_unilateral.extend(
            exits
                .iter()
                .filter(|exit| exit.output.is_some())
                .map(|exit| exit.leaf_id.clone()),
        );
        let cooperative = exits
            .into_iter()
            .filter(|exit| exit.output.is_some() && exit.recovered.is_none())
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
            selection => Some(wallet_selection(selection)?),
        };
        Ok(RecoverySelection {
            cooperative,
            unilateral,
            not_unilateral,
        })
    }
}

fn total_value(leaves: &[RecoverFundsLeaf]) -> u64 {
    leaves
        .iter()
        .map(|leaf| leaf.value_sats)
        .fold(0, u64::saturating_add)
}

/// Confirmed once any recovery of the output it spends confirmed: this one, or
/// one that replaced it.
async fn cooperative_recovery_status(
    chain: &dyn BitcoinChainService,
    tx: &RecoveryTransaction,
) -> Result<ExitTransactionStatus, SdkError> {
    let decoded: Transaction = deserialize_hex(&tx.tx_hex)
        .map_err(|e| SdkError::InvalidInput(format!("Invalid transaction: {e}")))?;
    let Some(input) = decoded.input.first() else {
        return Err(SdkError::InvalidInput(format!(
            "Cooperative recovery {} spends nothing",
            tx.txid
        )));
    };
    let outpoint = input.previous_output;
    Ok(
        match chain
            .get_outspend(outpoint.txid.to_string(), outpoint.vout)
            .await
        {
            Ok(Outspend::Spent { status, .. }) if status.confirmed => {
                ExitTransactionStatus::Confirmed {
                    block_height: status.block_height,
                }
            }
            Ok(_) => ExitTransactionStatus::Ready,
            Err(e) => {
                warn!("Failed to read cooperative recovery {}: {e}", tx.txid);
                tx.status
            }
        },
    )
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
    use bitcoin::{OutPoint, hashes::Hash};

    use crate::chain::stub::{ChainStub, tx_paying};

    use super::*;

    fn cooperative_spending(
        outpoint: OutPoint,
        status: ExitTransactionStatus,
    ) -> RecoveryTransaction {
        let tx = tx_paying(outpoint, 9_500);
        RecoveryTransaction {
            kind: RecoveryTxKind::Cooperative,
            node_id: Some("leaf".to_string()),
            txid: tx.compute_txid().to_string(),
            tx_hex: serialize_hex(&tx),
            cpfp_tx_hex: None,
            csv_timelock_blocks: None,
            depends_on: Vec::new(),
            status,
        }
    }

    fn watchtower_output() -> OutPoint {
        OutPoint {
            txid: bitcoin::Txid::from_byte_array([7; 32]),
            vout: 1,
        }
    }

    fn chain_with(outspend: Outspend) -> ChainStub {
        ChainStub::spending(watchtower_output(), outspend)
    }

    #[macros::async_test_all]
    async fn a_cooperative_recovery_is_confirmed_once_any_recovery_of_its_output_is() {
        let ours = cooperative_spending(watchtower_output(), ExitTransactionStatus::Ready);
        let chain = chain_with(ChainStub::spent(&ours.txid, true, Some(120)));
        assert_eq!(
            cooperative_recovery_status(&chain, &ours).await.unwrap(),
            ExitTransactionStatus::Confirmed {
                block_height: Some(120)
            }
        );

        let replaced = chain_with(ChainStub::spent("replacement", true, Some(121)));
        assert_eq!(
            cooperative_recovery_status(&replaced, &ours).await.unwrap(),
            ExitTransactionStatus::Confirmed {
                block_height: Some(121)
            }
        );
    }

    #[macros::async_test_all]
    async fn a_cooperative_recovery_not_in_a_block_is_ready() {
        let ours = cooperative_spending(
            watchtower_output(),
            ExitTransactionStatus::Confirmed {
                block_height: Some(120),
            },
        );

        let in_mempool = chain_with(ChainStub::spent(&ours.txid, false, None));
        assert_eq!(
            cooperative_recovery_status(&in_mempool, &ours)
                .await
                .unwrap(),
            ExitTransactionStatus::Ready
        );
        let unspent = chain_with(Outspend::Unspent);
        assert_eq!(
            cooperative_recovery_status(&unspent, &ours).await.unwrap(),
            ExitTransactionStatus::Ready
        );
    }

    #[macros::async_test_all]
    async fn an_unreadable_chain_keeps_the_status() {
        let confirmed = ExitTransactionStatus::Confirmed {
            block_height: Some(120),
        };
        let ours = cooperative_spending(watchtower_output(), confirmed);

        let status = cooperative_recovery_status(&ChainStub::default(), &ours).await;

        assert_eq!(status.unwrap(), confirmed);
    }
}

#[cfg(test)]
mod run_checks_tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    /// Completes the second time it is polled, so checks overlap.
    async fn yield_once() {
        let mut yielded = false;
        std::future::poll_fn(|cx| {
            if yielded {
                return std::task::Poll::Ready(());
            }
            yielded = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        })
        .await;
    }

    #[macros::async_test_all]
    async fn every_answer_is_returned() {
        let checks = (0..10u32).map(|i| async move { Some(i) }).collect();

        let mut answers: Vec<u32> = run_checks(checks).await;

        answers.sort_unstable();
        assert_eq!(answers, (0..10).collect::<Vec<_>>());
    }

    #[macros::async_test_all]
    async fn no_more_than_the_cap_run_at_once() {
        let running = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let checks = (0..20u32)
            .map(|i| {
                let running = running.clone();
                let most = most.clone();
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst).saturating_add(1);
                    most.fetch_max(now, Ordering::SeqCst);
                    yield_once().await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    Some(i)
                }
            })
            .collect();

        let answers: Vec<u32> = run_checks(checks).await;

        assert_eq!(answers.len(), 20);
        assert_eq!(most.load(Ordering::SeqCst), CONCURRENT_CHECKS);
    }

    #[macros::async_test_all]
    async fn an_unanswered_check_leaves_the_rest_unstarted() {
        let started = Arc::new(AtomicUsize::new(0));
        let checks = (0..20u32)
            .map(|i| {
                let started = started.clone();
                async move {
                    started.fetch_add(1, Ordering::SeqCst);
                    yield_once().await;
                    (i != 0).then_some(i)
                }
            })
            .collect();

        let answers: Vec<u32> = run_checks(checks).await;

        // The checks already running finish, and no other check starts.
        assert_eq!(started.load(Ordering::SeqCst), CONCURRENT_CHECKS);
        assert_eq!(answers.len(), CONCURRENT_CHECKS.saturating_sub(1));
        assert!(!answers.contains(&0));
    }
}
