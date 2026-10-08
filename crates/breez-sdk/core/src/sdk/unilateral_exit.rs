use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, ScriptBuf, Transaction, TxOut, Txid,
    XOnlyPublicKey,
    consensus::encode::{deserialize_hex, serialize_hex},
    secp256k1::PublicKey,
};

use spark_wallet::{
    ConfirmedExitNode as WalletConfirmedExitNode, CpfpInput,
    ExitChainState as WalletExitChainState, ExitCheck, ExitCheckInput,
    ExitNodeConfirmation as WalletExitNodeConfirmation, ExitRefund as WalletExitRefund,
    ExitRefundState as WalletExitRefundState, ExitTxKind, ExitTxStatus, Observation, RefundSweep,
    TreeNode, TreeNodeId, UnilateralExitBuild, UnilateralExitSkipReason, UnilateralExitSkippedLeaf,
    build_unilateral_exit, check_exit_chain, is_ephemeral_anchor_output, leaf_refund_addresses,
    scan_exit_chain, scan_funding,
};

use tracing::{debug, trace, warn};

use crate::{
    ChainTransaction, LeafRecovery, UpdateLeafRecovery,
    chain::BitcoinChainService,
    error::SdkError,
    models::{
        ConfirmedExitNode, CpfpFundingKind, CpfpInput as ModelCpfpInput,
        ExitChainState as ModelExitChainState, ExitNodeConfirmation, ExitRefund, ExitRefundState,
        ExitTransactionStatus, PerBranchFunding, RecoverFundsLeaf, RecoveryFunding, RecoveryMethod,
        RecoveryTransaction, RecoveryTxKind, SkippedLeaf, SkippedLeafReason,
    },
    signer::CpfpSigner,
    utils::time::now_secs,
};

use super::{BreezSdk, chain_queries::ChainQueries};

#[derive(Default)]
pub(super) struct UnilateralQuote {
    pub(super) leaves: Vec<RecoverFundsLeaf>,
    pub(super) skipped: Vec<SkippedLeaf>,
    pub(super) recoverable_value_sats: u64,
    pub(super) total_fee_sats: u64,
    pub(super) cpfp_fee_sats: u64,
    pub(super) fanout_fee_sats: u64,
    pub(super) sweep_fee_sats: u64,
    pub(super) funding: Option<RecoveryFunding>,
    pub(super) exit_chain_state: ModelExitChainState,
}

#[derive(Default)]
pub(super) struct UnilateralBuild {
    pub(super) leaves: Vec<RecoverFundsLeaf>,
    pub(super) recoverable_value_sats: u64,
    pub(super) total_fee_sats: u64,
    pub(super) cpfp_fee_sats: u64,
    pub(super) fanout_fee_sats: u64,
    pub(super) sweep_fee_sats: u64,
    pub(super) transactions: Vec<RecoveryTransaction>,
}

impl BreezSdk {
    /// Quotes a unilateral exit of the selected leaves: which would exit, the
    /// exact fee for the funding kind, and how much to fund. A leaf whose exit
    /// already finished is left out, and recorded as finished.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn quote_unilateral_exit(
        &self,
        destination: &Address,
        fee_rate_sat_per_vbyte: u64,
        funding_kind: Option<&CpfpFundingKind>,
        selection: spark_wallet::ExitLeafSelection,
        not_unilateral: &HashSet<String>,
        stored: &HashMap<String, LeafRecovery>,
        queries: &mut ChainQueries,
    ) -> Result<UnilateralQuote, SdkError> {
        let dest_script_len = destination.script_pubkey().len();
        // Without a funding kind no CPFP child is quoted, so these are not used.
        let (input_weight, output_script) = match funding_kind {
            Some(kind) => funding_kind_params(kind)?,
            None => (0, ScriptBuf::new()),
        };
        let mut context = self
            .spark_wallet
            .load_selected_exit_context(selection)
            .await?;
        // The chain walk below drops them as well, but only while the chain can
        // be read.
        context
            .leaf_ids
            .retain(|leaf_id| !not_unilateral.contains(&leaf_id.to_string()));

        // Ask the chain what these leaves have already done before pricing them,
        // so a leaf part-way out is quoted on the work it has left rather than on
        // a fresh exit, and is not dropped as unprofitable over work already
        // paid for. Every lookup the tree needs happens here.
        let refund_addresses = leaf_refund_addresses(
            &context.tree_nodes,
            &context.leaf_ids,
            self.config.network.into(),
        );
        let (exit_chain_state, sweeps) = resolve_exit_chain_state(
            queries,
            &context.tree_nodes,
            &context.leaf_ids,
            &refund_addresses,
        )
        .await;
        self.store_sweeps(&sweeps, stored).await;
        // A leaf whose exit already finished is not exited again, whether named or
        // not; `exit_chain_state` still reports it as swept or stopped.
        let before = context.leaf_ids.clone();
        context.drop_finished_leaves(&exit_chain_state);
        let mut skipped = skipped_dropped_leaves(&before, &context, &exit_chain_state, stored);

        let quote = self.spark_wallet.quote_unilateral_exit(
            &context,
            sat_per_kw_from_vbyte(fee_rate_sat_per_vbyte),
            input_weight,
            output_script.len(),
            output_script.minimal_non_dust().to_sat(),
            dest_script_len,
            &exit_chain_state,
        )?;
        skipped.extend(quote.skipped_leaves.iter().map(skipped_by_planner));
        // No selected leaves is not an error: return an empty quote.
        let recoverable_value_sats = quote
            .selected_leaves
            .iter()
            .map(|l| l.value)
            .fold(0u64, u64::saturating_add);
        let leaves = quote
            .selected_leaves
            .iter()
            .map(|l| RecoverFundsLeaf {
                leaf_id: l.id.to_string(),
                value_sats: l.value,
                method: RecoveryMethod::Unilateral,
            })
            .collect();
        let per_branch: Vec<PerBranchFunding> = quote
            .per_branch_funding
            .into_iter()
            .map(|(id, funding_sats)| PerBranchFunding {
                leaf_id: id.to_string(),
                funding_sats,
            })
            .collect();
        if funding_kind.is_none() && !per_branch.is_empty() {
            return Err(SdkError::InvalidInput(
                "A funding kind is needed: the selection holds a unilateral exit with steps \
                 left to broadcast"
                    .to_string(),
            ));
        }

        debug!(
            selected_leaves = quote.selected_leaves.len(),
            recoverable_value_sats,
            total_fee_sat = quote.total_fee_sat,
            cpfp_fee_sat = quote.cpfp_fee_sat,
            fanout_fee_sat = quote.fanout_fee_sat,
            sweep_fee_sat = quote.sweep_fee_sat,
            single_utxo_funding_sat = quote.single_utxo_funding_sat,
            branches = per_branch.len(),
            "quote_unilateral_exit: quote ready"
        );

        Ok(UnilateralQuote {
            leaves,
            skipped,
            recoverable_value_sats,
            total_fee_sats: quote.total_fee_sat,
            cpfp_fee_sats: quote.cpfp_fee_sat,
            fanout_fee_sats: quote.fanout_fee_sat,
            sweep_fee_sats: quote.sweep_fee_sat,
            funding: (!per_branch.is_empty()).then_some(RecoveryFunding {
                single_utxo_sats: quote.single_utxo_funding_sat,
                per_branch,
            }),
            exit_chain_state: exit_chain_state_model(exit_chain_state),
        })
    }

    /// Builds and signs a unilateral exit of exactly `leaf_ids` from the actual
    /// funding UTXOs, in topological broadcast order, without broadcasting.
    ///
    /// It reads on-chain state first: an already-confirmed fan-out or CPFP node
    /// is not rebuilt, and a leaf refund already on-chain (recognized by the
    /// leaf's refund address, so any refund variant counts) is swept directly.
    /// Re-running after partial progress therefore resumes rather than restarts.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(super) async fn build_unilateral_exit(
        &self,
        destination: &Address,
        leaf_ids: Vec<TreeNodeId>,
        fee_rate_sat_per_vbyte: u64,
        funding_inputs: Vec<ModelCpfpInput>,
        single_utxo_funding_sats: u64,
        exit_chain_state: &ModelExitChainState,
        signer: Option<&dyn CpfpSigner>,
        queries: &mut ChainQueries,
    ) -> Result<UnilateralBuild, SdkError> {
        let btc_network: bitcoin::Network = self.config.network.into();
        let chain = self.chain_service.as_ref();
        let dest_script_len = destination.script_pubkey().len();

        let supplied = funding_inputs
            .into_iter()
            .map(|i| i.into_funding_input(btc_network))
            .collect::<Result<Vec<_>, SdkError>>()?;
        let supplied_any = !supplied.is_empty();

        // A prior run spends the funding it was given, so what the caller hands
        // back may name outputs that are gone. Follow them to what they became
        // before planning, so the plan is made over what can actually be spent.
        let funding_inputs = resolve_funding(queries, supplied).await;
        // Followed to nothing: a previous run spent all of it. That is a
        // shortfall, not a malformed request.
        if supplied_any && funding_inputs.is_empty() {
            return Err(SdkError::InsufficientCpfpFunds {
                required_sats: single_utxo_funding_sats,
            });
        }
        let fee_rate_sat_per_kw = sat_per_kw_from_vbyte(fee_rate_sat_per_vbyte);
        let chain_state = exit_chain_state_from_model(exit_chain_state)?;
        let context = self
            .spark_wallet
            .load_exit_context(spark_wallet::ExitLeafSelection::Specific(leaf_ids))
            .await?;
        let prepared_exit = self.spark_wallet.prepare_unilateral_exit_plan(
            &context,
            fee_rate_sat_per_kw,
            funding_inputs,
            dest_script_len,
            &chain_state,
        )?;
        if prepared_exit.selected_leaves.is_empty() {
            debug!("build_unilateral_exit: plan selected no leaves, returning empty result");
            return Ok(UnilateralBuild::default());
        }
        trace!(
            selected_leaves = prepared_exit.selected_leaves.len(),
            tree_nodes = prepared_exit.tree_nodes.len(),
            has_fan_out = prepared_exit.fan_out_psbt.is_some(),
            "build_unilateral_exit: plan prepared"
        );

        let leaves: Vec<RecoverFundsLeaf> = prepared_exit
            .selected_leaves
            .iter()
            .map(|l| RecoverFundsLeaf {
                leaf_id: l.id.to_string(),
                value_sats: l.value,
                method: RecoveryMethod::Unilateral,
            })
            .collect();

        let build = build_unilateral_exit(&prepared_exit, &chain_state, fee_rate_sat_per_kw)?;
        let recoverable_value_sats = build.recoverable_value_sat;
        let cpfp_fee_sats = build.cpfp_fee_sat;
        let fanout_fee_sats = build.fanout_fee_sat;
        let build_fee_sats = cpfp_fee_sats.saturating_add(fanout_fee_sats);
        // Captured before the loop below consumes `build.branches`.
        let sweep_status = sweep_initial_status(&build);
        debug!(
            has_fan_out = build.fan_out.is_some(),
            branches = build.branches.len(),
            refund_outputs = build.refund_outputs.len(),
            cpfp_change_inputs = build.cpfp_change_inputs.len(),
            recoverable_value_sats,
            cpfp_fee_sats,
            fanout_fee_sats,
            "build_unilateral_exit: build assembled, signing"
        );

        let mut transactions: Vec<RecoveryTransaction> = Vec::new();

        if let Some(fan_out) = build.fan_out {
            trace!(
                txid = %fan_out.txid,
                status = ?fan_out.status,
                needs_signing = fan_out.to_sign.is_some(),
                "build_unilateral_exit: fan-out"
            );
            let tx_hex = match fan_out.to_sign {
                Some(psbt) => sign_psbt_via(psbt, signer).await?,
                None => serialize_hex(&fan_out.base_tx),
            };
            transactions.push(RecoveryTransaction {
                kind: RecoveryTxKind::FanOut,
                node_id: None,
                txid: fan_out.txid.to_string(),
                tx_hex,
                cpfp_tx_hex: None,
                csv_timelock_blocks: fan_out.csv_timelock_blocks,
                depends_on: fan_out.depends_on.iter().map(ToString::to_string).collect(),
                status: initial_status(fan_out.status),
            });
        }

        for branch in build.branches {
            trace!(leaf_id = %branch.leaf_id, txs = branch.txs.len(), "build_unilateral_exit: branch");
            for tx in branch.txs {
                let kind = match tx.kind {
                    ExitTxKind::Node => RecoveryTxKind::Node,
                    ExitTxKind::Refund => RecoveryTxKind::Refund,
                    // The fan-out is emitted above, never inside a branch.
                    ExitTxKind::FanOut => continue,
                };
                trace!(
                    ?kind,
                    node_id = ?tx.node_id.as_ref().map(ToString::to_string),
                    txid = %tx.txid,
                    status = ?tx.status,
                    needs_cpfp_child = tx.to_sign.is_some(),
                    csv_timelock_blocks = ?tx.csv_timelock_blocks,
                    depends_on = tx.depends_on.len(),
                    "build_unilateral_exit: exit tx"
                );
                let cpfp_tx_hex = match tx.to_sign {
                    Some(child) => Some(sign_psbt_via(child, signer).await?),
                    None => None,
                };
                transactions.push(RecoveryTransaction {
                    kind,
                    node_id: tx.node_id.map(|id| id.to_string()),
                    txid: tx.txid.to_string(),
                    tx_hex: serialize_hex(&tx.base_tx),
                    cpfp_tx_hex,
                    csv_timelock_blocks: tx.csv_timelock_blocks,
                    depends_on: tx.depends_on.iter().map(ToString::to_string).collect(),
                    status: initial_status(tx.status),
                });
            }
        }

        // A sweep over zero inputs would error: return without one when no refund
        // is on-chain yet. A later run sweeps any refund that surfaces.
        if build.refund_outputs.is_empty() {
            resolve_statuses(chain, &mut transactions).await?;
            debug!("build_unilateral_exit: no refund outputs to sweep, omitting the sweep");
            return Ok(UnilateralBuild {
                leaves,
                recoverable_value_sats,
                total_fee_sats: build_fee_sats,
                cpfp_fee_sats,
                fanout_fee_sats,
                sweep_fee_sats: 0,
                transactions,
            });
        }

        let refund_txids: Vec<String> = build
            .refund_outputs
            .iter()
            .map(|r| r.outpoint.txid.to_string())
            .collect();
        let sweep_psbt = self
            .spark_wallet
            .create_refund_sweep_transaction(
                build.refund_outputs,
                build.cpfp_change_inputs,
                destination.clone(),
                fee_rate_sat_per_kw,
            )
            .await?;
        let sweep_fee_sats = sweep_fee(&sweep_psbt);
        let total_fee_sats = build_fee_sats.saturating_add(sweep_fee_sats);
        let sweep_txid = sweep_psbt.unsigned_tx.compute_txid();
        trace!(
            txid = %sweep_txid,
            status = ?sweep_status,
            refund_inputs = refund_txids.len(),
            "build_unilateral_exit: sweep"
        );
        let sweep_tx_hex = finalize_sweep(sweep_psbt, signer).await?;
        transactions.push(RecoveryTransaction {
            kind: RecoveryTxKind::Sweep,
            node_id: None,
            txid: sweep_txid.to_string(),
            tx_hex: sweep_tx_hex,
            cpfp_tx_hex: None,
            csv_timelock_blocks: None,
            depends_on: refund_txids,
            status: sweep_status,
        });

        resolve_statuses(chain, &mut transactions).await?;
        debug!(
            transactions = transactions.len(),
            recoverable_value_sats,
            total_fee_sats,
            sweep_fee_sats,
            "build_unilateral_exit: complete"
        );
        Ok(UnilateralBuild {
            leaves,
            recoverable_value_sats,
            total_fee_sats,
            cpfp_fee_sats,
            fanout_fee_sats,
            sweep_fee_sats,
            transactions,
        })
    }

    /// Checks with the chain service whether the refund of each exited leaf was
    /// swept. The operators report an exited leaf the same either way. Returns
    /// what to store for the leaves it has every result for.
    pub(super) async fn check_exited_leaves(
        &self,
        leaves: &[TreeNode],
        queries: &mut ChainQueries,
    ) -> Vec<UpdateLeafRecovery> {
        if leaves.is_empty() {
            return Vec::new();
        }
        let leaf_ids: Vec<TreeNodeId> = leaves.iter().map(|leaf| leaf.id.clone()).collect();
        let selection = spark_wallet::ExitLeafSelection::Specific(leaf_ids);
        let context = match self
            .spark_wallet
            .load_selected_exit_context(selection)
            .await
        {
            Ok(context) => context,
            Err(e) => {
                warn!("Failed to load the exit chains of exited leaves: {e}");
                return Vec::new();
            }
        };
        let refund_addresses = leaf_refund_addresses(
            &context.tree_nodes,
            &context.leaf_ids,
            self.config.network.into(),
        );
        queries
            .resolve(|observed| {
                let scan = scan_exit_chain(
                    &context.tree_nodes,
                    &context.leaf_ids,
                    &refund_addresses,
                    observed,
                );
                ((), scan.pending)
            })
            .await;
        exited_leaf_checks(
            &context.tree_nodes,
            &context.leaf_ids,
            &refund_addresses,
            &queries.fetched(),
            now_secs(),
        )
    }

    /// Stores the `sweeps` that `stored` does not hold.
    async fn store_sweeps(&self, sweeps: &[RefundSweep], stored: &HashMap<String, LeafRecovery>) {
        let now = now_secs();
        for sweep in sweeps {
            let Some(transaction) = sweep_in_block(sweep) else {
                continue;
            };
            let leaf_id = sweep.leaf_id.to_string();
            let held = stored
                .get(&leaf_id)
                .is_some_and(|leaf| leaf.unilateral_exit_sweep.as_ref() == Some(&transaction));
            if held {
                continue;
            }
            self.store_leaf_recovery(UpdateLeafRecovery {
                leaf_id,
                chain_checked_at: Some(now),
                unilateral_exit_sweep: Some(transaction),
                ..Default::default()
            })
            .await;
        }
    }
}

/// What to store for each exited leaf, from the queries with a result: nothing
/// for a leaf with a query still pending, and a check without a sweep for a
/// leaf the results show no sweep for. The funds of such a leaf stay in the
/// total until the chain service shows a sweep of them in a block.
fn exited_leaf_checks(
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    refund_addresses: &HashMap<TreeNodeId, Address>,
    fetched: &[Observation],
    now: u64,
) -> Vec<UpdateLeafRecovery> {
    let mut checks = Vec::new();
    for leaf_id in leaf_ids {
        // `scan_exit_chain` queries every address it is given, so it gets this
        // leaf's alone.
        let refund_address: HashMap<TreeNodeId, Address> = refund_addresses
            .get(leaf_id)
            .map(|address| (leaf_id.clone(), address.clone()))
            .into_iter()
            .collect();
        let scan = scan_exit_chain(
            tree_nodes,
            std::slice::from_ref(leaf_id),
            &refund_address,
            fetched,
        );
        if !scan.pending.is_empty() {
            continue;
        }
        let verified = scan.state.unverified_nodes.is_empty()
            && scan.state.unverifiable_confirmed_nodes.is_empty();
        let sweep = if verified && is_swept(&scan.state, leaf_id) {
            let Some(sweep) = scan.sweeps.first().and_then(sweep_in_block) else {
                continue;
            };
            Some(sweep)
        } else {
            None
        };
        checks.push(UpdateLeafRecovery {
            leaf_id: leaf_id.to_string(),
            chain_checked_at: Some(now),
            unilateral_exit_sweep: sweep,
            ..Default::default()
        });
    }
    checks
}

/// `None` when the chain service did not report the sweep's block.
fn sweep_in_block(sweep: &RefundSweep) -> Option<ChainTransaction> {
    sweep.block_height.map(|block_height| ChainTransaction {
        txid: sweep.txid.to_string(),
        block_height,
    })
}

/// The leaves of `before` that `context` dropped although their exit did not
/// finish in a sweep.
fn skipped_dropped_leaves(
    before: &[TreeNodeId],
    context: &spark_wallet::ExitContext,
    state: &WalletExitChainState,
    stored: &HashMap<String, LeafRecovery>,
) -> Vec<SkippedLeaf> {
    let mut skipped = Vec::new();
    for leaf_id in before {
        if context.leaf_ids.contains(leaf_id) {
            continue;
        }
        let Some(leaf) = context.tree_nodes.get(leaf_id) else {
            continue;
        };
        let stored_sweep = stored
            .get(&leaf_id.to_string())
            .is_some_and(|leaf| leaf.unilateral_exit_sweep.is_some());
        if let Some(reason) = dropped_leaf_reason(leaf_id, state, stored_sweep) {
            skipped.push(SkippedLeaf {
                leaf_id: leaf_id.to_string(),
                value_sats: leaf.value,
                reason,
            });
        }
    }
    skipped
}

/// Updates the statuses of `transactions` from the chain. Returns whether a
/// transaction outside them spent an outpoint they still need.
pub(super) async fn check_recovery_transactions(
    queries: &mut ChainQueries,
    transactions: &mut [RecoveryTransaction],
) -> Result<bool, SdkError> {
    let inputs = transactions
        .iter()
        .map(exit_check_input)
        .collect::<Result<Vec<_>, SdkError>>()?;
    let check = resolve_exit_check(queries, &inputs).await;

    for tx in transactions.iter_mut() {
        let txid = Txid::from_str(&tx.txid)
            .map_err(|e| SdkError::InvalidInput(format!("Invalid txid {}: {e}", tx.txid)))?;
        tx.status = status_after_check(tx.status, &check, &txid);
    }

    resolve_statuses(queries.chain_service(), transactions).await?;
    Ok(check.diverged)
}

/// Why a quote leaves out a leaf its context dropped. `None` when the leaf's
/// exit finished in a sweep, which the chain read or a stored sweep shows.
fn dropped_leaf_reason(
    leaf_id: &TreeNodeId,
    state: &WalletExitChainState,
    stored_sweep: bool,
) -> Option<SkippedLeafReason> {
    if stored_sweep || is_swept(state, leaf_id) {
        return None;
    }
    Some(if state.stopped_leaves.contains(leaf_id) {
        SkippedLeafReason::NotRecoverable {
            message: "A transaction the wallet cannot continue from took the leaf's exit \
                      on-chain"
                .to_string(),
        }
    } else {
        // The wallet drops such a leaf from the context only when its own
        // lookup had no result.
        SkippedLeafReason::Unverified
    })
}

fn skipped_by_planner(leaf: &UnilateralExitSkippedLeaf) -> SkippedLeaf {
    SkippedLeaf {
        leaf_id: leaf.id.to_string(),
        value_sats: leaf.value,
        reason: match &leaf.reason {
            UnilateralExitSkipReason::Unprofitable => SkippedLeafReason::FeeExceedsValue,
            UnilateralExitSkipReason::Unexitable(reason) => SkippedLeafReason::NotRecoverable {
                message: format!("The leaf cannot be exited: {reason}"),
            },
        },
    }
}

fn is_swept(state: &WalletExitChainState, leaf_id: &TreeNodeId) -> bool {
    state.refunds.iter().any(|refund| {
        refund.leaf_id == *leaf_id && matches!(refund.state, WalletExitRefundState::Swept)
    })
}

/// The sweep's fee: total input value minus output value.
fn sweep_fee(sweep_psbt: &bitcoin::Psbt) -> u64 {
    let in_value: u64 = sweep_psbt
        .inputs
        .iter()
        .filter_map(|i| i.witness_utxo.as_ref())
        .map(|o| o.value.to_sat())
        .fold(0u64, u64::saturating_add);
    let out_value: u64 = sweep_psbt
        .unsigned_tx
        .output
        .iter()
        .map(|o| o.value.to_sat())
        .fold(0u64, u64::saturating_add);
    in_value.saturating_sub(out_value)
}

/// Converts a sat/vByte fee rate (the public API unit) to sat/kW (the exit
/// engine's unit): one vByte is 4 weight units, so 1 sat/vByte is 250 sat/kW.
fn sat_per_kw_from_vbyte(sat_per_vbyte: u64) -> u64 {
    sat_per_vbyte.saturating_mul(250)
}

/// The signed input weight and a representative output scriptPubKey for a
/// funding kind.
fn funding_kind_params(kind: &CpfpFundingKind) -> Result<(u64, ScriptBuf), SdkError> {
    // Only the scriptPubKey length matters here (it fixes output weight and
    // dust), so any valid program of the right size works.
    let witness_script = |version, program: &[u8]| -> Result<ScriptBuf, SdkError> {
        let program = bitcoin::WitnessProgram::new(version, program).map_err(|e| {
            SdkError::Generic(format!("invalid representative witness program: {e}"))
        })?;
        Ok(ScriptBuf::new_witness_program(&program))
    };
    let (weight, script) = match kind {
        CpfpFundingKind::P2wpkh => (
            spark_wallet::p2wpkh_input_weight().to_wu(),
            witness_script(bitcoin::WitnessVersion::V0, &[0u8; 20])?,
        ),
        CpfpFundingKind::P2tr => (
            spark_wallet::p2tr_key_path_input_weight().to_wu(),
            witness_script(bitcoin::WitnessVersion::V1, &[0u8; 32])?,
        ),
        CpfpFundingKind::Custom {
            script_pubkey_hex,
            signed_input_weight,
        } => {
            let script = ScriptBuf::from_hex(script_pubkey_hex).map_err(|e| {
                SdkError::InvalidInput(format!("Invalid funding script_pubkey_hex: {e}"))
            })?;
            // Only native SegWit funding is supported: the exit threads txids from
            // unsigned txs, stable only when the input's scriptSig stays empty.
            // Reject here so the quote fails before funding is gathered.
            if !script.is_witness_program() {
                return Err(SdkError::InvalidInput(
                    "Custom funding must pay to a native SegWit (witness-program) script"
                        .to_string(),
                ));
            }
            (*signed_input_weight, script)
        }
    };
    Ok((weight, script))
}

impl ModelCpfpInput {
    /// Converts into the spark-wallet funding type. Takes `network` to derive
    /// the P2WPKH/P2TR script from a pubkey.
    fn into_funding_input(self, network: bitcoin::Network) -> Result<CpfpInput, SdkError> {
        let parse_txid = |s: &str| {
            Txid::from_str(s)
                .map_err(|e| SdkError::InvalidInput(format!("Invalid funding txid: {e}")))
        };
        match self {
            ModelCpfpInput::P2wpkh {
                txid,
                vout,
                value_sats: value,
                pubkey,
            } => {
                let pk = PublicKey::from_str(&pubkey)
                    .map_err(|e| SdkError::InvalidInput(format!("Invalid funding pubkey: {e}")))?;
                let script_pubkey =
                    Address::p2wpkh(&CompressedPublicKey(pk), network).script_pubkey();
                Ok(CpfpInput {
                    outpoint: OutPoint {
                        txid: parse_txid(&txid)?,
                        vout,
                    },
                    witness_utxo: TxOut {
                        value: Amount::from_sat(value),
                        script_pubkey,
                    },
                    signed_input_weight: spark_wallet::p2wpkh_input_weight().to_wu(),
                })
            }
            ModelCpfpInput::P2tr {
                txid,
                vout,
                value_sats: value,
                pubkey,
            } => {
                let xonly = parse_xonly(&pubkey)?;
                let secp = bitcoin::secp256k1::Secp256k1::verification_only();
                let script_pubkey = Address::p2tr(&secp, xonly, None, network).script_pubkey();
                Ok(CpfpInput {
                    outpoint: OutPoint {
                        txid: parse_txid(&txid)?,
                        vout,
                    },
                    witness_utxo: TxOut {
                        value: Amount::from_sat(value),
                        script_pubkey,
                    },
                    signed_input_weight: spark_wallet::p2tr_key_path_input_weight().to_wu(),
                })
            }
            ModelCpfpInput::Custom {
                txid,
                vout,
                value_sats: value,
                script_pubkey_hex,
                signed_input_weight,
            } => {
                let script_pubkey = ScriptBuf::from_hex(&script_pubkey_hex).map_err(|e| {
                    SdkError::InvalidInput(format!("Invalid funding scriptPubKey hex: {e}"))
                })?;
                // The exit signs/weighs funding inputs as SegWit and spends them in
                // a v3/TRUC anchor package; a legacy input breaks both. Reject it.
                if !script_pubkey.is_witness_program() {
                    return Err(SdkError::InvalidInput(
                        "Custom funding input must pay to a SegWit (witness-program) script"
                            .to_string(),
                    ));
                }
                Ok(CpfpInput {
                    outpoint: OutPoint {
                        txid: parse_txid(&txid)?,
                        vout,
                    },
                    witness_utxo: TxOut {
                        value: Amount::from_sat(value),
                        script_pubkey,
                    },
                    signed_input_weight,
                })
            }
        }
    }
}

/// Parses an x-only pubkey from hex, accepting both x-only (32-byte) and
/// compressed (33-byte) encodings.
fn parse_xonly(pubkey: &str) -> Result<XOnlyPublicKey, SdkError> {
    if let Ok(xonly) = XOnlyPublicKey::from_str(pubkey) {
        return Ok(xonly);
    }
    let pk = PublicKey::from_str(pubkey)
        .map_err(|e| SdkError::InvalidInput(format!("Invalid funding pubkey: {e}")))?;
    Ok(pk.x_only_public_key().0)
}

/// Reads what the chain has already done to `leaf_ids`, before any funding is
/// considered. Also returns the sweeps the results show.
async fn resolve_exit_chain_state(
    queries: &mut ChainQueries,
    tree_nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
    refund_addresses: &HashMap<TreeNodeId, Address>,
) -> (WalletExitChainState, Vec<RefundSweep>) {
    let (state, sweeps) = queries
        .resolve(|observed| {
            let scan = scan_exit_chain(tree_nodes, leaf_ids, refund_addresses, observed);
            ((scan.state, scan.sweeps), scan.pending)
        })
        .await;
    debug!(
        nodes = state.nodes.len(),
        refunds = state.refunds.len(),
        "resolve_exit_chain_state: what the chain has already done"
    );
    (state, sweeps)
}

/// Reads back the state a caller carried over from `prepare_recover_funds`.
fn exit_chain_state_from_model(
    state: &ModelExitChainState,
) -> Result<WalletExitChainState, SdkError> {
    Ok(WalletExitChainState {
        nodes: state
            .confirmed_nodes
            .iter()
            .map(|node| {
                Ok(WalletConfirmedExitNode {
                    block_height: node.block_height,
                    node_id: node_id(&node.node_id)?,
                    confirmed_by: match node.confirmed_by {
                        ExitNodeConfirmation::Cpfp => WalletExitNodeConfirmation::Cpfp,
                        ExitNodeConfirmation::Direct => WalletExitNodeConfirmation::Direct,
                    },
                })
            })
            .collect::<Result<_, SdkError>>()?,
        refunds: state
            .refunds
            .iter()
            .map(|refund| {
                let restored = match &refund.state {
                    ExitRefundState::OnChain {
                        tx_hex,
                        vout,
                        value_sats,
                        block_height,
                    } => WalletExitRefundState::OnChain {
                        block_height: *block_height,
                        tx: deserialize_hex(tx_hex).map_err(|e| {
                            SdkError::InvalidInput(format!("Invalid refund transaction: {e}"))
                        })?,
                        vout: *vout,
                        value: *value_sats,
                    },
                    ExitRefundState::Swept => WalletExitRefundState::Swept,
                };
                Ok(WalletExitRefund {
                    leaf_id: node_id(&refund.leaf_id)?,
                    state: restored,
                })
            })
            .collect::<Result<_, SdkError>>()?,
        stopped_leaves: node_ids(&state.stopped_leaf_ids)?,
        unverified_nodes: node_ids(&state.unverified_node_ids)?,
        unverifiable_confirmed_nodes: node_ids(&state.unverifiable_confirmed_node_ids)?,
    })
}

fn node_id(id: &str) -> Result<TreeNodeId, SdkError> {
    TreeNodeId::from_str(id).map_err(|e| SdkError::InvalidInput(format!("Invalid id {id}: {e}")))
}

pub(super) fn node_ids(ids: &[String]) -> Result<Vec<TreeNodeId>, SdkError> {
    ids.iter().map(|id| node_id(id)).collect()
}

/// Restates the wallet's on-chain reading in the API's own types: ids as strings
/// and transactions as hex, so a caller can read what an exit has already done
/// rather than hand an opaque token back.
fn exit_chain_state_model(state: WalletExitChainState) -> ModelExitChainState {
    ModelExitChainState {
        confirmed_nodes: state
            .nodes
            .into_iter()
            .map(|node| ConfirmedExitNode {
                node_id: node.node_id.to_string(),
                confirmed_by: match node.confirmed_by {
                    WalletExitNodeConfirmation::Cpfp => ExitNodeConfirmation::Cpfp,
                    WalletExitNodeConfirmation::Direct => ExitNodeConfirmation::Direct,
                },
                block_height: node.block_height,
            })
            .collect(),
        refunds: state
            .refunds
            .into_iter()
            .map(|refund| ExitRefund {
                leaf_id: refund.leaf_id.to_string(),
                state: match refund.state {
                    WalletExitRefundState::OnChain {
                        tx,
                        vout,
                        value,
                        block_height,
                    } => ExitRefundState::OnChain {
                        tx_hex: serialize_hex(&tx),
                        vout,
                        value_sats: value,
                        block_height,
                    },
                    WalletExitRefundState::Swept => ExitRefundState::Swept,
                },
            })
            .collect(),
        stopped_leaf_ids: ids(state.stopped_leaves),
        unverified_node_ids: ids(state.unverified_nodes),
        unverifiable_confirmed_node_ids: ids(state.unverifiable_confirmed_nodes),
    }
}

fn ids(ids: Vec<TreeNodeId>) -> Vec<String> {
    ids.into_iter().map(|id| id.to_string()).collect()
}

/// Fills in the status of every transaction that is neither on-chain nor
/// unreadable: whether it can go out now, or what it is waiting on. Derived from
/// the whole list, so it is resolved once the list is complete rather than as
/// each entry is built.
///
/// A relative timelock counts from the height of the input being spent, so
/// maturity needs that height and the chain tip. Both are read here rather than
/// fetched again per transaction. A height that cannot be read leaves the
/// transaction waiting rather than ready, the direction that never sends a
/// transaction the mempool would reject.
pub(super) async fn resolve_statuses(
    chain: &dyn BitcoinChainService,
    transactions: &mut [RecoveryTransaction],
) -> Result<(), SdkError> {
    let confirmed: HashSet<&str> = transactions
        .iter()
        .filter(|tx| matches!(tx.status, ExitTransactionStatus::Confirmed { .. }))
        .map(|tx| tx.txid.as_str())
        .collect();
    let met: Vec<bool> = transactions
        .iter()
        .map(|tx| {
            tx.depends_on
                .iter()
                .all(|dep| confirmed.contains(dep.as_str()))
        })
        .collect();

    // Seeded from the list, which already carries the height of everything the
    // exit walk confirmed, so the chain is only asked about inputs from outside
    // it: the ancestor a resumed exit starts from.
    let mut heights: HashMap<Txid, Option<u32>> = transactions
        .iter()
        .filter_map(|tx| match tx.status {
            ExitTransactionStatus::Confirmed {
                block_height: Some(height),
            } => Some((Txid::from_str(&tx.txid).ok()?, Some(height))),
            _ => None,
        })
        .collect();
    let mut tip: Option<u32> = None;

    let mut resolved = Vec::with_capacity(transactions.len());
    for (tx, met) in transactions.iter().zip(&met) {
        // On-chain, or its own on-chain state unknown: neither leaves anything
        // to wait for.
        if matches!(
            tx.status,
            ExitTransactionStatus::Confirmed { .. } | ExitTransactionStatus::Unverified
        ) {
            resolved.push(tx.status);
            continue;
        }
        if !met {
            resolved.push(ExitTransactionStatus::WaitingForDependencies);
            continue;
        }
        if tx.csv_timelock_blocks.is_none() {
            resolved.push(ExitTransactionStatus::Ready);
            continue;
        }
        let decoded = deserialize_hex::<Transaction>(&tx.tx_hex)
            .map_err(|e| SdkError::InvalidInput(format!("Invalid transaction: {e}")))?;
        let Some(spendable_at) = spendable_at_height(chain, &decoded, &mut heights).await else {
            resolved.push(ExitTransactionStatus::WaitingForTimelock {
                spendable_at_height: None,
            });
            continue;
        };
        if tip.is_none() {
            tip = chain.tip_height().await.ok();
        }
        // A transaction is accepted while it would be valid in the next block.
        let matured = tip.is_some_and(|tip| tip.saturating_add(1) >= spendable_at);
        resolved.push(if matured {
            ExitTransactionStatus::Ready
        } else {
            ExitTransactionStatus::WaitingForTimelock {
                spendable_at_height: Some(spendable_at),
            }
        });
    }

    for (tx, status) in transactions.iter_mut().zip(resolved) {
        tx.status = status;
    }
    Ok(())
}

/// The status of one kept transaction after a chain read.
///
/// - The chain has it in a block: `Confirmed`, at the height the read reported,
///   or at the height already held when the read settled it along the spend
///   chain instead of asking about it.
/// - The chain was asked and reported it is not in a block: reset to a
///   placeholder `resolve_statuses` replaces, discarding a confirmation a reorg
///   has undone.
/// - Anything else: left as it is. `resolve_statuses` recomputes every status
///   except `Confirmed` and `Unverified`, and neither of those may be dropped on
///   a read that did not contradict it.
fn status_after_check(
    current: ExitTransactionStatus,
    check: &ExitCheck,
    txid: &Txid,
) -> ExitTransactionStatus {
    let held_height = match current {
        ExitTransactionStatus::Confirmed { block_height } => block_height,
        _ => None,
    };
    match (check.confirmed.get(txid), current) {
        (Some(block_height), _) => ExitTransactionStatus::Confirmed {
            block_height: block_height.or(held_height),
        },
        (None, ExitTransactionStatus::Confirmed { .. }) if check.not_confirmed.contains(txid) => {
            ExitTransactionStatus::WaitingForDependencies
        }
        (None, _) => current,
    }
}

/// The first block height that can include `tx`, given the relative timelocks on
/// its inputs. `None` when an input's confirmation height cannot be established,
/// which leaves maturity unknown.
async fn spendable_at_height(
    chain: &dyn BitcoinChainService,
    tx: &Transaction,
    heights: &mut HashMap<Txid, Option<u32>>,
) -> Option<u32> {
    let mut spendable_at = None;
    for input in &tx.input {
        let blocks = match input.sequence.to_relative_lock_time() {
            Some(bitcoin::relative::LockTime::Blocks(blocks)) => u32::from(blocks.value()),
            _ => continue,
        };
        if blocks == 0 {
            continue;
        }
        let funded_by = input.previous_output.txid;
        if heights.get(&funded_by).is_none() {
            let height = match chain.get_transaction_status(funded_by.to_string()).await {
                Ok(status) if status.confirmed => status.block_height,
                _ => None,
            };
            heights.insert(funded_by, height);
        }
        let known = heights.get(&funded_by).copied().flatten();
        // One unreadable input is enough to leave the whole transaction unknown:
        // it may be the one that matures last.
        let height = known?;
        spendable_at = Some(spendable_at.unwrap_or(0).max(height.saturating_add(blocks)));
    }
    spendable_at
}

/// Decodes one kept transaction into what the check reads.
fn exit_check_input(tx: &RecoveryTransaction) -> Result<ExitCheckInput, SdkError> {
    let decode = |hex: &str| {
        deserialize_hex::<Transaction>(hex)
            .map_err(|e| SdkError::InvalidInput(format!("Invalid transaction: {e}")))
    };
    Ok(ExitCheckInput {
        tx: decode(&tx.tx_hex)?,
        cpfp: tx.cpfp_tx_hex.as_deref().map(decode).transpose()?,
        confirmed: matches!(tx.status, ExitTransactionStatus::Confirmed { .. }),
    })
}

/// Which kept transactions are in a block, and whether the on-chain state
/// diverged from them, as the chain service reports it.
async fn resolve_exit_check(queries: &mut ChainQueries, inputs: &[ExitCheckInput]) -> ExitCheck {
    let check = queries
        .resolve(|observed| {
            let mut check = check_exit_chain(inputs, observed);
            let pending = std::mem::take(&mut check.pending);
            (check, pending)
        })
        .await;
    debug!(
        confirmed = check.confirmed.len(),
        diverged = check.diverged,
        "resolve_exit_check: read back"
    );
    check
}

/// Follows the supplied funding to what it is worth now. The only chain reading
/// a build does: the tree was read while preparing.
async fn resolve_funding(queries: &mut ChainQueries, supplied: Vec<CpfpInput>) -> Vec<CpfpInput> {
    let resolved = queries
        .resolve(|observed| {
            let scan = scan_funding(&supplied, observed);
            (scan.inputs, scan.pending)
        })
        .await;
    debug!(
        supplied = supplied.len(),
        resolved = resolved.len(),
        "resolve_funding: what the supplied funding is worth now"
    );
    resolved
}

/// The status a built transaction starts at. What an unconfirmed one is waiting
/// for takes the whole list to work out, so [`resolve_statuses`] replaces this.
fn initial_status(status: ExitTxStatus) -> ExitTransactionStatus {
    match status {
        ExitTxStatus::Confirmed { block_height } => {
            ExitTransactionStatus::Confirmed { block_height }
        }
        ExitTxStatus::Unconfirmed => ExitTransactionStatus::WaitingForDependencies,
        ExitTxStatus::Unverified => ExitTransactionStatus::Unverified,
    }
}

/// The sweep's starting status, derived from the refunds it spends. A verified
/// refund is spent-and-dropped once its sweep confirms (the exit then returns
/// with no sweep), so a freshly-returned sweep over verified refunds is never
/// yet on-chain. An unverified refund (its chain lookup failed) could already be
/// on-chain and swept without us knowing, so the sweep is `Unverified`.
fn sweep_initial_status(build: &UnilateralExitBuild) -> ExitTransactionStatus {
    let any_refund_unverified = build
        .branches
        .iter()
        .flat_map(|b| b.txs.iter())
        .any(|t| t.kind == ExitTxKind::Refund && t.status == ExitTxStatus::Unverified);
    if any_refund_unverified {
        ExitTransactionStatus::Unverified
    } else {
        ExitTransactionStatus::WaitingForDependencies
    }
}

/// Signs a PSBT via the external `CpfpSigner`, returning the tx as hex.
/// Ephemeral anchor inputs are finalized here (`OP_TRUE`, no signature).
async fn sign_psbt_via(
    mut psbt: bitcoin::Psbt,
    signer: Option<&dyn CpfpSigner>,
) -> Result<String, SdkError> {
    let signer = signer.ok_or_else(no_signer)?;
    for input in &mut psbt.inputs {
        if let Some(txo) = &input.witness_utxo
            && is_ephemeral_anchor_output(txo)
        {
            input.final_script_witness = Some(bitcoin::Witness::new());
        }
    }
    let out_bytes = signer
        .sign_psbt(psbt.serialize())
        .await
        .map_err(|e| SdkError::Signer(format!("CPFP signer error: {e}")))?;
    let out_psbt = bitcoin::Psbt::deserialize(&out_bytes)
        .map_err(|e| SdkError::Generic(format!("Failed to deserialize signed PSBT: {e}")))?;
    ensure_all_inputs_finalized(&out_psbt)?;
    Ok(serialize_hex(&out_psbt.extract_tx_unchecked_fee_rate()))
}

/// Finalizes the sweep. Refund inputs are already signed by spark-wallet, so the
/// external signer is only invoked when CPFP-change inputs still need it.
async fn finalize_sweep(
    psbt: bitcoin::Psbt,
    signer: Option<&dyn CpfpSigner>,
) -> Result<String, SdkError> {
    let needs_signer = psbt
        .inputs
        .iter()
        .any(|input| input.final_script_witness.is_none());
    let psbt = if needs_signer {
        let out_bytes = signer
            .ok_or_else(no_signer)?
            .sign_psbt(psbt.serialize())
            .await
            .map_err(|e| SdkError::Signer(format!("Sweep signer error: {e}")))?;
        bitcoin::Psbt::deserialize(&out_bytes).map_err(|e| {
            SdkError::Generic(format!("Failed to deserialize signed sweep PSBT: {e}"))
        })?
    } else {
        psbt
    };
    ensure_all_inputs_finalized(&psbt)?;
    Ok(serialize_hex(&psbt.extract_tx_unchecked_fee_rate()))
}

fn no_signer() -> SdkError {
    SdkError::InvalidInput("A signer is needed to sign the funding inputs".to_string())
}

/// Rejects a PSBT with any input the signer left unfinalized (neither a witness
/// nor a scriptSig), so a missing signature fails here instead of at broadcast.
fn ensure_all_inputs_finalized(psbt: &bitcoin::Psbt) -> Result<(), SdkError> {
    if let Some(index) = psbt
        .inputs
        .iter()
        .position(|input| input.final_script_witness.is_none() && input.final_script_sig.is_none())
    {
        return Err(SdkError::Signer(format!(
            "PSBT input {index} was not signed"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chain::{
            Outspend, TxStatus, Utxo,
            stub::{ChainStub, tx_paying},
        },
        error::SignerError,
    };
    use bitcoin::hashes::Hash;
    use spark_wallet::{ChainQuery, ChainResult, ExitBranch, ExitTx, SpendInfo};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn refund_tx(status: ExitTxStatus) -> ExitTx {
        ExitTx {
            kind: ExitTxKind::Refund,
            node_id: None,
            txid: Txid::from_byte_array([3; 32]),
            base_tx: Transaction {
                version: bitcoin::transaction::Version::TWO,
                lock_time: bitcoin::absolute::LockTime::ZERO,
                input: vec![],
                output: vec![],
            },
            to_sign: None,
            csv_timelock_blocks: None,
            depends_on: vec![],
            status,
        }
    }

    fn build_with_refund(status: ExitTxStatus) -> UnilateralExitBuild {
        UnilateralExitBuild {
            fan_out: None,
            branches: vec![ExitBranch {
                leaf_id: TreeNodeId::from_str("leaf").unwrap(),
                txs: vec![refund_tx(status)],
            }],
            refund_outputs: vec![],
            cpfp_change_inputs: vec![],
            recoverable_value_sat: 0,
            cpfp_fee_sat: 0,
            fanout_fee_sat: 0,
        }
    }

    #[test]
    fn a_sweep_over_verified_refunds_starts_out_waiting_on_them() {
        assert_eq!(
            sweep_initial_status(&build_with_refund(ExitTxStatus::Unconfirmed)),
            ExitTransactionStatus::WaitingForDependencies
        );
    }

    #[test]
    fn a_sweep_over_an_unverified_refund_is_unverified() {
        assert_eq!(
            sweep_initial_status(&build_with_refund(ExitTxStatus::Unverified)),
            ExitTransactionStatus::Unverified
        );
    }

    fn unsigned_two_input_psbt() -> bitcoin::Psbt {
        let tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![
                bitcoin::TxIn {
                    previous_output: OutPoint {
                        txid: Txid::from_byte_array([1; 32]),
                        vout: 0,
                    },
                    ..Default::default()
                },
                bitcoin::TxIn {
                    previous_output: OutPoint {
                        txid: Txid::from_byte_array([2; 32]),
                        vout: 0,
                    },
                    ..Default::default()
                },
            ],
            output: vec![TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        let mut psbt = bitcoin::Psbt::from_unsigned_tx(tx).unwrap();
        for input in &mut psbt.inputs {
            input.witness_utxo = Some(TxOut {
                value: Amount::from_sat(2_000),
                script_pubkey: ScriptBuf::new(),
            });
        }
        psbt
    }

    fn finalize_input(input: &mut bitcoin::psbt::Input) {
        let mut witness = bitcoin::Witness::new();
        witness.push([0x01u8]);
        input.final_script_witness = Some(witness);
    }

    /// A `CpfpSigner` that finalizes only the first `finalize` inputs.
    struct PartialSigner {
        finalize: usize,
    }

    #[macros::async_trait]
    impl CpfpSigner for PartialSigner {
        async fn sign_psbt(&self, psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError> {
            let mut psbt = bitcoin::Psbt::deserialize(&psbt_bytes).unwrap();
            for input in psbt.inputs.iter_mut().take(self.finalize) {
                finalize_input(input);
            }
            Ok(psbt.serialize())
        }
    }

    #[test]
    fn ensure_all_inputs_finalized_rejects_unsigned() {
        assert!(ensure_all_inputs_finalized(&unsigned_two_input_psbt()).is_err());
    }

    #[test]
    fn ensure_all_inputs_finalized_accepts_finalized() {
        let mut psbt = unsigned_two_input_psbt();
        psbt.inputs.iter_mut().for_each(finalize_input);
        assert!(ensure_all_inputs_finalized(&psbt).is_ok());
    }

    #[macros::async_test_all]
    async fn sign_psbt_via_errors_when_an_input_is_left_unsigned() {
        let result = sign_psbt_via(
            unsigned_two_input_psbt(),
            Some(&PartialSigner { finalize: 1 }),
        )
        .await;
        assert!(matches!(result, Err(SdkError::Signer(_))));
    }

    #[macros::async_test_all]
    async fn signing_without_a_signer_asks_for_one() {
        let result = sign_psbt_via(unsigned_two_input_psbt(), None).await;
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }

    #[macros::async_test_all]
    async fn sign_psbt_via_succeeds_when_every_input_is_signed() {
        let result = sign_psbt_via(
            unsigned_two_input_psbt(),
            Some(&PartialSigner { finalize: 2 }),
        )
        .await;
        assert!(result.is_ok());
    }

    /// A transaction spending `parent` under a relative timelock of `csv` blocks.
    fn timelocked_tx(parent: Txid, csv: u32) -> Transaction {
        Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                previous_output: OutPoint {
                    txid: parent,
                    vout: 0,
                },
                sequence: bitcoin::Sequence::from_height(u16::try_from(csv).unwrap()),
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    fn model_tx(
        tx: &Transaction,
        csv: Option<u32>,
        depends_on: Vec<String>,
        status: ExitTransactionStatus,
    ) -> RecoveryTransaction {
        RecoveryTransaction {
            kind: RecoveryTxKind::Node,
            node_id: None,
            txid: tx.compute_txid().to_string(),
            tx_hex: serialize_hex(tx),
            cpfp_tx_hex: None,
            csv_timelock_blocks: csv,
            depends_on,
            status,
        }
    }

    #[test]
    fn only_a_swept_refund_ends_an_exit() {
        let id = |id: &str| TreeNodeId::from_str(id).unwrap();
        let refund = |leaf_id: &str, state| WalletExitRefund {
            leaf_id: id(leaf_id),
            state,
        };
        let state = WalletExitChainState {
            nodes: Vec::new(),
            refunds: vec![
                refund("swept", WalletExitRefundState::Swept),
                refund(
                    "on-chain",
                    WalletExitRefundState::OnChain {
                        tx: timelocked_tx(Txid::from_byte_array([1; 32]), 0),
                        vout: 0,
                        value: 1_000,
                        block_height: None,
                    },
                ),
            ],
            stopped_leaves: vec![id("stopped"), id("on-chain")],
            unverified_nodes: Vec::new(),
            unverifiable_confirmed_nodes: Vec::new(),
        };
        let swept: Vec<&str> = ["swept", "on-chain", "stopped", "open"]
            .into_iter()
            .filter(|leaf_id| is_swept(&state, &id(leaf_id)))
            .collect();
        assert_eq!(swept, vec!["swept"], "a stopped leaf stays recoverable");

        assert_eq!(dropped_leaf_reason(&id("swept"), &state, false), None);
        assert!(matches!(
            dropped_leaf_reason(&id("stopped"), &state, false),
            Some(SkippedLeafReason::NotRecoverable { .. })
        ));
        assert_eq!(
            dropped_leaf_reason(&id("open"), &state, false),
            Some(SkippedLeafReason::Unverified),
            "a leaf the chain read says nothing about"
        );
        assert_eq!(
            dropped_leaf_reason(&id("open"), &state, true),
            None,
            "a sweep an earlier read stored"
        );
    }

    /// An exited leaf with its refund in a block, and the chain stub that shows it.
    struct ExitedLeaf {
        tree_nodes: HashMap<TreeNodeId, TreeNode>,
        leaf_id: TreeNodeId,
        refund_addresses: HashMap<TreeNodeId, Address>,
        refund_outpoint: OutPoint,
        chain: ChainStub,
    }

    fn exited_leaf(leaf_id: &str, refund_byte: u8) -> ExitedLeaf {
        let mut leaf = spark_wallet::tree_store_tests::create_test_node_with_parent(
            leaf_id,
            None,
            spark_wallet::TreeNodeStatus::Exited,
        );
        let deposit = OutPoint {
            txid: Txid::from_byte_array([refund_byte.wrapping_add(100); 32]),
            vout: 0,
        };
        leaf.node_tx = tx_paying(deposit, 1_000);
        let address = Address::p2tr(
            &bitcoin::secp256k1::Secp256k1::verification_only(),
            leaf.verifying_public_key.x_only_public_key().0,
            None,
            bitcoin::Network::Regtest,
        );
        let refund = tx_paying(
            OutPoint {
                txid: leaf.node_tx.compute_txid(),
                vout: 0,
            },
            900,
        );
        let refund_outpoint = OutPoint {
            txid: Txid::from_byte_array([refund_byte; 32]),
            vout: 0,
        };
        let mut chain = ChainStub::default();
        // The leaf's node tx is in a block.
        chain.outspends.insert(
            (deposit.txid.to_string(), deposit.vout),
            ChainStub::spent(&leaf.node_tx.compute_txid().to_string(), true, Some(90)),
        );
        chain.address_txos.insert(
            address.to_string(),
            vec![Utxo {
                txid: refund_outpoint.txid.to_string(),
                vout: 0,
                value: 900,
                status: TxStatus {
                    confirmed: true,
                    block_height: Some(95),
                    block_time: None,
                },
            }],
        );
        chain
            .transactions
            .insert(refund_outpoint.txid.to_string(), serialize_hex(&refund));
        ExitedLeaf {
            leaf_id: leaf.id.clone(),
            refund_addresses: HashMap::from([(leaf.id.clone(), address)]),
            tree_nodes: HashMap::from([(leaf.id.clone(), leaf)]),
            refund_outpoint,
            chain,
        }
    }

    impl ExitedLeaf {
        fn refund_spent(mut self, outspend: Outspend) -> Self {
            let outpoint = self.refund_outpoint;
            self.chain
                .outspends
                .insert((outpoint.txid.to_string(), outpoint.vout), outspend);
            self
        }

        async fn checks(self) -> Vec<UpdateLeafRecovery> {
            let leaf_ids = [self.leaf_id.clone()];
            let mut queries = ChainQueries::new(std::sync::Arc::new(self.chain));
            queries
                .resolve(|observed| {
                    let scan = scan_exit_chain(
                        &self.tree_nodes,
                        &leaf_ids,
                        &self.refund_addresses,
                        observed,
                    );
                    ((), scan.pending)
                })
                .await;
            exited_leaf_checks(
                &self.tree_nodes,
                &leaf_ids,
                &self.refund_addresses,
                &queries.fetched(),
                1_000,
            )
        }
    }

    fn checked_leaf(leaf_id: &str, sweep: Option<ChainTransaction>) -> UpdateLeafRecovery {
        UpdateLeafRecovery {
            leaf_id: leaf_id.to_string(),
            chain_checked_at: Some(1_000),
            unilateral_exit_sweep: sweep,
            ..Default::default()
        }
    }

    #[macros::async_test_all]
    async fn an_exited_leaf_is_stored_with_the_sweep_of_its_refund() {
        let swept = exited_leaf("leaf", 5)
            .refund_spent(ChainStub::spent("sweep", true, Some(100)))
            .checks()
            .await;
        // The stub's spender txid does not parse, so the outspend query has no result.
        assert!(swept.is_empty());

        let sweep = Txid::from_byte_array([9; 32]).to_string();
        let swept = exited_leaf("leaf", 5)
            .refund_spent(ChainStub::spent(&sweep, true, Some(100)))
            .checks()
            .await;
        assert_eq!(
            swept,
            vec![checked_leaf(
                "leaf",
                Some(ChainTransaction {
                    txid: sweep.clone(),
                    block_height: 100,
                })
            )]
        );

        // A refund with no sweep, or with one only in the mempool, gets a check
        // without a sweep.
        for outspend in [Outspend::Unspent, ChainStub::spent(&sweep, false, None)] {
            let open = exited_leaf("leaf", 5).refund_spent(outspend).checks().await;
            assert_eq!(open, vec![checked_leaf("leaf", None)]);
        }
    }

    #[macros::async_test_all]
    async fn an_exited_leaf_without_every_result_is_left_for_the_next_sync() {
        // The query for the refund's spend has no result.
        let missing = exited_leaf("leaf", 5).checks().await;
        assert!(missing.is_empty());

        // The SDK stores a sweep only with the height of its block.
        let sweep = Txid::from_byte_array([9; 32]).to_string();
        let unnamed = exited_leaf("leaf", 5)
            .refund_spent(ChainStub::spent(&sweep, true, None))
            .checks()
            .await;
        assert!(unnamed.is_empty());
    }

    #[macros::async_test_all]
    async fn an_exited_leaf_with_no_refund_at_its_address_stays_to_recover() {
        let mut exited = exited_leaf("leaf", 5);
        for txos in exited.chain.address_txos.values_mut() {
            txos.clear();
        }

        assert_eq!(exited.checks().await, vec![checked_leaf("leaf", None)]);
    }

    #[test]
    fn an_exited_leaf_is_checked_on_its_own_results() {
        let fetched = exited_leaf("fetched", 5);
        let waiting = exited_leaf("waiting", 6);
        let tree_nodes: HashMap<TreeNodeId, TreeNode> = fetched
            .tree_nodes
            .clone()
            .into_iter()
            .chain(waiting.tree_nodes.clone())
            .collect();
        let refund_addresses: HashMap<TreeNodeId, Address> = fetched
            .refund_addresses
            .clone()
            .into_iter()
            .chain(waiting.refund_addresses.clone())
            .collect();
        let leaf_ids = [fetched.leaf_id.clone(), waiting.leaf_id.clone()];
        // Every result for the first leaf, none for the second.
        let deposit = tree_nodes[&fetched.leaf_id].node_tx.input[0].previous_output;
        let node_txid = tree_nodes[&fetched.leaf_id].node_tx.compute_txid();
        let observed = vec![
            Observation {
                query: ChainQuery::Outspend(deposit),
                result: ChainResult::Spend(Some(SpendInfo {
                    spender_txid: node_txid,
                    confirmed: true,
                    block_height: Some(90),
                })),
            },
            Observation {
                query: ChainQuery::RefundAddress {
                    leaf_id: fetched.leaf_id.clone(),
                    address: refund_addresses[&fetched.leaf_id].clone(),
                },
                result: ChainResult::AddressUtxos(Vec::new()),
            },
        ];

        let checks =
            exited_leaf_checks(&tree_nodes, &leaf_ids, &refund_addresses, &observed, 1_000);

        assert_eq!(checks, vec![checked_leaf("fetched", None)]);
    }

    fn check_of(
        confirmed: &[(Txid, Option<u32>)],
        not_confirmed: &[Txid],
    ) -> spark_wallet::ExitCheck {
        spark_wallet::ExitCheck {
            confirmed: confirmed.iter().copied().collect(),
            diverged: false,
            not_confirmed: not_confirmed.iter().copied().collect(),
            pending: Vec::new(),
        }
    }

    /// The check settles an ancestor along the spend chain without asking, so it
    /// reports no height for it. Overwriting with that loses the height the
    /// caller is holding, which is what a child's timelock counts from.
    #[macros::async_test_all]
    async fn a_settled_ancestor_keeps_the_height_the_caller_already_had() {
        let txid = Txid::from_byte_array([4; 32]);
        let held = ExitTransactionStatus::Confirmed {
            block_height: Some(880_000),
        };

        assert_eq!(
            status_after_check(held, &check_of(&[(txid, None)], &[]), &txid),
            held,
            "a height-less confirmation does not erase the one held"
        );
        assert_eq!(
            status_after_check(held, &check_of(&[(txid, Some(880_004))], &[]), &txid),
            ExitTransactionStatus::Confirmed {
                block_height: Some(880_004)
            },
            "a height the chain did report wins"
        );
    }

    /// A transaction the chain reports as not in a block loses its stored
    /// `Confirmed`, so a reorg cannot leave the exit reporting `Done` over a
    /// transaction that is no longer there. A failed lookup leaves it alone.
    #[macros::async_test_all]
    async fn a_confirmation_the_chain_contradicts_is_dropped() {
        let txid = Txid::from_byte_array([5; 32]);
        let held = ExitTransactionStatus::Confirmed {
            block_height: Some(880_000),
        };

        assert_eq!(
            status_after_check(held, &check_of(&[], &[txid]), &txid),
            ExitTransactionStatus::WaitingForDependencies,
            "re-derived, so resolve_statuses works out where it really stands"
        );
        assert_eq!(
            status_after_check(held, &check_of(&[], &[]), &txid),
            held,
            "no answer either way leaves the caller's record alone"
        );
    }

    /// `Unverified` says the chain could not be read while the exit was built, so
    /// the SDK cannot tell whether an earlier fee-bumping child already spent the
    /// funding this transaction would use. A check reads only the kept exit, never
    /// the funding, so it can promote the status on finding the transaction in a
    /// block, but must not clear it on failing to.
    #[macros::async_test_all]
    async fn a_check_promotes_an_unverified_transaction_but_never_clears_it() {
        let txid = Txid::from_byte_array([6; 32]);

        assert_eq!(
            status_after_check(
                ExitTransactionStatus::Unverified,
                &check_of(&[], &[]),
                &txid
            ),
            ExitTransactionStatus::Unverified,
            "not found is not evidence that broadcasting it is safe"
        );
        assert_eq!(
            status_after_check(
                ExitTransactionStatus::Unverified,
                &check_of(&[(txid, Some(880_000))], &[]),
                &txid,
            ),
            ExitTransactionStatus::Confirmed {
                block_height: Some(880_000)
            },
            "found in a block settles it outright"
        );
    }

    /// `resolve_statuses` recomputes everything except `Confirmed` and
    /// `Unverified`, so a check that learned nothing about a transaction can leave
    /// its status alone and still have it come out up to date.
    #[macros::async_test_all]
    async fn a_check_that_learned_nothing_leaves_a_recomputable_status_alone() {
        let txid = Txid::from_byte_array([7; 32]);
        for status in [
            ExitTransactionStatus::Ready,
            ExitTransactionStatus::WaitingForDependencies,
            ExitTransactionStatus::WaitingForTimelock {
                spendable_at_height: Some(880_010),
            },
        ] {
            assert_eq!(
                status_after_check(status, &check_of(&[], &[]), &txid),
                status
            );
        }
    }

    /// A parent at height 100 with a 6 block timelock is spendable from block
    /// 106, so a tip of 105 is enough: the child would be valid in block 106.
    #[macros::async_test_all]
    async fn a_matured_timelock_is_ready_one_block_early() {
        let parent = model_tx(
            &timelocked_tx(Txid::from_byte_array([9; 32]), 0),
            None,
            vec![],
            ExitTransactionStatus::Confirmed {
                block_height: Some(100),
            },
        );
        let parent_txid = Txid::from_str(&parent.txid).unwrap();
        let child = model_tx(
            &timelocked_tx(parent_txid, 6),
            Some(6),
            vec![parent.txid.clone()],
            ExitTransactionStatus::WaitingForDependencies,
        );
        let chain = ChainStub {
            tip: Some(105),
            heights: HashMap::new(),
            ..Default::default()
        };

        let mut txs = vec![parent, child];
        resolve_statuses(&chain, &mut txs).await.unwrap();

        assert_eq!(txs[1].status, ExitTransactionStatus::Ready);
    }

    #[macros::async_test_all]
    async fn an_unmatured_timelock_reports_the_block_it_waits_for() {
        let parent = model_tx(
            &timelocked_tx(Txid::from_byte_array([9; 32]), 0),
            None,
            vec![],
            ExitTransactionStatus::Confirmed {
                block_height: Some(100),
            },
        );
        let parent_txid = Txid::from_str(&parent.txid).unwrap();
        let child = model_tx(
            &timelocked_tx(parent_txid, 6),
            Some(6),
            vec![parent.txid.clone()],
            ExitTransactionStatus::WaitingForDependencies,
        );
        let chain = ChainStub {
            tip: Some(104),
            heights: HashMap::new(),
            ..Default::default()
        };

        let mut txs = vec![parent, child];
        resolve_statuses(&chain, &mut txs).await.unwrap();

        assert_eq!(
            txs[1].status,
            ExitTransactionStatus::WaitingForTimelock {
                spendable_at_height: Some(106)
            }
        );
    }

    /// An unconfirmed parent holds the child at its dependencies: the timelock
    /// has no height to count from yet.
    #[macros::async_test_all]
    async fn an_unconfirmed_dependency_outranks_the_timelock() {
        let parent = model_tx(
            &timelocked_tx(Txid::from_byte_array([9; 32]), 0),
            None,
            vec![],
            ExitTransactionStatus::WaitingForDependencies,
        );
        let parent_txid = Txid::from_str(&parent.txid).unwrap();
        let child = model_tx(
            &timelocked_tx(parent_txid, 6),
            Some(6),
            vec![parent.txid.clone()],
            ExitTransactionStatus::WaitingForDependencies,
        );
        let chain = ChainStub {
            tip: Some(999),
            heights: HashMap::new(),
            ..Default::default()
        };

        let mut txs = vec![parent, child];
        resolve_statuses(&chain, &mut txs).await.unwrap();

        assert_eq!(txs[1].status, ExitTransactionStatus::WaitingForDependencies);
    }

    /// The exit a resume builds starts below the node it resumes from, so the
    /// height its first timelock counts from is not in the list. It comes from
    /// the chain instead.
    #[macros::async_test_all]
    async fn a_height_outside_the_list_is_read_from_the_chain() {
        let ancestor = Txid::from_byte_array([7; 32]);
        let tx = timelocked_tx(ancestor, 10);
        let chain = ChainStub {
            tip: Some(1_000),
            heights: HashMap::from([(ancestor.to_string(), 500)]),
            ..Default::default()
        };

        let mut txs = vec![model_tx(
            &tx,
            Some(10),
            vec![],
            ExitTransactionStatus::WaitingForDependencies,
        )];
        resolve_statuses(&chain, &mut txs).await.unwrap();

        assert_eq!(txs[0].status, ExitTransactionStatus::Ready);
    }

    /// An unreadable chain leaves the transaction waiting rather than ready:
    /// the direction that never sends what the mempool would reject.
    #[macros::async_test_all]
    async fn an_unreadable_height_leaves_the_timelock_unknown() {
        let tx = timelocked_tx(Txid::from_byte_array([7; 32]), 10);
        let chain = ChainStub {
            tip: Some(1_000),
            heights: HashMap::new(),
            ..Default::default()
        };

        let mut txs = vec![model_tx(
            &tx,
            Some(10),
            vec![],
            ExitTransactionStatus::WaitingForDependencies,
        )];
        resolve_statuses(&chain, &mut txs).await.unwrap();

        assert_eq!(
            txs[0].status,
            ExitTransactionStatus::WaitingForTimelock {
                spendable_at_height: None
            }
        );
    }
}
