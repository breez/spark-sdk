use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use spark_wallet::SparkWallet;
use tracing::{error, info, warn};

use crate::{
    BitcoinChainService, DepositInfo, RefundState, SdkError,
    chain::{Outspend, TxStatus},
    persist::{Storage, UpdateDepositPayload},
    utils::utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
};

const UTXO_PAGE_SIZE: u32 = 100;

pub struct DepositChainSyncer {
    storage: Arc<dyn Storage>,
    spark_wallet: Arc<SparkWallet>,
    utxo_fetcher: CachedUtxoFetcher,
    chain_service: Arc<dyn BitcoinChainService>,
}

#[derive(Eq, Hash, PartialEq, Clone)]
pub(crate) struct TxOutput {
    pub txid: String,
    pub vout: u32,
}

impl DepositChainSyncer {
    pub fn new(
        chain_service: Arc<dyn BitcoinChainService>,
        storage: Arc<dyn Storage>,
        spark_wallet: Arc<SparkWallet>,
    ) -> Self {
        Self {
            storage: storage.clone(),
            spark_wallet,
            utxo_fetcher: CachedUtxoFetcher::new(chain_service.clone(), storage),
            chain_service,
        }
    }

    /// Returns a list of (`DetailedUtxo`, `is_mature`) pairs for all non-refunded deposit UTXOs.
    pub async fn sync(&self) -> Result<Vec<(DetailedUtxo, bool)>, SdkError> {
        info!("Syncing deposit UTXOs via identity");

        let mut detailed_utxos: HashMap<TxOutput, (DetailedUtxo, bool)> = HashMap::new();
        let mut cursor = None;
        let mut hit_error = false;

        // Process UTXOs page by page, fetching tx details sequentially.
        // On fetch errors we stop processing but still reconcile what succeeded.
        loop {
            let (utxos, next_cursor) = match self
                .spark_wallet
                .get_utxos_for_identity(UTXO_PAGE_SIZE, cursor)
                .await
            {
                Ok(result) => result,
                Err(e) => {
                    if detailed_utxos.is_empty() {
                        // Rebroadcast pending refunds before surfacing the error:
                        // a refund retry must not depend on the operator feed.
                        if let Err(e) = self.reconcile_deposits(&HashMap::new(), true).await {
                            warn!("Failed to reconcile deposits: {e}");
                        }
                        return Err(e.into());
                    }
                    warn!(
                        "Failed to fetch UTXOs page, processing {} fetched so far: {e}",
                        detailed_utxos.len()
                    );
                    hit_error = true;
                    break;
                }
            };

            for utxo in &utxos {
                let txid_str = utxo.txid.to_string();
                match self
                    .utxo_fetcher
                    .fetch_detailed_utxo(&txid_str, utxo.vout)
                    .await
                {
                    Ok(detailed_utxo) => {
                        self.storage
                            .add_deposit(
                                detailed_utxo.txid.to_string(),
                                detailed_utxo.vout,
                                detailed_utxo.value,
                                utxo.is_mature,
                            )
                            .await?;
                        let key = TxOutput {
                            txid: detailed_utxo.txid.to_string(),
                            vout: detailed_utxo.vout,
                        };
                        detailed_utxos.insert(key, (detailed_utxo, utxo.is_mature));
                    }
                    Err(e) => {
                        warn!(
                            "Failed to fetch utxo details, processing {} fetched so far: {e}",
                            detailed_utxos.len()
                        );
                        hit_error = true;
                        break;
                    }
                }
            }

            if hit_error || next_cursor.is_none() {
                break;
            }
            cursor = next_cursor;
        }

        let refunded = self.reconcile_deposits(&detailed_utxos, hit_error).await?;

        Ok(detailed_utxos
            .into_values()
            .filter(|(u, _)| {
                !refunded.contains(&TxOutput {
                    txid: u.txid.to_string(),
                    vout: u.vout,
                })
            })
            .collect())
    }

    /// Removes stale deposits and checks refund confirmations.
    /// Returns the set of refunded outputs.
    async fn reconcile_deposits(
        &self,
        all_utxos: &HashMap<TxOutput, (DetailedUtxo, bool)>,
        incomplete: bool,
    ) -> Result<HashSet<TxOutput>, SdkError> {
        let deposits = self.storage.list_deposits().await?;
        let mut refunded = HashSet::new();
        let mut refunded_deposits = Vec::new();
        for deposit in deposits {
            let key = TxOutput {
                txid: deposit.txid.clone(),
                vout: deposit.vout,
            };
            match deposit.refund_tx_id.clone() {
                Some(txid) => {
                    info!(
                        "Found refund transaction {}:{} deposit tx: {}",
                        txid, deposit.vout, deposit.txid
                    );
                    refunded.insert(key);
                    refunded_deposits.push(deposit);
                }
                None => {
                    if !incomplete && !all_utxos.contains_key(&key) {
                        self.storage
                            .delete_deposit(deposit.txid, deposit.vout)
                            .await?;
                    }
                }
            }
        }

        for deposit in &refunded_deposits {
            self.resolve_refunded_deposit(deposit).await;
        }

        Ok(refunded)
    }

    /// Drives one refunded deposit towards a settled refund: drops it once its
    /// output is spent for good, and rebroadcasts the stored refund while it is
    /// not. Failures are logged, never propagated, so one deposit cannot stop
    /// the rest of the sync.
    async fn resolve_refunded_deposit(&self, deposit: &DepositInfo) {
        info!(
            "Checking refund of deposit {}:{}",
            deposit.txid, deposit.vout
        );
        let Some(refund_txid) = deposit.refund_tx_id.clone() else {
            return;
        };
        let state = deposit.refund_state.as_ref();

        // The refund's own confirmation settles most cases.
        let status = self
            .chain_service
            .get_transaction_status(refund_txid.clone())
            .await;
        let mut outpoint_checked = true;
        let action = if let Some(action) = refund_action_from_status(status.as_ref().ok(), state) {
            action
        } else {
            // The refund is not on chain: it either never got out, or another
            // transaction took the deposit. Only the outpoint tells those apart.
            let outspend = self
                .chain_service
                .get_outspend(deposit.txid.clone(), deposit.vout)
                .await;
            if let Err(e) = &outspend {
                warn!(
                    "Outspend lookup failed for deposit {}:{}, assuming the refund never landed: {e}",
                    deposit.txid, deposit.vout
                );
                outpoint_checked = false;
            }
            refund_action_from_outspend(outspend.as_ref().ok())
        };

        match action {
            RefundAction::Delete => {
                if let Err(e) = self
                    .storage
                    .delete_deposit(deposit.txid.clone(), deposit.vout)
                    .await
                {
                    error!(
                        "Failed to delete refunded deposit {}:{}: {e}",
                        deposit.txid, deposit.vout
                    );
                }
            }
            RefundAction::MarkBroadcast => {
                self.set_refund_state(deposit, RefundState::Broadcast).await;
            }
            RefundAction::Rebroadcast => {
                let Some(refund_tx) = deposit.refund_tx.clone() else {
                    return;
                };
                match self.chain_service.broadcast_transaction(refund_tx).await {
                    Ok(()) => {
                        info!(
                            "Rebroadcast refund of deposit {}:{}",
                            deposit.txid, deposit.vout
                        );
                        self.set_refund_state(deposit, RefundState::Broadcast).await;
                    }
                    Err(e) if already_on_network(&e.to_string()) => {
                        info!(
                            "Refund of deposit {}:{} is already on the network",
                            deposit.txid, deposit.vout
                        );
                        self.set_refund_state(deposit, RefundState::Broadcast).await;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to rebroadcast refund of deposit {}:{}: {e}",
                            deposit.txid, deposit.vout
                        );
                        // The outpoint was not read, so this failure says nothing
                        // about where the refund is. Recording it would overwrite a
                        // `Broadcast` refund with an unrelated reason.
                        if outpoint_checked {
                            self.set_refund_state(
                                deposit,
                                RefundState::BroadcastPending {
                                    last_error: Some(e.to_string()),
                                },
                            )
                            .await;
                        }
                    }
                }
            }
            RefundAction::None => {}
        }
    }

    /// Records a state against the refund it was decided for. The deposits were
    /// read before the chain lookups, so a `refund_deposit` call can have stored a
    /// different refund since; the write is scoped to the observed txid and does
    /// nothing when that happens.
    async fn set_refund_state(&self, deposit: &DepositInfo, state: RefundState) {
        let Some(refund_txid) = deposit.refund_tx_id.clone() else {
            return;
        };
        if let Err(e) = self
            .storage
            .update_deposit(
                deposit.txid.clone(),
                deposit.vout,
                UpdateDepositPayload::RefundBroadcastState { refund_txid, state },
            )
            .await
        {
            error!(
                "Failed to update refund state of deposit {}:{}: {e}",
                deposit.txid, deposit.vout
            );
        }
    }
}

/// What a refunded deposit needs next, given its output on chain and how far
/// its stored refund has got.
#[derive(Debug, PartialEq, Eq)]
enum RefundAction {
    /// The output is spent for good, so the deposit is settled and the row can go.
    Delete,
    /// The stored refund is on the network but not recorded as such.
    MarkBroadcast,
    /// Nothing is spending the output, so put the stored refund back out.
    Rebroadcast,
    /// A conflicting transaction is unconfirmed, or the state already matches.
    None,
}

/// Decides from the refund's own confirmation. A `None` status means it is not
/// on chain, which only the deposit output can resolve, so this returns `None`
/// for [`refund_action_from_outspend`].
fn refund_action_from_status(
    status: Option<&TxStatus>,
    state: Option<&RefundState>,
) -> Option<RefundAction> {
    match status {
        Some(status) if status.confirmed => Some(RefundAction::Delete),
        // It is on chain, so it did reach the network.
        Some(_) if !matches!(state, Some(RefundState::Broadcast)) => {
            Some(RefundAction::MarkBroadcast)
        }
        Some(_) => Some(RefundAction::None),
        None => None,
    }
}

/// Whether the broadcast was refused because the network already has the
/// transaction, which makes the rebroadcast a success. Bitcoin Core answers a
/// resend this way rather than accepting it again.
fn already_on_network(error: &str) -> bool {
    let error = error.to_lowercase();
    error.contains("already in mempool")
        || error.contains("already in block chain")
        || error.contains("already in utxo set")
        || error.contains("already known")
        || error.contains("txn-already")
}

/// Decides from the deposit output, for a refund that is not on chain. `None`
/// means the lookup was unavailable.
fn refund_action_from_outspend(outspend: Option<&Outspend>) -> RefundAction {
    match outspend {
        // Another transaction took the deposit for good, so the deposit is
        // settled whichever refund did it.
        Some(Outspend::Spent { status, .. }) if status.confirmed => RefundAction::Delete,
        // The stored refund is not on the network, and an unconfirmed spender is
        // some other transaction. The stored refund never got out, so putting it
        // back is the only way it lands, and a rejection records why.
        _ => RefundAction::Rebroadcast,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RefundAction, already_on_network, refund_action_from_outspend, refund_action_from_status,
    };
    use crate::{RefundState, chain::Outspend, chain::TxStatus};

    fn status(confirmed: bool) -> TxStatus {
        TxStatus {
            confirmed,
            block_height: None,
            block_time: None,
        }
    }

    fn spent(confirmed: bool) -> Outspend {
        Outspend::Spent {
            txid: "spender-txid".to_string(),
            vin: 0,
            status: status(confirmed),
        }
    }

    fn pending() -> RefundState {
        RefundState::BroadcastPending { last_error: None }
    }

    #[test]
    fn a_confirmed_refund_settles_the_deposit() {
        assert_eq!(
            refund_action_from_status(Some(&status(true)), Some(&pending())),
            Some(RefundAction::Delete)
        );
        assert_eq!(
            refund_action_from_status(Some(&status(true)), Some(&RefundState::Broadcast)),
            Some(RefundAction::Delete)
        );
    }

    #[test]
    fn a_refund_already_on_chain_did_reach_the_network() {
        assert_eq!(
            refund_action_from_status(Some(&status(false)), Some(&pending())),
            Some(RefundAction::MarkBroadcast)
        );
        // A refund stored before this state existed is not known to have landed.
        assert_eq!(
            refund_action_from_status(Some(&status(false)), None),
            Some(RefundAction::MarkBroadcast)
        );
        // Already recorded, nothing to change.
        assert_eq!(
            refund_action_from_status(Some(&status(false)), Some(&RefundState::Broadcast)),
            Some(RefundAction::None)
        );
    }

    #[test]
    fn an_unknown_refund_defers_to_the_deposit_output() {
        assert_eq!(refund_action_from_status(None, Some(&pending())), None);
        assert_eq!(refund_action_from_status(None, None), None);
        assert_eq!(
            refund_action_from_status(None, Some(&RefundState::Broadcast)),
            None
        );
    }

    #[test]
    fn an_unspent_deposit_rebroadcasts_the_stored_refund() {
        // Never broadcast, or broadcast and since dropped from the mempool.
        assert_eq!(
            refund_action_from_outspend(Some(&Outspend::Unspent)),
            RefundAction::Rebroadcast
        );
        // Without an outspend endpoint the refund is assumed never to have
        // landed, which is what recovers a deposit stuck by a failed broadcast.
        assert_eq!(refund_action_from_outspend(None), RefundAction::Rebroadcast);
    }

    #[test]
    fn a_resend_the_network_already_has_is_not_a_failure() {
        // What Bitcoin Core answers a resend with, through esplora's passthrough.
        assert!(already_on_network(
            "Status error: 400 - sendrawtransaction RPC error: \
             {\"code\":-27,\"message\":\"Transaction already in block chain\"}"
        ));
        assert!(already_on_network("bad-txns: txn-already-in-mempool"));
        assert!(already_on_network("Transaction already in mempool"));
        assert!(already_on_network("txn-already-known"));
        // Core >= v25 wording for the same rejection.
        assert!(already_on_network(
            "Transaction outputs already in utxo set"
        ));
        // A genuine refusal still records why.
        assert!(!already_on_network(
            "min relay fee not met, 111 < 222 (code -26)"
        ));
        assert!(!already_on_network("txn-mempool-conflict"));
    }

    #[test]
    fn a_deposit_spent_for_good_settles_whoever_spent_it() {
        assert_eq!(
            refund_action_from_outspend(Some(&spent(true))),
            RefundAction::Delete
        );
    }

    #[test]
    fn a_replacement_that_never_got_out_is_rebroadcast() {
        // The outpoint is only read when the stored refund is not on chain, so an
        // unconfirmed spender is a different transaction and the stored refund is a
        // replacement that never landed.
        assert_eq!(
            refund_action_from_outspend(Some(&spent(false))),
            RefundAction::Rebroadcast
        );
    }
}
