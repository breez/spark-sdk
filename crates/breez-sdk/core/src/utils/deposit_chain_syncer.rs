use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use spark_wallet::SparkWallet;
use tracing::{error, info, warn};

use crate::{
    BitcoinChainService, DepositInfo, InstantClaimStatus, RefundState, SdkError,
    chain::{Outspend, TxStatus, Utxo},
    persist::{Storage, UpdateDepositPayload, UpdateWatchedAddressPayload},
    utils::deposit_address_watch::{
        AddressObservation, now_secs, observed_actions, plan_watch, unreported_utxos,
    },
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
                        if let Err(e) = self
                            .reconcile_deposits(&HashMap::new(), &HashSet::new(), true)
                            .await
                        {
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
                match fetch_and_record_deposit(
                    &self.utxo_fetcher,
                    &self.storage,
                    &txid_str,
                    utxo.vout,
                    utxo.is_mature,
                )
                .await
                {
                    Ok(detailed_utxo) => {
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

        // The chain sees a deposit as soon as it is broadcast, which is what makes
        // a 0-conf claim possible; the operators only report it once it confirms.
        let mut confirmed_onchain: HashSet<TxOutput> = HashSet::new();
        if let Some(now) = now_secs() {
            let already_seen: HashSet<TxOutput> = detailed_utxos.keys().cloned().collect();
            let watch = sync_chain_watch(
                self.chain_service.as_ref(),
                &self.utxo_fetcher,
                &self.storage,
                &already_seen,
                now,
            )
            .await;
            detailed_utxos.extend(watch.unconfirmed);
            confirmed_onchain = watch.confirmed;
            if !watch.complete {
                hit_error = true;
            }
        } else {
            warn!("Skipping the deposit address watch: unusable system clock");
        }

        let refunded = self
            .reconcile_deposits(&detailed_utxos, &confirmed_onchain, hit_error)
            .await?;

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
    /// `confirmed_onchain` holds outpoints the address watch saw confirmed. The two
    /// sources are read one after the other, so a block landing between them
    /// hides a deposit from both; this is the evidence that it is still there.
    async fn reconcile_deposits(
        &self,
        all_utxos: &HashMap<TxOutput, (DetailedUtxo, bool)>,
        confirmed_onchain: &HashSet<TxOutput>,
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
                    if !incomplete
                        && deposit_unobserved(&key, all_utxos, confirmed_onchain)
                        && self.can_drop_unobserved_deposit(&deposit).await
                    {
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

    /// Whether a deposit neither the operators nor the chain watch reported this
    /// pass can be deleted.
    ///
    /// Only a submitted instant claim is held back. Such a deposit leaves both
    /// sources for as long as the provider takes to sweep the UTXO, and its
    /// status is the only guard against claiming it twice, so it is kept until
    /// the outpoint is spent.
    ///
    /// Anything else is dropped. That is not a guarantee the deposit is settled:
    /// the two sources are read one after the other, so a block landing between
    /// them hides a deposit from both, and the row is deleted and re-announced
    /// next pass. Transient and self-healing, at the cost of losing whatever
    /// claim error or decline was recorded against it.
    async fn can_drop_unobserved_deposit(&self, deposit: &DepositInfo) -> bool {
        if !matches!(
            deposit.instant_claim_status,
            Some(InstantClaimStatus::Submitted { .. })
        ) {
            return true;
        }
        match self
            .chain_service
            .get_outspend(deposit.txid.clone(), deposit.vout)
            .await
        {
            Ok(Outspend::Spent { status, .. }) if status.confirmed => true,
            Ok(_) => false,
            Err(e) => {
                warn!(
                    "Outspend lookup failed for claimed deposit {}:{}, keeping it: {e}",
                    deposit.txid, deposit.vout
                );
                false
            }
        }
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

async fn fetch_and_record_deposit(
    utxo_fetcher: &CachedUtxoFetcher,
    storage: &Arc<dyn Storage>,
    txid: &str,
    vout: u32,
    is_mature: bool,
) -> Result<DetailedUtxo, SdkError> {
    let detailed_utxo = utxo_fetcher.fetch_detailed_utxo(txid, vout).await?;
    storage
        .add_deposit(
            detailed_utxo.txid.to_string(),
            detailed_utxo.vout,
            detailed_utxo.value,
            is_mature,
        )
        .await?;
    Ok(detailed_utxo)
}

/// What one address-watch pass saw.
pub(crate) struct ChainWatchResult {
    /// Unconfirmed deposits, which the claim cascade takes from here.
    pub unconfirmed: HashMap<TxOutput, (DetailedUtxo, bool)>,
    /// Outpoints seen confirmed on a watched address. Deliberately apart from
    /// `unconfirmed`: the operators own anything confirmed, so these must not
    /// reach the cascade and go down the immature branch. They are proof the
    /// deposit still exists, which is what reconciliation needs.
    pub confirmed: HashSet<TxOutput>,
    /// Whether every watched address was read.
    pub complete: bool,
}

impl ChainWatchResult {
    fn empty(complete: bool) -> Self {
        Self {
            unconfirmed: HashMap::new(),
            confirmed: HashSet::new(),
            complete,
        }
    }
}

/// Polls the watched addresses for deposits still in the mempool.
///
/// When an address could not be read, `complete` is false and the caller must
/// not delete deposit rows this pass: a row that was not re-observed has not
/// necessarily gone away.
pub(crate) async fn sync_chain_watch(
    chain_service: &dyn BitcoinChainService,
    utxo_fetcher: &CachedUtxoFetcher,
    storage: &Arc<dyn Storage>,
    already_seen: &HashSet<TxOutput>,
    now: u64,
) -> ChainWatchResult {
    let mut unconfirmed = HashMap::new();
    let mut confirmed = HashSet::new();
    let watched = match storage.list_watched_deposit_addresses().await {
        Ok(watched) => watched,
        Err(e) => {
            warn!("Failed to read the watched deposit addresses: {e}");
            return ChainWatchResult::empty(false);
        }
    };
    if watched.is_empty() {
        return ChainWatchResult::empty(true);
    }
    let plan = plan_watch(&watched, now);
    // Retiring these does not depend on the polling below, so it is safe even
    // when the pass turns out to be incomplete.
    for (address, issued_at) in plan.retire {
        apply_watch_write(
            storage,
            address,
            UpdateWatchedAddressPayload::Unwatch { issued_at },
        )
        .await;
    }

    let mut observations = HashMap::new();
    let mut complete = true;
    'watch_loop: for address in plan.poll {
        let utxos = match chain_service.get_address_utxos(address.clone()).await {
            Ok(utxos) => utxos,
            Err(e) => {
                warn!("Failed to read the UTXOs of watched address {address}: {e}");
                complete = false;
                break;
            }
        };
        let (confirmed_utxos, unconfirmed_utxos): (Vec<Utxo>, Vec<Utxo>) =
            utxos.into_iter().partition(|utxo| utxo.status.confirmed);
        confirmed.extend(confirmed_utxos.into_iter().map(|utxo| TxOutput {
            txid: utxo.txid,
            vout: utxo.vout,
        }));
        observations.insert(
            address.clone(),
            if unconfirmed_utxos.is_empty() {
                AddressObservation::NothingToClaim
            } else {
                AddressObservation::Claimable
            },
        );

        for new_utxo in unreported_utxos(unconfirmed_utxos, already_seen) {
            let is_mature = false;
            match fetch_and_record_deposit(
                utxo_fetcher,
                storage,
                &new_utxo.txid,
                new_utxo.vout,
                is_mature,
            )
            .await
            {
                Ok(detailed_utxo) => {
                    let key = TxOutput {
                        txid: detailed_utxo.txid.to_string(),
                        vout: detailed_utxo.vout,
                    };
                    unconfirmed.insert(key, (detailed_utxo, false));
                }
                Err(e) => {
                    warn!(
                        "Failed to fetch details of mempool deposit {}:{}: {e}",
                        new_utxo.txid, new_utxo.vout
                    );
                    complete = false;
                    break 'watch_loop;
                }
            }
        }
    }

    for (address, payload) in observed_actions(&watched, &observations, now) {
        apply_watch_write(storage, address, payload).await;
    }

    ChainWatchResult {
        unconfirmed,
        confirmed,
        complete,
    }
}

/// Whether a pass turned up no evidence that a deposit still exists.
///
/// Being absent from the operator feed is not enough on its own. The two sources
/// are read one after the other, so a block landing between them hides a deposit
/// from both: the operators read it too early to see it confirmed, and the watch
/// reads it too late to see it unconfirmed. An outpoint the watch saw confirmed
/// counts as observed even though it is not claimable here.
fn deposit_unobserved(
    key: &TxOutput,
    all_utxos: &HashMap<TxOutput, (DetailedUtxo, bool)>,
    confirmed_onchain: &HashSet<TxOutput>,
) -> bool {
    !all_utxos.contains_key(key) && !confirmed_onchain.contains(key)
}

/// Applies one watch-table write, logging rather than propagating a failure:
/// the watch only makes a deposit claimable sooner, so it must not fail a sync.
async fn apply_watch_write(
    storage: &Arc<dyn Storage>,
    address: String,
    payload: UpdateWatchedAddressPayload,
) {
    if let Err(e) = storage
        .update_watched_deposit_address(address.clone(), payload)
        .await
    {
        warn!("Failed to update watched deposit address {address}: {e}");
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

#[cfg(test)]
mod reconcile_tests {
    use super::{DetailedUtxo, TxOutput, deposit_unobserved};
    use std::collections::{HashMap, HashSet};

    fn key(txid: &str) -> TxOutput {
        TxOutput {
            txid: txid.to_string(),
            vout: 0,
        }
    }

    #[test]
    fn absent_from_both_sources_is_unobserved() {
        assert!(deposit_unobserved(
            &key("a"),
            &HashMap::new(),
            &HashSet::new()
        ));
    }

    #[test]
    fn an_outpoint_the_watch_saw_confirmed_counts_as_observed() {
        // The handoff case: the operators read it before the block, the watch
        // read it after. Without this the row is deleted and re-announced.
        let confirmed_onchain: HashSet<TxOutput> = [key("a")].into_iter().collect();
        assert!(!deposit_unobserved(
            &key("a"),
            &HashMap::new(),
            &confirmed_onchain
        ));
    }

    #[test]
    fn the_operator_feed_still_counts_on_its_own() {
        let tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        };
        let txid = tx.compute_txid();
        let mut all_utxos: HashMap<TxOutput, (DetailedUtxo, bool)> = HashMap::new();
        all_utxos.insert(
            key("a"),
            (
                DetailedUtxo {
                    tx,
                    vout: 0,
                    txid,
                    value: 1,
                },
                true,
            ),
        );
        assert!(!deposit_unobserved(&key("a"), &all_utxos, &HashSet::new()));
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod chain_watch_tests {
    use super::*;
    use crate::chain::{ChainServiceError, RecommendedFees};
    use crate::persist::UpdateWatchedAddressPayload;
    use bitcoin::Transaction;
    use bitcoin::consensus::encode::serialize_hex;

    /// Chain service serving a fixed set of address UTXOs and the transactions
    /// behind them. `fail_addresses` makes the address lookup error instead.
    struct WatchChainService {
        by_address: HashMap<String, Vec<Utxo>>,
        txs: HashMap<String, String>,
        fail_addresses: bool,
    }

    impl WatchChainService {
        fn new(entries: &[(&str, &Transaction, bool)], fail_addresses: bool) -> Self {
            let mut by_address: HashMap<String, Vec<Utxo>> = HashMap::new();
            let mut txs = HashMap::new();
            for (address, tx, confirmed) in entries {
                let txid = tx.compute_txid().to_string();
                txs.insert(txid.clone(), serialize_hex(*tx));
                by_address
                    .entry((*address).to_string())
                    .or_default()
                    .push(Utxo {
                        txid,
                        vout: 0,
                        value: tx.output[0].value.to_sat(),
                        status: TxStatus {
                            confirmed: *confirmed,
                            block_height: confirmed.then_some(100),
                            block_time: None,
                        },
                    });
            }
            Self {
                by_address,
                txs,
                fail_addresses,
            }
        }
    }

    #[macros::async_trait]
    impl BitcoinChainService for WatchChainService {
        async fn get_address_utxos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
            if self.fail_addresses {
                return Err(ChainServiceError::Generic("boom".to_string()));
            }
            Ok(self.by_address.get(&address).cloned().unwrap_or_default())
        }

        async fn get_address_txos(&self, _address: String) -> Result<Vec<Utxo>, ChainServiceError> {
            unreachable!()
        }

        async fn get_transaction_status(
            &self,
            _txid: String,
        ) -> Result<TxStatus, ChainServiceError> {
            unreachable!()
        }

        async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError> {
            self.txs
                .get(&txid)
                .cloned()
                .ok_or_else(|| ChainServiceError::Generic(format!("no tx {txid}")))
        }

        async fn get_outspend(
            &self,
            _txid: String,
            _vout: u32,
        ) -> Result<Outspend, ChainServiceError> {
            unreachable!()
        }

        async fn broadcast_transaction(&self, _tx: String) -> Result<(), ChainServiceError> {
            unreachable!()
        }

        async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
            unreachable!()
        }

        async fn tip_height(&self) -> Result<u32, ChainServiceError> {
            unreachable!()
        }
    }

    // The chain service stands in for the polling window: while it is
    // answering, the address is handed out again.
    struct ReissueOnRead {
        storage: Arc<dyn Storage>,
    }
    #[macros::async_trait]
    impl BitcoinChainService for ReissueOnRead {
        async fn get_address_utxos(
            &self,
            _address: String,
        ) -> Result<Vec<Utxo>, ChainServiceError> {
            self.storage
                .update_watched_deposit_address(
                    "stale".to_string(),
                    UpdateWatchedAddressPayload::Watch { issued_at: 100_000 },
                )
                .await
                .unwrap();
            Ok(Vec::new())
        }
        async fn get_address_txos(&self, _address: String) -> Result<Vec<Utxo>, ChainServiceError> {
            unreachable!()
        }
        async fn get_transaction_status(
            &self,
            _txid: String,
        ) -> Result<TxStatus, ChainServiceError> {
            unreachable!()
        }
        async fn get_transaction_hex(&self, _txid: String) -> Result<String, ChainServiceError> {
            unreachable!()
        }
        async fn get_outspend(
            &self,
            _txid: String,
            _vout: u32,
        ) -> Result<Outspend, ChainServiceError> {
            unreachable!()
        }
        async fn broadcast_transaction(&self, _tx: String) -> Result<(), ChainServiceError> {
            unreachable!()
        }
        async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
            unreachable!()
        }
        async fn tip_height(&self) -> Result<u32, ChainServiceError> {
            unreachable!()
        }
    }

    fn test_tx(value_sat: u64) -> Transaction {
        Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![bitcoin::TxOut {
                value: bitcoin::Amount::from_sat(value_sat),
                script_pubkey: bitcoin::ScriptBuf::new(),
            }],
        }
    }

    fn test_storage() -> Arc<dyn Storage> {
        let mut dir = std::env::temp_dir();
        dir.push(format!("breez-chain-watch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(crate::SqliteStorage::new(&dir).unwrap())
    }

    async fn watch(storage: &Arc<dyn Storage>, address: &str) {
        storage
            .update_watched_deposit_address(
                address.to_string(),
                UpdateWatchedAddressPayload::Watch { issued_at: 1_000 },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn records_an_unconfirmed_deposit_as_immature() {
        let tx = test_tx(50_000);
        let storage = test_storage();
        watch(&storage, "addr").await;
        let chain = WatchChainService::new(&[("addr", &tx, false)], false);
        let fetcher = CachedUtxoFetcher::new(
            Arc::new(WatchChainService::new(&[("addr", &tx, false)], false)),
            storage.clone(),
        );

        let watch = sync_chain_watch(&chain, &fetcher, &storage, &HashSet::new(), 1_000).await;
        let (unconfirmed, complete) = (watch.unconfirmed, watch.complete);

        assert!(complete);
        assert_eq!(unconfirmed.len(), 1);
        let (_, is_mature) = unconfirmed.values().next().unwrap();
        assert!(!is_mature, "chain-discovered deposits are never mature");

        let deposits = storage.list_deposits().await.unwrap();
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].txid, tx.compute_txid().to_string());
        assert_eq!(deposits[0].amount_sats, 50_000);
        assert!(!deposits[0].is_mature);
    }

    #[tokio::test]
    async fn skips_a_confirmed_output() {
        // Anything at a confirmation is the operator feed's job.
        let tx = test_tx(50_000);
        let storage = test_storage();
        watch(&storage, "addr").await;
        let chain = WatchChainService::new(&[("addr", &tx, true)], false);
        let fetcher = CachedUtxoFetcher::new(
            Arc::new(WatchChainService::new(&[("addr", &tx, true)], false)),
            storage.clone(),
        );

        let watch = sync_chain_watch(&chain, &fetcher, &storage, &HashSet::new(), 1_000).await;
        let (unconfirmed, complete) = (watch.unconfirmed, watch.complete);

        assert!(complete);
        assert!(unconfirmed.is_empty());
        assert!(storage.list_deposits().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_confirmed_output_is_reported_apart_from_the_claimable_ones() {
        // It must not reach the claim cascade: the operators own anything
        // confirmed, and it would go down the immature branch from `unconfirmed`.
        let tx = test_tx(50_000);
        let txid = tx.compute_txid();
        let storage = test_storage();
        watch(&storage, "addr").await;
        let chain = WatchChainService::new(&[("addr", &tx, true)], false);
        let fetcher = CachedUtxoFetcher::new(
            Arc::new(WatchChainService::new(&[("addr", &tx, true)], false)),
            storage.clone(),
        );

        let watch_result =
            sync_chain_watch(&chain, &fetcher, &storage, &HashSet::new(), 1_000).await;

        assert!(watch_result.complete);
        assert!(
            watch_result.unconfirmed.is_empty(),
            "must not reach the cascade"
        );
        assert!(watch_result.confirmed.contains(&TxOutput {
            txid: txid.to_string(),
            vout: 0,
        }));
    }

    #[tokio::test]
    async fn skips_an_outpoint_the_operators_already_reported() {
        let tx = test_tx(50_000);
        let txid = tx.compute_txid();
        let storage = test_storage();
        watch(&storage, "addr").await;
        let chain = WatchChainService::new(&[("addr", &tx, false)], false);
        let fetcher = CachedUtxoFetcher::new(
            Arc::new(WatchChainService::new(&[("addr", &tx, false)], false)),
            storage.clone(),
        );
        let already_seen: HashSet<TxOutput> = [TxOutput {
            txid: txid.to_string(),
            vout: 0,
        }]
        .into_iter()
        .collect();

        let watch = sync_chain_watch(&chain, &fetcher, &storage, &already_seen, 1_000).await;
        let (unconfirmed, complete) = (watch.unconfirmed, watch.complete);

        assert!(complete);
        assert!(unconfirmed.is_empty());
        // Still live for retirement purposes, so the address stays watched.
        assert_eq!(
            storage
                .list_watched_deposit_addresses()
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn an_address_read_failure_reports_incomplete() {
        // An incomplete pass must not let reconciliation delete live rows.
        let tx = test_tx(50_000);
        let storage = test_storage();
        watch(&storage, "addr").await;
        let chain = WatchChainService::new(&[("addr", &tx, false)], true);
        let fetcher = CachedUtxoFetcher::new(
            Arc::new(WatchChainService::new(&[("addr", &tx, false)], false)),
            storage.clone(),
        );

        let watch = sync_chain_watch(&chain, &fetcher, &storage, &HashSet::new(), 1_000).await;
        let (unconfirmed, complete) = (watch.unconfirmed, watch.complete);

        assert!(!complete);
        assert!(unconfirmed.is_empty());
    }

    #[tokio::test]
    async fn no_watched_addresses_costs_no_requests() {
        let storage = test_storage();
        let chain = WatchChainService::new(&[], true);
        let fetcher =
            CachedUtxoFetcher::new(Arc::new(WatchChainService::new(&[], true)), storage.clone());

        let watch = sync_chain_watch(&chain, &fetcher, &storage, &HashSet::new(), 1_000).await;
        let (unconfirmed, complete) = (watch.unconfirmed, watch.complete);

        // `fail_addresses` is set, so a request would have reported incomplete.
        assert!(complete);
        assert!(unconfirmed.is_empty());
    }

    #[tokio::test]
    async fn an_address_requested_mid_pass_is_not_retired() {
        // `receive_payment` restarts an address's window while the pass is out
        // polling. Retiring it off the pre-poll snapshot would drop one that was
        // just asked for.
        let storage = test_storage();
        storage
            .update_watched_deposit_address(
                "current".to_string(),
                UpdateWatchedAddressPayload::Watch { issued_at: 50_000 },
            )
            .await
            .unwrap();
        storage
            .update_watched_deposit_address(
                "stale".to_string(),
                UpdateWatchedAddressPayload::Watch { issued_at: 0 },
            )
            .await
            .unwrap();

        let chain = ReissueOnRead {
            storage: storage.clone(),
        };
        let fetcher = CachedUtxoFetcher::new(
            Arc::new(WatchChainService::new(&[], false)),
            storage.clone(),
        );

        // "current" is inside its window so it is polled, which is what re-issues
        // "stale"; "stale" is past its window on the pre-poll snapshot.
        sync_chain_watch(&chain, &fetcher, &storage, &HashSet::new(), 100_000).await;

        let watched = storage.list_watched_deposit_addresses().await.unwrap();
        assert!(
            watched.iter().any(|w| w.address == "stale"),
            "retired an address that was handed out again during the pass"
        );
    }

    #[tokio::test]
    async fn a_read_failure_does_not_retire_an_address_it_never_reached() {
        // The pass gives up on the first read error, so later addresses are
        // unread. One of those may still be holding an unconfirmed deposit, and
        // silence is not evidence its window should close.
        let storage = test_storage();
        storage
            .update_watched_deposit_address(
                "current".to_string(),
                UpdateWatchedAddressPayload::Watch { issued_at: 50_000 },
            )
            .await
            .unwrap();
        storage
            .update_watched_deposit_address(
                "old".to_string(),
                UpdateWatchedAddressPayload::Watch { issued_at: 0 },
            )
            .await
            .unwrap();
        storage
            .update_watched_deposit_address("old".to_string(), UpdateWatchedAddressPayload::Seen)
            .await
            .unwrap();

        // Every address read fails, so nothing is observed.
        let chain = WatchChainService::new(&[], true);
        let fetcher =
            CachedUtxoFetcher::new(Arc::new(WatchChainService::new(&[], true)), storage.clone());

        let complete = sync_chain_watch(&chain, &fetcher, &storage, &HashSet::new(), 100_000)
            .await
            .complete;

        assert!(!complete);
        let watched = storage.list_watched_deposit_addresses().await.unwrap();
        assert_eq!(
            watched.len(),
            2,
            "an unread address must survive the pass that could not read it"
        );
    }

    #[tokio::test]
    async fn an_unconfirmed_deposit_survives_its_address_window() {
        // End to end: the address is long past its window but has taken a
        // deposit that has not confirmed, so it is still polled and the deposit
        // is still contributed. Asserting this through `sync_chain_watch` rather
        // than `watch_actions` alone, because the bug it guards was the poll set
        // and the retirement rules disagreeing.
        let tx = test_tx(50_000);
        let storage = test_storage();
        storage
            .update_watched_deposit_address(
                "old".to_string(),
                UpdateWatchedAddressPayload::Watch { issued_at: 0 },
            )
            .await
            .unwrap();
        storage
            .update_watched_deposit_address("old".to_string(), UpdateWatchedAddressPayload::Seen)
            .await
            .unwrap();

        let chain = WatchChainService::new(&[("old", &tx, false)], false);
        let fetcher = CachedUtxoFetcher::new(
            Arc::new(WatchChainService::new(&[("old", &tx, false)], false)),
            storage.clone(),
        );

        let watch = sync_chain_watch(
            &chain,
            &fetcher,
            &storage,
            &HashSet::new(),
            10 * 24 * 60 * 60,
        )
        .await;
        let (unconfirmed, complete) = (watch.unconfirmed, watch.complete);

        assert!(complete);
        assert_eq!(
            unconfirmed.len(),
            1,
            "a live deposit must still be contributed"
        );
        assert_eq!(
            storage
                .list_watched_deposit_addresses()
                .await
                .unwrap()
                .len(),
            1,
            "the address must not be retired while its deposit is unconfirmed"
        );
    }

    #[tokio::test]
    async fn a_deposit_the_provider_refused_stays_visible() {
        // Refusing an early claim does not settle the deposit: it is still
        // pending and unclaimed, and only the operators reporting it at a
        // confirmation takes over from the watch.
        let tx = test_tx(50_000);
        let txid = tx.compute_txid();
        let storage = test_storage();
        watch(&storage, "current").await;
        storage
            .update_watched_deposit_address(
                "old".to_string(),
                UpdateWatchedAddressPayload::Watch { issued_at: 500 },
            )
            .await
            .unwrap();
        storage
            .add_deposit(txid.to_string(), 0, 50_000, false)
            .await
            .unwrap();
        storage
            .update_deposit(
                txid.to_string(),
                0,
                UpdateDepositPayload::InstantClaim {
                    status: InstantClaimStatus::Declined {
                        max_fee_sats: None,
                        confirmations: 0,
                    },
                },
            )
            .await
            .unwrap();

        let chain = WatchChainService::new(&[("old", &tx, false)], false);
        let fetcher = CachedUtxoFetcher::new(
            Arc::new(WatchChainService::new(&[("old", &tx, false)], false)),
            storage.clone(),
        );

        let watch = sync_chain_watch(&chain, &fetcher, &storage, &HashSet::new(), 1_000).await;
        let (unconfirmed, complete) = (watch.unconfirmed, watch.complete);

        assert!(complete);
        // Still contributed, so reconciliation keeps the row and the deposit
        // stays in list_unclaimed_deposits.
        assert_eq!(unconfirmed.len(), 1);
        // And the address is still polled.
        let watched = storage.list_watched_deposit_addresses().await.unwrap();
        assert_eq!(watched.len(), 2);
    }

    #[tokio::test]
    async fn a_first_sighting_marks_the_address_seen() {
        let tx = test_tx(50_000);
        let storage = test_storage();
        watch(&storage, "current").await;
        storage
            .update_watched_deposit_address(
                "old".to_string(),
                UpdateWatchedAddressPayload::Watch { issued_at: 500 },
            )
            .await
            .unwrap();

        let chain = WatchChainService::new(&[("old", &tx, false)], false);
        let fetcher = CachedUtxoFetcher::new(
            Arc::new(WatchChainService::new(&[("old", &tx, false)], false)),
            storage.clone(),
        );

        sync_chain_watch(&chain, &fetcher, &storage, &HashSet::new(), 1_000).await;

        let watched = storage.list_watched_deposit_addresses().await.unwrap();
        let old = watched.iter().find(|w| w.address == "old").unwrap();
        assert!(old.seen);
    }
}
