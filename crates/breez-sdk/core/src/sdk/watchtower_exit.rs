use std::{collections::HashMap, fmt::Display, str::FromStr};

use bitcoin::{
    Address, Amount, ScriptBuf, Transaction, TxOut, Txid,
    address::NetworkUnchecked,
    consensus::encode::{deserialize_hex, serialize_hex},
    hex::{DisplayHex, FromHex},
};
use spark_wallet::{
    MIN_RELAY_FEE_SAT_PER_VBYTE, TreeNode, TreeNodeStatus, UnsignedWatchtowerExitRecovery,
    WatchtowerExitedOutput, build_watchtower_exit_recovery,
};
use tracing::{error, info, warn};

use crate::{
    Network, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType,
    PrepareRecoverWatchtowerExitedFundsRequest, PrepareRecoverWatchtowerExitedFundsResponse,
    RecoverWatchtowerExitedFundsRequest, RecoverWatchtowerExitedFundsResponse, SdkEvent,
    WatchtowerExitRecoveryError, WatchtowerExitRecoveryFailure, WatchtowerExitRecoveryInfo,
    WatchtowerExitRecoveryQuote, WatchtowerExitRecoveryState, WatchtowerExitRecoverySuccess,
    WatchtowerExitedFundsInfo,
    chain::{Outspend, TxStatus},
    error::SdkError,
    models::Payment,
    persist::{CachedWatchtowerExit, CachedWatchtowerExitRecovery, ObjectCacheRepository},
    utils::{deposit_chain_syncer::already_on_network, time::now_secs},
};

use super::{
    BreezSdk,
    deposits::{PendingRefund, refund_fee_sats, replacement_min_fee_sats},
};

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Reads local state only. Leaves out outputs whose recovery has confirmed,
    /// and outputs too small to pay the fee.
    pub async fn prepare_recover_watchtower_exited_funds(
        &self,
        request: PrepareRecoverWatchtowerExitedFundsRequest,
    ) -> Result<PrepareRecoverWatchtowerExitedFundsResponse, SdkError> {
        let destination = self.recovery_destination(&request.destination)?;
        check_fee_rate(request.fee_rate_sat_per_vbyte)?;

        let mut quotes = Vec::new();
        for exit in self.watchtower_exits().await? {
            if !exit.is_recoverable() {
                continue;
            }
            let recovery = build_recovery(
                &exit.output()?,
                &destination,
                request.fee_rate_sat_per_vbyte,
            )?;
            if let Some(recovery) = recovery {
                quotes.push(exit.quote(recovery.fee_sat));
            }
        }
        Ok(PrepareRecoverWatchtowerExitedFundsResponse {
            total_amount_sat: quotes.iter().map(|quote| quote.amount_sat).sum(),
            total_fee_sat: quotes.iter().map(|quote| quote.fee_sat).sum(),
            destination: request.destination,
            fee_rate_sat_per_vbyte: request.fee_rate_sat_per_vbyte,
            quotes,
        })
    }

    /// The Spark operators co-sign every recovery, so they have to be reachable.
    pub async fn recover_watchtower_exited_funds(
        &self,
        request: RecoverWatchtowerExitedFundsRequest,
    ) -> Result<RecoverWatchtowerExitedFundsResponse, SdkError> {
        let prepared = request.prepare_response;
        let destination = self.recovery_destination(&prepared.destination)?;
        check_fee_rate(prepared.fee_rate_sat_per_vbyte)?;

        let _lock = self.watchtower_exit_lock.lock().await;
        let mut exits: HashMap<String, CachedWatchtowerExit> = self
            .watchtower_exits()
            .await?
            .into_iter()
            .map(|exit| (exit.leaf_id.clone(), exit))
            .collect();

        let mut recovered = Vec::new();
        let mut failed = Vec::new();
        for quote in prepared.quotes {
            let exit = exits
                .remove(&quote.leaf_id)
                .filter(|exit| exit.txid == quote.txid && exit.vout == quote.vout);
            let result = match exit {
                Some(exit) => {
                    self.recover_watchtower_exit(
                        exit,
                        &destination,
                        prepared.fee_rate_sat_per_vbyte,
                    )
                    .await
                }
                None => Err(generic(
                    "No watchtower-exited funds of this wallet are in this output",
                )),
            };
            match result {
                Ok(recovery) => recovered.push(WatchtowerExitRecoverySuccess {
                    leaf_id: quote.leaf_id,
                    txid: quote.txid,
                    vout: quote.vout,
                    recovery,
                }),
                Err(error) => {
                    warn!(
                        "Failed to recover watchtower-exited funds {}:{}: {error}",
                        quote.txid, quote.vout
                    );
                    failed.push(WatchtowerExitRecoveryFailure {
                        leaf_id: quote.leaf_id,
                        txid: quote.txid,
                        vout: quote.vout,
                        error,
                    });
                }
            }
        }
        Ok(RecoverWatchtowerExitedFundsResponse { recovered, failed })
    }
}

impl BreezSdk {
    pub(crate) async fn watchtower_exited_sats(&self) -> Result<u64, SdkError> {
        Ok(self
            .watchtower_exits()
            .await?
            .iter()
            .filter(|exit| exit.recovery.is_none())
            .map(|exit| exit.amount_sats)
            .sum())
    }

    pub(crate) async fn sync_watchtower_exited_funds(&self) -> Result<(), SdkError> {
        let leaves = self.spark_wallet.list_watchtower_exited_leaves().await?;
        if leaves.is_empty() {
            return Ok(());
        }
        let _lock = self.watchtower_exit_lock.lock().await;
        let repository = ObjectCacheRepository::new(self.storage.clone());

        let mut exits = Vec::with_capacity(leaves.len());
        let mut unresolved: Vec<TreeNode> = Vec::new();
        for leaf in &leaves {
            match repository
                .fetch_watchtower_exit(&leaf.id.to_string())
                .await?
            {
                Some(exit) => exits.push((leaf.status, exit)),
                None => unresolved.push(leaf.clone()),
            }
        }

        let mut new_funds = Vec::new();
        for output in self
            .spark_wallet
            .resolve_watchtower_exited_outputs(&unresolved)
            .await?
        {
            let exit = CachedWatchtowerExit::from_output(&output);
            repository.save_watchtower_exit(&exit).await?;
            new_funds.push(exit.info());
            let status = unresolved
                .iter()
                .find(|leaf| leaf.id == output.leaf_id)
                .map_or(TreeNodeStatus::WatchtowerExited, |leaf| leaf.status);
            exits.push((status, exit));
        }
        if !new_funds.is_empty() {
            info!("Found {} watchtower-exited outputs", new_funds.len());
            self.event_emitter
                .emit(&SdkEvent::NewWatchtowerExitedFunds {
                    new_watchtower_exited_funds: new_funds,
                })
                .await;
        }

        // Only a signed recovery spends these outputs, and the operators mark
        // every leaf they sign one for as recovered, so the rest need no lookup.
        let to_follow: Vec<CachedWatchtowerExit> = exits
            .into_iter()
            .filter(|(status, exit)| {
                !exit.is_final()
                    && (exit.recovery.is_some()
                        || *status == TreeNodeStatus::WatchtowerExitRecovered)
            })
            .map(|(_, exit)| exit)
            .collect();
        if to_follow.is_empty() {
            return Ok(());
        }
        let tip_height = self.chain_service.tip_height().await?;
        for exit in to_follow {
            if let Err(e) = self.follow_recovery(exit, tip_height, &repository).await {
                error!("Failed to follow a watchtower exit recovery: {e}");
            }
        }
        Ok(())
    }

    async fn watchtower_exits(&self) -> Result<Vec<CachedWatchtowerExit>, SdkError> {
        let repository = ObjectCacheRepository::new(self.storage.clone());
        let mut exits = Vec::new();
        for leaf in self.spark_wallet.list_watchtower_exited_leaves().await? {
            if let Some(exit) = repository
                .fetch_watchtower_exit(&leaf.id.to_string())
                .await?
            {
                exits.push(exit);
            }
        }
        Ok(exits)
    }

    fn recovery_destination(&self, destination: &str) -> Result<Address, SdkError> {
        destination
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|e| SdkError::InvalidInput(format!("Invalid destination address: {e}")))?
            .require_network(self.config.network.into())
            .map_err(|e| SdkError::InvalidInput(format!("Address network mismatch: {e}")))
    }

    async fn recover_watchtower_exit(
        &self,
        mut exit: CachedWatchtowerExit,
        destination: &Address,
        fee_rate_sat_per_vbyte: u64,
    ) -> Result<WatchtowerExitRecoveryInfo, WatchtowerExitRecoveryError> {
        if !exit.is_recoverable() {
            return Err(generic("A recovery of these funds has already confirmed"));
        }
        let output = exit.output().map_err(generic)?;
        let recovery = build_recovery(&output, destination, fee_rate_sat_per_vbyte)
            .map_err(generic)?
            .ok_or_else(|| generic("The output is too small to pay this fee"))?;
        let pending = exit.recovery_to_outbid();
        if let Some(pending) = &pending {
            check_outbids(pending, &recovery)?;
        }

        let tx = self
            .spark_wallet
            .cosign_watchtower_exit_recovery(&output, recovery.tx)
            .await
            .map_err(|e| {
                if e.is_operator_unavailable() {
                    WatchtowerExitRecoveryError::OperatorsUnavailable {
                        message: e.to_string(),
                    }
                } else {
                    generic(e)
                }
            })?;

        let tx_hex = serialize_hex(&tx);
        let state = match self
            .chain_service
            .broadcast_transaction(tx_hex.clone())
            .await
        {
            Ok(()) => WatchtowerExitRecoveryState::Broadcast,
            Err(e) if already_on_network(&e.to_string()) => WatchtowerExitRecoveryState::Broadcast,
            // A refused replacement leaves the pending recovery in place.
            Err(e) if pending.is_some() => return Err(generic(e)),
            Err(e) => WatchtowerExitRecoveryState::BroadcastPending {
                last_error: Some(e.to_string()),
            },
        };
        let created_at = exit
            .recovery
            .as_ref()
            .map_or_else(now_secs, |recovery| recovery.created_at);
        exit.recovery = Some(CachedWatchtowerExitRecovery {
            tx_id: tx.compute_txid().to_string(),
            tx_hex,
            state,
            created_at,
            is_confirmed: false,
            is_final: false,
        });
        ObjectCacheRepository::new(self.storage.clone())
            .save_watchtower_exit(&exit)
            .await
            .map_err(generic)?;
        self.record_recovery_payment(&exit).await;
        exit.recovery
            .as_ref()
            .map(CachedWatchtowerExitRecovery::info)
            .ok_or_else(|| generic("The recovery was not recorded"))
    }

    async fn follow_recovery(
        &self,
        mut exit: CachedWatchtowerExit,
        tip_height: u32,
        repository: &ObjectCacheRepository,
    ) -> Result<(), SdkError> {
        let own_status = match &exit.recovery {
            Some(recovery) => self
                .chain_service
                .get_transaction_status(recovery.tx_id.clone())
                .await
                .ok(),
            None => None,
        };
        let outspend = match own_status {
            Some(_) => None,
            None => match self
                .chain_service
                .get_outspend(exit.txid.clone(), exit.vout)
                .await
            {
                Ok(outspend) => Some(outspend),
                Err(e) => {
                    warn!(
                        "Outspend lookup failed for watchtower-exited output {}:{}: {e}",
                        exit.txid, exit.vout
                    );
                    return Ok(());
                }
            },
        };
        let own_tx_id = exit
            .recovery
            .as_ref()
            .map(|recovery| recovery.tx_id.as_str());

        match recovery_action(own_tx_id, own_status.as_ref(), outspend.as_ref()) {
            RecoveryAction::None => return Ok(()),
            RecoveryAction::Seen { confirmation } => {
                let final_confirmations = final_confirmations(self.config.network);
                if let Some(recovery) = exit.recovery.as_mut() {
                    recovery.state = WatchtowerExitRecoveryState::Broadcast;
                    recovery.is_confirmed = confirmation.is_confirmed();
                    recovery.is_final = is_final(&confirmation, tip_height, final_confirmations);
                }
            }
            RecoveryAction::Adopt {
                tx_id,
                confirmation,
            } => {
                let tx_hex = self
                    .chain_service
                    .get_transaction_hex(tx_id.clone())
                    .await?;
                let created_at = exit
                    .recovery
                    .as_ref()
                    .map_or_else(now_secs, |recovery| recovery.created_at);
                exit.recovery = Some(CachedWatchtowerExitRecovery {
                    tx_id,
                    tx_hex,
                    state: WatchtowerExitRecoveryState::Broadcast,
                    created_at,
                    is_confirmed: confirmation.is_confirmed(),
                    is_final: is_final(
                        &confirmation,
                        tip_height,
                        final_confirmations(self.config.network),
                    ),
                });
            }
            RecoveryAction::Rebroadcast => {
                let Some(recovery) = exit.recovery.as_mut() else {
                    return Ok(());
                };
                recovery.is_confirmed = false;
                recovery.state = match self
                    .chain_service
                    .broadcast_transaction(recovery.tx_hex.clone())
                    .await
                {
                    Ok(()) => WatchtowerExitRecoveryState::Broadcast,
                    Err(e) if already_on_network(&e.to_string()) => {
                        WatchtowerExitRecoveryState::Broadcast
                    }
                    Err(e) => WatchtowerExitRecoveryState::BroadcastPending {
                        last_error: Some(e.to_string()),
                    },
                };
            }
        }
        repository.save_watchtower_exit(&exit).await?;
        self.record_recovery_payment(&exit).await;
        Ok(())
    }

    /// Failures are only logged: the next sync writes the payment again.
    async fn record_recovery_payment(&self, exit: &CachedWatchtowerExit) {
        let Some(payment) = exit.payment() else {
            return;
        };
        match self.storage.apply_payment_update(payment.clone()).await {
            Ok(true) => {
                self.event_emitter
                    .emit(&SdkEvent::from_payment(payment))
                    .await;
            }
            Ok(false) => {}
            Err(e) => error!(
                "Failed to store the payment of recovery {}: {e}",
                payment.id
            ),
        }
    }
}

/// A reorg deeper than this is not followed: a completed payment cannot go
/// back to pending.
fn final_confirmations(network: Network) -> u32 {
    match network {
        Network::Regtest => 1,
        Network::Mainnet | Network::Signet => 6,
    }
}

/// A confirmation without a height counts as final: its depth cannot be measured.
fn is_final(confirmation: &Confirmation, tip_height: u32, final_confirmations: u32) -> bool {
    match confirmation {
        Confirmation::Unconfirmed => false,
        Confirmation::Confirmed { height: None } => true,
        Confirmation::Confirmed {
            height: Some(height),
        } => tip_height.saturating_sub(*height).saturating_add(1) >= final_confirmations,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Confirmation {
    Unconfirmed,
    Confirmed { height: Option<u32> },
}

impl Confirmation {
    fn is_confirmed(&self) -> bool {
        matches!(self, Confirmation::Confirmed { .. })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RecoveryAction {
    None,
    Seen {
        confirmation: Confirmation,
    },
    Adopt {
        tx_id: String,
        confirmation: Confirmation,
    },
    Rebroadcast,
}

fn recovery_action(
    own_tx_id: Option<&str>,
    own_status: Option<&TxStatus>,
    outspend: Option<&Outspend>,
) -> RecoveryAction {
    if let Some(status) = own_status {
        return RecoveryAction::Seen {
            confirmation: confirmation(status),
        };
    }
    match outspend {
        Some(Outspend::Spent { txid, status, .. }) if Some(txid.as_str()) == own_tx_id => {
            RecoveryAction::Seen {
                confirmation: confirmation(status),
            }
        }
        Some(Outspend::Spent { txid, status, .. }) => RecoveryAction::Adopt {
            tx_id: txid.clone(),
            confirmation: confirmation(status),
        },
        Some(Outspend::Unspent) if own_tx_id.is_some() => RecoveryAction::Rebroadcast,
        Some(Outspend::Unspent) | None => RecoveryAction::None,
    }
}

fn confirmation(status: &TxStatus) -> Confirmation {
    if status.confirmed {
        Confirmation::Confirmed {
            height: status.block_height,
        }
    } else {
        Confirmation::Unconfirmed
    }
}

fn check_fee_rate(fee_rate_sat_per_vbyte: u64) -> Result<(), SdkError> {
    if fee_rate_sat_per_vbyte < MIN_RELAY_FEE_SAT_PER_VBYTE {
        return Err(SdkError::InvalidInput(format!(
            "Fee rate must be at least {MIN_RELAY_FEE_SAT_PER_VBYTE} sat/vbyte"
        )));
    }
    Ok(())
}

fn build_recovery(
    output: &WatchtowerExitedOutput,
    destination: &Address,
    fee_rate_sat_per_vbyte: u64,
) -> Result<Option<UnsignedWatchtowerExitRecovery>, SdkError> {
    Ok(build_watchtower_exit_recovery(
        output,
        destination,
        spark_wallet::Fee::Rate {
            sat_per_vbyte: fee_rate_sat_per_vbyte,
        },
    )?)
}

fn check_outbids(
    pending: &PendingRefund,
    recovery: &UnsignedWatchtowerExitRecovery,
) -> Result<(), WatchtowerExitRecoveryError> {
    let required_fee_sat = replacement_min_fee_sats(pending, recovery.vsize);
    if recovery.fee_sat >= required_fee_sat {
        return Ok(());
    }
    Err(WatchtowerExitRecoveryError::ReplacementFeeTooLow {
        required_fee_sat,
        required_fee_rate_sat_per_vbyte: required_fee_sat.div_ceil(recovery.vsize),
    })
}

fn generic(error: impl Display) -> WatchtowerExitRecoveryError {
    WatchtowerExitRecoveryError::Generic {
        message: error.to_string(),
    }
}

fn payment_id(leaf_id: &str) -> String {
    format!("watchtower-exit-recovery:{leaf_id}")
}

impl CachedWatchtowerExit {
    fn from_output(output: &WatchtowerExitedOutput) -> Self {
        Self {
            leaf_id: output.leaf_id.to_string(),
            leaf_value: output.leaf_value,
            txid: output.outpoint.txid.to_string(),
            vout: output.outpoint.vout,
            amount_sats: output.tx_out.value.to_sat(),
            script_pubkey: output.tx_out.script_pubkey.as_bytes().to_lower_hex_string(),
            recovery: None,
        }
    }

    fn output(&self) -> Result<WatchtowerExitedOutput, SdkError> {
        let txid = Txid::from_str(&self.txid)
            .map_err(|e| SdkError::Generic(format!("invalid stored txid: {e}")))?;
        let script_pubkey = Vec::<u8>::from_hex(&self.script_pubkey)
            .map_err(|e| SdkError::Generic(format!("invalid stored script: {e}")))?;
        Ok(WatchtowerExitedOutput {
            leaf_id: self
                .leaf_id
                .parse()
                .map_err(|e| SdkError::Generic(format!("invalid stored leaf id: {e}")))?,
            leaf_value: self.leaf_value,
            outpoint: bitcoin::OutPoint {
                txid,
                vout: self.vout,
            },
            tx_out: TxOut {
                value: Amount::from_sat(self.amount_sats),
                script_pubkey: ScriptBuf::from_bytes(script_pubkey),
            },
        })
    }

    fn is_final(&self) -> bool {
        self.recovery
            .as_ref()
            .is_some_and(|recovery| recovery.is_final)
    }

    fn is_recoverable(&self) -> bool {
        self.recovery
            .as_ref()
            .is_none_or(|recovery| !recovery.is_confirmed && !recovery.is_final)
    }

    fn recovery_to_outbid(&self) -> Option<PendingRefund> {
        let recovery = self.recovery.as_ref()?;
        if recovery.state != WatchtowerExitRecoveryState::Broadcast {
            return None;
        }
        let tx = deserialize_hex::<Transaction>(&recovery.tx_hex).ok()?;
        Some(PendingRefund {
            fee_sats: refund_fee_sats(&tx, self.amount_sats)?,
            vsize: tx.vsize().try_into().unwrap_or(u64::MAX),
        })
    }

    fn info(&self) -> WatchtowerExitedFundsInfo {
        WatchtowerExitedFundsInfo {
            leaf_id: self.leaf_id.clone(),
            txid: self.txid.clone(),
            vout: self.vout,
            amount_sat: self.amount_sats,
        }
    }

    fn quote(&self, fee_sat: u64) -> WatchtowerExitRecoveryQuote {
        WatchtowerExitRecoveryQuote {
            leaf_id: self.leaf_id.clone(),
            txid: self.txid.clone(),
            vout: self.vout,
            amount_sat: self.amount_sats,
            fee_sat,
            pending_recovery: self
                .recovery
                .as_ref()
                .map(CachedWatchtowerExitRecovery::info),
        }
    }

    /// `fees` is the rest of the leaf's value, including what the watchtower's
    /// transaction paid.
    fn payment(&self) -> Option<Payment> {
        let recovery = self.recovery.as_ref()?;
        let tx = deserialize_hex::<Transaction>(&recovery.tx_hex).ok()?;
        let received: u64 = tx.output.iter().map(|output| output.value.to_sat()).sum();
        Some(Payment {
            id: payment_id(&self.leaf_id),
            payment_type: PaymentType::Send,
            status: if recovery.is_final {
                PaymentStatus::Completed
            } else {
                PaymentStatus::Pending
            },
            amount: u128::from(received),
            fees: u128::from(self.leaf_value.saturating_sub(received)),
            timestamp: recovery.created_at,
            method: PaymentMethod::WatchtowerExitRecovery,
            details: Some(PaymentDetails::WatchtowerExitRecovery {
                tx_id: recovery.tx_id.clone(),
            }),
            conversion_details: None,
        })
    }
}

impl CachedWatchtowerExitRecovery {
    fn info(&self) -> WatchtowerExitRecoveryInfo {
        WatchtowerExitRecoveryInfo {
            tx_id: self.tx_id.clone(),
            tx_hex: self.tx_hex.clone(),
            state: self.state.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(confirmed: bool, height: Option<u32>) -> TxStatus {
        TxStatus {
            confirmed,
            block_height: height,
            block_time: None,
        }
    }

    fn spent_by(txid: &str, confirmed: bool, height: Option<u32>) -> Outspend {
        Outspend::Spent {
            txid: txid.to_string(),
            vin: 0,
            status: status(confirmed, height),
        }
    }

    #[test]
    fn a_stored_recovery_on_the_network_is_seen() {
        let action = recovery_action(Some("ours"), Some(&status(true, Some(100))), None);
        assert_eq!(
            action,
            RecoveryAction::Seen {
                confirmation: Confirmation::Confirmed { height: Some(100) }
            }
        );

        let action = recovery_action(Some("ours"), Some(&status(false, None)), None);
        assert_eq!(
            action,
            RecoveryAction::Seen {
                confirmation: Confirmation::Unconfirmed
            }
        );
    }

    #[test]
    fn a_spend_by_the_stored_recovery_is_seen() {
        let outspend = spent_by("ours", true, Some(100));
        let action = recovery_action(Some("ours"), None, Some(&outspend));
        assert_eq!(
            action,
            RecoveryAction::Seen {
                confirmation: Confirmation::Confirmed { height: Some(100) }
            }
        );
    }

    #[test]
    fn a_spend_by_another_recovery_is_adopted() {
        let outspend = spent_by("theirs", false, None);
        let action = recovery_action(Some("ours"), None, Some(&outspend));
        assert_eq!(
            action,
            RecoveryAction::Adopt {
                tx_id: "theirs".to_string(),
                confirmation: Confirmation::Unconfirmed
            }
        );

        let action = recovery_action(None, None, Some(&outspend));
        assert_eq!(
            action,
            RecoveryAction::Adopt {
                tx_id: "theirs".to_string(),
                confirmation: Confirmation::Unconfirmed
            }
        );
    }

    #[test]
    fn an_unspent_output_rebroadcasts_the_stored_recovery() {
        let action = recovery_action(Some("ours"), None, Some(&Outspend::Unspent));
        assert_eq!(action, RecoveryAction::Rebroadcast);
    }

    #[test]
    fn an_unspent_output_without_a_stored_recovery_changes_nothing() {
        assert_eq!(
            recovery_action(None, None, Some(&Outspend::Unspent)),
            RecoveryAction::None
        );
        assert_eq!(recovery_action(None, None, None), RecoveryAction::None);
    }

    #[test]
    fn a_recovery_is_final_once_buried_deep_enough() {
        let at = |height| Confirmation::Confirmed {
            height: Some(height),
        };
        assert!(!is_final(&Confirmation::Unconfirmed, 110, 6));
        assert!(!is_final(&at(106), 110, 6));
        assert!(is_final(&at(105), 110, 6));
        assert!(is_final(&at(110), 110, 1));
    }

    #[test]
    fn a_confirmation_without_a_height_is_final() {
        assert!(is_final(&Confirmation::Confirmed { height: None }, 110, 6));
    }

    fn exit_with(recovery: Option<CachedWatchtowerExitRecovery>) -> CachedWatchtowerExit {
        CachedWatchtowerExit {
            leaf_id: "leaf".to_string(),
            leaf_value: 10_000,
            txid: "00".repeat(32),
            vout: 0,
            amount_sats: 9_800,
            script_pubkey: String::new(),
            recovery,
        }
    }

    fn recovery_paying(value: u64, is_final: bool) -> CachedWatchtowerExitRecovery {
        let tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn::default()],
            output: vec![TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        CachedWatchtowerExitRecovery {
            tx_id: tx.compute_txid().to_string(),
            tx_hex: serialize_hex(&tx),
            state: WatchtowerExitRecoveryState::Broadcast,
            created_at: 1_700_000_000,
            is_confirmed: is_final,
            is_final,
        }
    }

    #[test]
    fn no_payment_before_a_recovery_is_signed() {
        assert!(exit_with(None).payment().is_none());
    }

    #[test]
    fn a_recovery_payment_charges_everything_the_destination_did_not_get() {
        let payment = exit_with(Some(recovery_paying(9_500, false)))
            .payment()
            .unwrap();

        assert_eq!(payment.id, "watchtower-exit-recovery:leaf");
        assert_eq!(payment.payment_type, PaymentType::Send);
        assert_eq!(payment.status, PaymentStatus::Pending);
        assert_eq!(payment.amount, 9_500);
        assert_eq!(payment.fees, 500);
        assert_eq!(payment.method, PaymentMethod::WatchtowerExitRecovery);
        assert_eq!(payment.timestamp, 1_700_000_000);
    }

    #[test]
    fn a_final_recovery_completes_its_payment() {
        let payment = exit_with(Some(recovery_paying(9_500, true)))
            .payment()
            .unwrap();
        assert_eq!(payment.status, PaymentStatus::Completed);
    }

    #[test]
    fn only_a_broadcast_recovery_has_to_be_outbid() {
        let mut recovery = recovery_paying(9_500, false);
        let pending = exit_with(Some(recovery.clone()))
            .recovery_to_outbid()
            .unwrap();
        assert_eq!(pending.fee_sats, 300);

        recovery.state = WatchtowerExitRecoveryState::BroadcastPending { last_error: None };
        assert!(exit_with(Some(recovery)).recovery_to_outbid().is_none());
    }

    #[test]
    fn funds_stay_recoverable_until_a_recovery_confirms() {
        assert!(exit_with(None).is_recoverable());
        let mut recovery = recovery_paying(9_500, false);
        assert!(exit_with(Some(recovery.clone())).is_recoverable());

        recovery.is_confirmed = true;
        assert!(!exit_with(Some(recovery)).is_recoverable());
        assert!(!exit_with(Some(recovery_paying(9_500, true))).is_recoverable());
    }

    fn unsigned_paying(fee_sat: u64, vsize: u64) -> UnsignedWatchtowerExitRecovery {
        let recovery = recovery_paying(0, false);
        UnsignedWatchtowerExitRecovery {
            tx: deserialize_hex(&recovery.tx_hex).unwrap(),
            fee_sat,
            vsize,
        }
    }

    #[test]
    fn a_replacement_that_does_not_outbid_reports_what_would() {
        let pending = PendingRefund {
            fee_sats: 300,
            vsize: 111,
        };

        let Err(WatchtowerExitRecoveryError::ReplacementFeeTooLow {
            required_fee_sat,
            required_fee_rate_sat_per_vbyte,
        }) = check_outbids(&pending, &unsigned_paying(300, 111))
        else {
            panic!("a replacement paying the same fee does not outbid");
        };
        assert_eq!(required_fee_sat, replacement_min_fee_sats(&pending, 111));
        assert!(required_fee_rate_sat_per_vbyte * 111 >= required_fee_sat);
        assert!((required_fee_rate_sat_per_vbyte - 1) * 111 < required_fee_sat);

        assert_eq!(
            check_outbids(&pending, &unsigned_paying(required_fee_sat, 111)),
            Ok(())
        );
    }

    #[test]
    fn a_rate_below_the_relay_minimum_is_refused() {
        assert!(matches!(check_fee_rate(0), Err(SdkError::InvalidInput(_))));
        assert!(check_fee_rate(MIN_RELAY_FEE_SAT_PER_VBYTE).is_ok());
    }
}
