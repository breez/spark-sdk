use std::str::FromStr;
use std::sync::Arc;

use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, ScriptBuf, Transaction, TxOut, Txid,
    XOnlyPublicKey,
    address::NetworkUnchecked,
    consensus::encode::{deserialize_hex, serialize_hex},
    secp256k1::PublicKey,
};

use spark_wallet::{
    AddressUtxo, ChainQuery, ChainResult, CpfpInput, ExitTxKind, ExitTxStatus, Observation,
    PreparedUnilateralExit, SpendInfo, TreeNodeId, UnilateralExitBuild, build_unilateral_exit,
    is_ephemeral_anchor_output, next_chain_queries,
};

use tracing::{debug, trace, warn};

use crate::{
    chain::{BitcoinChainService, Outspend},
    error::SdkError,
    models::{
        ConfirmationStatus, CpfpFundingKind, CpfpInput as ModelCpfpInput, ExitLeafSelection,
        PerBranchFunding, PrepareUnilateralExitRequest, PrepareUnilateralExitResponse,
        UnilateralExitLeaf, UnilateralExitRequest, UnilateralExitResponse,
        UnilateralExitTransaction, UnilateralExitTxKind,
    },
    signer::CpfpSigner,
};

use super::BreezSdk;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Quotes a unilateral exit without any funding UTXOs: selects which leaves
    /// would exit, computes the exact fee for the given funding kind, and reports
    /// how much to fund.
    pub async fn prepare_unilateral_exit(
        &self,
        request: PrepareUnilateralExitRequest,
    ) -> Result<PrepareUnilateralExitResponse, SdkError> {
        debug!(
            fee_rate_sat_per_vbyte = request.fee_rate_sat_per_vbyte,
            funding_kind = ?request.funding_kind,
            selection = ?request.selection,
            "prepare_unilateral_exit: quoting"
        );
        let btc_network: bitcoin::Network = self.config.network.into();

        let destination = request
            .destination
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|e| SdkError::InvalidInput(format!("Invalid destination address: {e}")))?
            .require_network(btc_network)
            .map_err(|e| SdkError::InvalidInput(format!("Address network mismatch: {e}")))?;
        let dest_script_len = destination.script_pubkey().len();

        // Leaf auto-resolution lives in the wallet.
        let selection = match request.selection {
            ExitLeafSelection::Auto => spark_wallet::ExitLeafSelection::Auto,
            ExitLeafSelection::Specific { leaf_ids } => {
                if leaf_ids.is_empty() {
                    return Err(SdkError::InvalidInput("No leaves to exit".to_string()));
                }
                let ids = leaf_ids
                    .iter()
                    .map(|s| {
                        TreeNodeId::from_str(s).map_err(|e| {
                            SdkError::InvalidInput(format!("Invalid leaf id {s}: {e}"))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                spark_wallet::ExitLeafSelection::Specific(ids)
            }
        };

        let (input_weight, output_script) = funding_kind_params(&request.funding_kind)?;
        let quote = self
            .spark_wallet
            .quote_unilateral_exit(
                sat_per_kw_from_vbyte(request.fee_rate_sat_per_vbyte),
                selection,
                input_weight,
                output_script.len(),
                output_script.minimal_non_dust().to_sat(),
                dest_script_len,
            )
            .await?;
        // No selected leaves is not an error: return an empty quote.
        let recoverable_value_sat = quote
            .selected_leaves
            .iter()
            .map(|l| l.value)
            .fold(0u64, u64::saturating_add);
        let leaves = quote
            .selected_leaves
            .iter()
            .map(|l| UnilateralExitLeaf {
                leaf_id: l.id.to_string(),
                value: l.value,
            })
            .collect();
        let per_branch_funding: Vec<PerBranchFunding> = quote
            .per_branch_funding
            .into_iter()
            .map(|(id, funding_sat)| PerBranchFunding {
                leaf_id: id.to_string(),
                funding_sat,
            })
            .collect();

        debug!(
            selected_leaves = quote.selected_leaves.len(),
            recoverable_value_sat,
            total_fee_sat = quote.total_fee_sat,
            fanout_fee_sat = quote.fanout_fee_sat,
            single_utxo_funding_sat = quote.single_utxo_funding_sat,
            branches = per_branch_funding.len(),
            "prepare_unilateral_exit: quote ready"
        );

        Ok(PrepareUnilateralExitResponse {
            leaves,
            recoverable_value_sat,
            total_fee_sat: quote.total_fee_sat,
            fanout_fee_sat: quote.fanout_fee_sat,
            single_utxo_funding_sat: quote.single_utxo_funding_sat,
            per_branch_funding,
            fee_rate_sat_per_vbyte: request.fee_rate_sat_per_vbyte,
            destination: request.destination,
        })
    }

    /// Builds and signs a complete unilateral exit from a `prepare_unilateral_exit`
    /// quote and the actual funding UTXOs, returning the full transaction set in
    /// topological broadcast order without broadcasting. Broadcast it over time,
    /// respecting each transaction's `depends_on` and `csv_timelock_blocks`.
    ///
    /// It resolves on-chain state first (see [`resolve_exit_observations`]): an
    /// already-confirmed fan-out or CPFP node is not rebuilt, and a leaf refund
    /// already on-chain (recognized by the leaf's refund address, so any refund
    /// variant counts) is swept directly. Re-running after partial progress
    /// therefore resumes rather than restarts.
    #[allow(clippy::too_many_lines)]
    pub async fn unilateral_exit(
        &self,
        request: UnilateralExitRequest,
        signer: Arc<dyn CpfpSigner>,
    ) -> Result<UnilateralExitResponse, SdkError> {
        let UnilateralExitRequest {
            prepared,
            funding_inputs,
        } = request;
        debug!(
            leaves = prepared.leaves.len(),
            funding_inputs = funding_inputs.len(),
            fee_rate_sat_per_vbyte = prepared.fee_rate_sat_per_vbyte,
            "unilateral_exit: building"
        );
        let btc_network: bitcoin::Network = self.config.network.into();
        let chain = self.chain_service.as_ref();

        let destination = prepared
            .destination
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|e| SdkError::InvalidInput(format!("Invalid destination address: {e}")))?
            .require_network(btc_network)
            .map_err(|e| SdkError::InvalidInput(format!("Address network mismatch: {e}")))?;
        let dest_script_len = destination.script_pubkey().len();

        // The build never re-selects: the quote's leaves are an explicit set.
        let leaf_ids = prepared
            .leaves
            .iter()
            .map(|l| {
                TreeNodeId::from_str(&l.leaf_id).map_err(|e| {
                    SdkError::InvalidInput(format!("Invalid leaf id {}: {e}", l.leaf_id))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if leaf_ids.is_empty() {
            // An empty quote builds to an empty result rather than erroring.
            debug!("unilateral_exit: quote has no leaves, returning empty result");
            return Ok(empty_exit_response());
        }

        let funding_inputs = funding_inputs
            .into_iter()
            .map(|i| i.into_funding_input(btc_network))
            .collect::<Result<Vec<_>, SdkError>>()?;
        if funding_inputs.is_empty() {
            return Err(SdkError::InvalidInput(
                "At least one funding input is required".to_string(),
            ));
        }

        let fee_rate_sat_per_kw = sat_per_kw_from_vbyte(prepared.fee_rate_sat_per_vbyte);
        let prepared_exit = self
            .spark_wallet
            .prepare_unilateral_exit_plan(
                fee_rate_sat_per_kw,
                spark_wallet::ExitLeafSelection::Specific(leaf_ids),
                funding_inputs,
                dest_script_len,
            )
            .await?;
        if prepared_exit.plan.selected_leaves.is_empty() {
            debug!("unilateral_exit: plan selected no leaves, returning empty result");
            return Ok(empty_exit_response());
        }
        trace!(
            selected_leaves = prepared_exit.plan.selected_leaves.len(),
            tree_nodes = prepared_exit.plan.tree_nodes.len(),
            has_fan_out = prepared_exit.plan.fan_out_psbt.is_some(),
            "unilateral_exit: plan prepared"
        );

        let leaves: Vec<UnilateralExitLeaf> = prepared_exit
            .plan
            .selected_leaves
            .iter()
            .map(|l| UnilateralExitLeaf {
                leaf_id: l.id.to_string(),
                value: l.value,
            })
            .collect();

        let observed = resolve_exit_observations(chain, &prepared_exit).await?;
        let build = build_unilateral_exit(&prepared_exit, &observed, fee_rate_sat_per_kw)?;
        let recoverable_value_sat = build.recoverable_value_sat;
        let build_fee_sat = build.total_fee_sat;
        // Captured before the loop below consumes `build.branches`.
        let sweep_status = sweep_confirmation_status(&build);
        debug!(
            has_fan_out = build.fan_out.is_some(),
            branches = build.branches.len(),
            refund_outputs = build.refund_outputs.len(),
            cpfp_change_inputs = build.cpfp_change_inputs.len(),
            recoverable_value_sat,
            build_fee_sat,
            "unilateral_exit: build assembled, signing"
        );

        let mut transactions: Vec<UnilateralExitTransaction> = Vec::new();

        if let Some(fan_out) = build.fan_out {
            trace!(
                txid = %fan_out.txid,
                status = ?fan_out.status,
                needs_signing = fan_out.to_sign.is_some(),
                "unilateral_exit: fan-out"
            );
            let tx_hex = match fan_out.to_sign {
                Some(psbt) => sign_psbt_via(psbt, signer.as_ref()).await?,
                None => serialize_hex(&fan_out.base_tx),
            };
            transactions.push(UnilateralExitTransaction {
                kind: UnilateralExitTxKind::FanOut,
                node_id: None,
                txid: fan_out.txid.to_string(),
                tx_hex,
                cpfp_tx_hex: None,
                csv_timelock_blocks: fan_out.csv_timelock_blocks,
                depends_on: fan_out.depends_on.iter().map(ToString::to_string).collect(),
                status: confirmation_status(fan_out.status),
            });
        }

        for branch in build.branches {
            trace!(leaf_id = %branch.leaf_id, txs = branch.txs.len(), "unilateral_exit: branch");
            for tx in branch.txs {
                let kind = match tx.kind {
                    ExitTxKind::Node => UnilateralExitTxKind::Node,
                    ExitTxKind::Refund => UnilateralExitTxKind::Refund,
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
                    "unilateral_exit: exit tx"
                );
                let cpfp_tx_hex = match tx.to_sign {
                    Some(child) => Some(sign_psbt_via(child, signer.as_ref()).await?),
                    None => None,
                };
                transactions.push(UnilateralExitTransaction {
                    kind,
                    node_id: tx.node_id.map(|id| id.to_string()),
                    txid: tx.txid.to_string(),
                    tx_hex: serialize_hex(&tx.base_tx),
                    cpfp_tx_hex,
                    csv_timelock_blocks: tx.csv_timelock_blocks,
                    depends_on: tx.depends_on.iter().map(ToString::to_string).collect(),
                    status: confirmation_status(tx.status),
                });
            }
        }

        // A sweep over zero inputs would error: return without one when no refund
        // is on-chain yet. A later run sweeps any refund that surfaces.
        if build.refund_outputs.is_empty() {
            debug!("unilateral_exit: no refund outputs to sweep, omitting the sweep");
            return Ok(UnilateralExitResponse {
                recoverable_value_sat,
                total_fee_sat: build_fee_sat,
                leaves,
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
                destination,
                fee_rate_sat_per_kw,
            )
            .await?;
        let actual_sweep_fee = sweep_fee(&sweep_psbt);
        let total_fee_sat = build_fee_sat.saturating_add(actual_sweep_fee);
        let sweep_txid = sweep_psbt.unsigned_tx.compute_txid();
        trace!(
            txid = %sweep_txid,
            status = ?sweep_status,
            refund_inputs = refund_txids.len(),
            "unilateral_exit: sweep"
        );
        let sweep_tx_hex = finalize_sweep(sweep_psbt, signer.as_ref()).await?;
        transactions.push(UnilateralExitTransaction {
            kind: UnilateralExitTxKind::Sweep,
            node_id: None,
            txid: sweep_txid.to_string(),
            tx_hex: sweep_tx_hex,
            cpfp_tx_hex: None,
            csv_timelock_blocks: None,
            depends_on: refund_txids,
            status: sweep_status,
        });

        debug!(
            transactions = transactions.len(),
            recoverable_value_sat, total_fee_sat, "unilateral_exit: complete"
        );
        Ok(UnilateralExitResponse {
            recoverable_value_sat,
            total_fee_sat,
            leaves,
            transactions,
        })
    }
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
                value,
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
                value,
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
                value,
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

/// Drives the wallet's pure resolver to completion: it reports which chain
/// lookups the exit needs, core performs them, and the results are fed back until
/// nothing more is needed. Core never interprets the exit tree itself.
async fn resolve_exit_observations(
    chain: &dyn BitcoinChainService,
    prepared: &PreparedUnilateralExit,
) -> Result<Vec<Observation>, SdkError> {
    let mut observed: Vec<Observation> = Vec::new();
    let mut round = 0u32;
    loop {
        let queries = next_chain_queries(prepared, &observed)?;
        if queries.is_empty() {
            break;
        }
        round = round.saturating_add(1);
        trace!(
            round,
            queries = queries.len(),
            "resolve_exit_observations: round"
        );
        // Each query is answered exactly once (a failed lookup records
        // `Unavailable`), so the loop always progresses and terminates.
        for query in queries {
            let result = execute_chain_query(chain, &query).await;
            observed.push(Observation { query, result });
        }
    }
    debug!(
        rounds = round,
        observations = observed.len(),
        "resolve_exit_observations: on-chain state resolved"
    );
    Ok(observed)
}

/// Performs one [`ChainQuery`], translating this crate's chain types into the
/// wallet's `bitcoin`-only [`ChainResult`]. A failed lookup becomes
/// [`ChainResult::Unavailable`] so the wallet flags the affected tx as unverified
/// rather than treating it as confirmed or absent.
async fn execute_chain_query(chain: &dyn BitcoinChainService, query: &ChainQuery) -> ChainResult {
    match query {
        ChainQuery::Outspend(outpoint) => {
            match chain
                .get_outspend(outpoint.txid.to_string(), outpoint.vout)
                .await
            {
                Ok(Outspend::Spent { txid, status, .. }) => match Txid::from_str(&txid) {
                    Ok(spender_txid) => {
                        trace!(%outpoint, spender = %spender_txid, confirmed = status.confirmed, "chain: outpoint spent");
                        ChainResult::Spend(Some(SpendInfo {
                            spender_txid,
                            confirmed: status.confirmed,
                        }))
                    }
                    Err(e) => {
                        warn!("outspend of {outpoint} has an unparsable spender txid {txid}: {e}");
                        ChainResult::Unavailable
                    }
                },
                Ok(Outspend::Unspent) => {
                    trace!(%outpoint, "chain: outpoint unspent");
                    ChainResult::Spend(None)
                }
                Err(e) => {
                    warn!("get_outspend for {outpoint} failed: {e}");
                    ChainResult::Unavailable
                }
            }
        }
        ChainQuery::Transaction(txid) => match chain.get_transaction_hex(txid.to_string()).await {
            Ok(hex) => match deserialize_hex::<Transaction>(&hex) {
                Ok(tx) => {
                    trace!(%txid, outputs = tx.output.len(), "chain: transaction fetched");
                    ChainResult::Transaction(tx)
                }
                Err(e) => {
                    warn!("failed to decode transaction {txid}: {e}");
                    ChainResult::Unavailable
                }
            },
            Err(e) => {
                warn!("get_transaction_hex for {txid} failed: {e}");
                ChainResult::Unavailable
            }
        },
        ChainQuery::RefundAddress { leaf_id, address } => {
            match chain.get_address_utxos(address.to_string()).await {
                Ok(utxos) => {
                    let utxos: Vec<AddressUtxo> = utxos
                        .into_iter()
                        .filter_map(|u| match Txid::from_str(&u.txid) {
                            Ok(txid) => Some(AddressUtxo {
                                txid,
                                vout: u.vout,
                                value: u.value,
                                confirmed: u.status.confirmed,
                            }),
                            Err(e) => {
                                warn!("skipping refund utxo {} for leaf {leaf_id}: {e}", u.txid);
                                None
                            }
                        })
                        .collect();
                    trace!(
                        %leaf_id,
                        utxos = utxos.len(),
                        confirmed = utxos.iter().filter(|u| u.confirmed).count(),
                        "chain: refund address scanned"
                    );
                    ChainResult::AddressUtxos(utxos)
                }
                Err(e) => {
                    warn!("get_address_utxos for leaf {leaf_id} failed: {e}");
                    ChainResult::Unavailable
                }
            }
        }
        ChainQuery::RefundAddressFunded { leaf_id, address } => {
            match chain
                .get_address_funded_txo_count(address.to_string())
                .await
            {
                Ok(count) => {
                    trace!(%leaf_id, count, "chain: refund address funded count");
                    ChainResult::AddressFunded(count)
                }
                Err(e) => {
                    warn!("get_address_funded_txo_count for leaf {leaf_id} failed: {e}");
                    ChainResult::Unavailable
                }
            }
        }
    }
}

fn confirmation_status(status: ExitTxStatus) -> ConfirmationStatus {
    match status {
        ExitTxStatus::Confirmed => ConfirmationStatus::Confirmed,
        ExitTxStatus::Unconfirmed => ConfirmationStatus::Unconfirmed,
        ExitTxStatus::Unverified => ConfirmationStatus::Unverified,
    }
}

/// The sweep's status, derived from the refunds it spends. A verified refund is
/// spent-and-dropped once its sweep confirms (the exit then returns with no
/// sweep), so a freshly-returned sweep over verified refunds is never yet
/// on-chain: `Unconfirmed`. An unverified refund (its chain lookup failed) could
/// already be on-chain and swept without us knowing, so the sweep is `Unverified`.
fn sweep_confirmation_status(build: &UnilateralExitBuild) -> ConfirmationStatus {
    let any_refund_unverified = build
        .branches
        .iter()
        .flat_map(|b| b.txs.iter())
        .any(|t| t.kind == ExitTxKind::Refund && t.status == ExitTxStatus::Unverified);
    if any_refund_unverified {
        ConfirmationStatus::Unverified
    } else {
        ConfirmationStatus::Unconfirmed
    }
}

fn empty_exit_response() -> UnilateralExitResponse {
    UnilateralExitResponse {
        recoverable_value_sat: 0,
        total_fee_sat: 0,
        leaves: Vec::new(),
        transactions: Vec::new(),
    }
}

/// Signs a PSBT via the external `CpfpSigner`, returning the tx as hex.
/// Ephemeral anchor inputs are finalized here (`OP_TRUE`, no signature).
async fn sign_psbt_via(
    mut psbt: bitcoin::Psbt,
    signer: &dyn CpfpSigner,
) -> Result<String, SdkError> {
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
async fn finalize_sweep(psbt: bitcoin::Psbt, signer: &dyn CpfpSigner) -> Result<String, SdkError> {
    let needs_signer = psbt
        .inputs
        .iter()
        .any(|input| input.final_script_witness.is_none());
    let psbt = if needs_signer {
        let out_bytes = signer
            .sign_psbt(psbt.serialize())
            .await
            .map_err(|e| SdkError::Signer(format!("Sweep signer error: {e}")))?;
        bitcoin::Psbt::deserialize(&out_bytes)
            .map_err(|e| SdkError::Generic(format!("Failed to deserialize signed sweep PSBT: {e}")))?
    } else {
        psbt
    };
    ensure_all_inputs_finalized(&psbt)?;
    Ok(serialize_hex(&psbt.extract_tx_unchecked_fee_rate()))
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
    use crate::error::SignerError;
    use bitcoin::hashes::Hash;
    use spark_wallet::{ExitBranch, ExitTx};

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
            total_fee_sat: 0,
        }
    }

    #[test]
    fn sweep_status_is_unconfirmed_when_refunds_are_verified() {
        assert_eq!(
            sweep_confirmation_status(&build_with_refund(ExitTxStatus::Unconfirmed)),
            ConfirmationStatus::Unconfirmed
        );
    }

    #[test]
    fn sweep_status_is_unverified_when_a_refund_is_unverified() {
        assert_eq!(
            sweep_confirmation_status(&build_with_refund(ExitTxStatus::Unverified)),
            ConfirmationStatus::Unverified
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
        let result = sign_psbt_via(unsigned_two_input_psbt(), &PartialSigner { finalize: 1 }).await;
        assert!(matches!(result, Err(SdkError::Signer(_))));
    }

    #[macros::async_test_all]
    async fn sign_psbt_via_succeeds_when_every_input_is_signed() {
        let result = sign_psbt_via(unsigned_two_input_psbt(), &PartialSigner { finalize: 2 }).await;
        assert!(result.is_ok());
    }
}
