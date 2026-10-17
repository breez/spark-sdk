#[cfg(test)]
use std::collections::HashMap;

use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use spark::services::TransferId;
#[cfg(test)]
use tokio::sync::RwLock;

/// A taproot signature commits to every input's prevout value and script, which
/// the raw tx does not carry. The address gives the script and the signing key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoopExitPrevout {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub address: String,
}

#[derive(Debug, Clone)]
pub struct CoopExitRecord {
    pub id: String,
    pub user_identity_public_key: PublicKey,
    pub user_transfer_id: TransferId,
    pub withdrawal_address: String,
    pub amount_sats: u64,
    pub fee_sats: u64,
    pub raw_coop_exit_tx: Vec<u8>,
    pub raw_connector_tx: Vec<u8>,
    pub coop_exit_txid: String,
    pub prevouts: Vec<CoopExitPrevout>,
    /// Whether the user's transfer for this request has been verified.
    pub completed: bool,
    pub broadcast_txid: Option<String>,
    pub leaves_claimed: bool,
    /// Whether the leaves are claimed and the exit is deep enough to stop
    /// rebroadcasting it.
    pub settled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl CoopExitRecord {
    pub fn expires_at(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Duration::from_std(super::INCOMPLETE_REQUEST_TTL)
            .ok()
            .and_then(|ttl| self.created_at.checked_add_signed(ttl))
            .unwrap_or(chrono::DateTime::<chrono::Utc>::MAX_UTC)
    }
}

#[async_trait::async_trait]
pub trait CoopExitStore: Send + Sync {
    /// Stores the request with the wallet coins its transactions spend, which no other
    /// transaction may then take. The leaves backing it are held until it completes.
    async fn insert(&self, record: &CoopExitRecord, leaf_ids: &[String]) -> Result<(), String>;
    async fn any_leaf_in_open_request(&self, leaf_ids: &[String]) -> Result<bool, String>;
    /// Removes the request unless it is completed, releasing its coins.
    async fn abandon(&self, id: &str) -> Result<(), String>;
    async fn set_completed(&self, id: &str) -> Result<(), String>;
    async fn set_broadcast_txid(&self, id: &str, txid: &str) -> Result<(), String>;
    async fn set_leaves_claimed(&self, id: &str) -> Result<(), String>;
    async fn set_settled(&self, id: &str) -> Result<(), String>;
    async fn get(&self, id: &str) -> Result<Option<CoopExitRecord>, String>;
    async fn get_by_transfer_id(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<CoopExitRecord>, String>;
    /// Completed requests not yet settled.
    async fn pending(&self) -> Result<Vec<CoopExitRecord>, String>;
    async fn incomplete_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<CoopExitRecord>, String>;
    /// The most recent `limit` requests, newest first.
    async fn list(&self, limit: u32) -> Result<Vec<CoopExitRecord>, String>;
}

#[cfg(test)]
#[derive(Default)]
pub struct InMemoryCoopExitStore {
    records: RwLock<HashMap<String, CoopExitRecord>>,
    open_leaves: RwLock<HashMap<String, String>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl CoopExitStore for InMemoryCoopExitStore {
    async fn insert(&self, record: &CoopExitRecord, leaf_ids: &[String]) -> Result<(), String> {
        let mut records = self.records.write().await;
        if records
            .values()
            .any(|existing| existing.user_transfer_id == record.user_transfer_id)
        {
            return Err("transfer already backs a coop exit".to_string());
        }
        let mut open_leaves = self.open_leaves.write().await;
        if leaf_ids.iter().any(|leaf| open_leaves.contains_key(leaf)) {
            return Err("a leaf already backs an open request".to_string());
        }
        for leaf in leaf_ids {
            open_leaves.insert(leaf.clone(), record.id.clone());
        }
        records.insert(record.id.clone(), record.clone());
        Ok(())
    }

    async fn any_leaf_in_open_request(&self, leaf_ids: &[String]) -> Result<bool, String> {
        let open_leaves = self.open_leaves.read().await;
        Ok(leaf_ids.iter().any(|leaf| open_leaves.contains_key(leaf)))
    }

    async fn abandon(&self, id: &str) -> Result<(), String> {
        let mut records = self.records.write().await;
        if records.get(id).is_some_and(|record| !record.completed) {
            records.remove(id);
            self.open_leaves
                .write()
                .await
                .retain(|_, request| request != id);
        }
        Ok(())
    }

    async fn set_completed(&self, id: &str) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.completed = true;
            record.updated_at = chrono::Utc::now();
        }
        self.open_leaves
            .write()
            .await
            .retain(|_, request| request != id);
        Ok(())
    }

    async fn set_broadcast_txid(&self, id: &str, txid: &str) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.broadcast_txid = Some(txid.to_string());
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_leaves_claimed(&self, id: &str) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.leaves_claimed = true;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_settled(&self, id: &str) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.settled = true;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<CoopExitRecord>, String> {
        Ok(self.records.read().await.get(id).cloned())
    }

    async fn get_by_transfer_id(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<CoopExitRecord>, String> {
        Ok(self
            .records
            .read()
            .await
            .values()
            .find(|record| record.user_transfer_id == *transfer_id)
            .cloned())
    }

    async fn pending(&self) -> Result<Vec<CoopExitRecord>, String> {
        Ok(self
            .records
            .read()
            .await
            .values()
            .filter(|record| record.completed && !record.settled)
            .cloned()
            .collect())
    }

    async fn incomplete_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<CoopExitRecord>, String> {
        Ok(self
            .records
            .read()
            .await
            .values()
            .filter(|record| !record.completed && record.created_at < cutoff)
            .cloned()
            .collect())
    }

    async fn list(&self, limit: u32) -> Result<Vec<CoopExitRecord>, String> {
        let mut records: Vec<CoopExitRecord> =
            self.records.read().await.values().cloned().collect();
        records.sort_by_key(|record| std::cmp::Reverse(record.created_at));
        records.truncate(limit as usize);
        Ok(records)
    }
}

#[cfg(test)]
pub(crate) mod scenarios {
    use std::sync::atomic::{AtomicU32, Ordering};

    use bitcoin::absolute::LockTime;
    use bitcoin::hashes::Hash;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use bitcoin::transaction::Version;
    use bitcoin::{Address, Amount, Network, OutPoint, Transaction, TxIn, TxOut, Txid};
    use chrono::{DateTime, Duration, Utc};
    use futures::future::join_all;
    use spark::services::TransferId;

    use super::{CoopExitPrevout, CoopExitRecord, CoopExitStore};

    fn key(byte: u8) -> PublicKey {
        SecretKey::from_slice(&[byte; 32])
            .unwrap()
            .public_key(&Secp256k1::new())
    }

    fn address(byte: u8) -> Address {
        let key = key(byte).x_only_public_key().0;
        Address::p2tr(&Secp256k1::new(), key, None, Network::Regtest)
    }

    /// Each request has its own transfer and its own two wallet coins.
    pub(crate) fn request(id: &str) -> CoopExitRecord {
        static MADE: AtomicU32 = AtomicU32::new(0);
        let number = MADE.fetch_add(1, Ordering::Relaxed);
        let wallet = address(2);
        let coins = [0, 1].map(|vout| OutPoint::new(Txid::hash(&number.to_be_bytes()), vout));
        let exit = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: coins
                .iter()
                .map(|coin| TxIn {
                    previous_output: *coin,
                    ..TxIn::default()
                })
                .collect(),
            output: vec![
                TxOut {
                    value: Amount::from_sat(10_000),
                    script_pubkey: address(3).script_pubkey(),
                },
                TxOut {
                    value: Amount::from_sat(1_500),
                    script_pubkey: wallet.script_pubkey(),
                },
            ],
        };
        let connector = Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(exit.compute_txid(), 1),
                ..TxIn::default()
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(330),
                    script_pubkey: wallet.script_pubkey(),
                };
                2
            ],
        };
        let created_at =
            DateTime::from_timestamp(i64::from(number).saturating_add(1_700_000_000), 0).unwrap();
        CoopExitRecord {
            id: id.to_string(),
            user_identity_public_key: key(1),
            user_transfer_id: TransferId::generate(),
            withdrawal_address: address(3).to_string(),
            amount_sats: 10_000,
            fee_sats: 2_500,
            raw_coop_exit_tx: bitcoin::consensus::serialize(&exit),
            raw_connector_tx: bitcoin::consensus::serialize(&connector),
            coop_exit_txid: exit.compute_txid().to_string(),
            prevouts: coins
                .iter()
                .map(|coin| CoopExitPrevout {
                    txid: coin.txid.to_string(),
                    vout: coin.vout,
                    value: 6_500,
                    address: wallet.to_string(),
                })
                .collect(),
            completed: false,
            broadcast_txid: None,
            leaves_claimed: false,
            settled: false,
            created_at,
            updated_at: created_at,
        }
    }

    pub(crate) fn leaves(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    fn ids(requests: Vec<CoopExitRecord>) -> Vec<String> {
        requests.into_iter().map(|request| request.id).collect()
    }

    fn sorted_ids(requests: Vec<CoopExitRecord>) -> Vec<String> {
        let mut ids = ids(requests);
        ids.sort();
        ids
    }

    /// Coop exit records have no equality, so a scenario compares them by how
    /// they print.
    fn fields(request: &CoopExitRecord) -> String {
        format!("{request:?}")
    }

    async fn read(store: &dyn CoopExitStore, id: &str) -> CoopExitRecord {
        store.get(id).await.unwrap().unwrap()
    }

    async fn in_open_request(store: &dyn CoopExitStore, leaf_ids: &[&str]) -> bool {
        store
            .any_leaf_in_open_request(&leaves(leaf_ids))
            .await
            .unwrap()
    }

    fn tomorrow() -> DateTime<Utc> {
        Utc::now().checked_add_signed(Duration::days(1)).unwrap()
    }

    async fn a_request_in_each_state(store: &dyn CoopExitStore) {
        let states = ["incomplete", "completed", "broadcast", "claimed", "settled"];
        for (reached, id) in states.into_iter().enumerate() {
            let request = request(id);
            store.insert(&request, &[]).await.unwrap();
            if reached >= 1 {
                store.set_completed(id).await.unwrap();
            }
            if reached >= 2 {
                let txid = &request.coop_exit_txid;
                store.set_broadcast_txid(id, txid).await.unwrap();
            }
            if reached >= 3 {
                store.set_leaves_claimed(id).await.unwrap();
            }
            if reached >= 4 {
                store.set_settled(id).await.unwrap();
            }
        }
    }

    pub(crate) async fn a_request_reads_back_as_it_was_stored(store: &dyn CoopExitStore) {
        let request = request("a");
        store.insert(&request, &[]).await.unwrap();

        let stored = read(store, "a").await;

        // The store sets a new request's times from its own clock.
        let expected = CoopExitRecord {
            created_at: stored.created_at,
            updated_at: stored.updated_at,
            ..request
        };
        assert_eq!(fields(&stored), fields(&expected));
    }

    pub(crate) async fn a_request_is_found_by_the_transfer_the_user_paid_for_it_with(
        store: &dyn CoopExitStore,
    ) {
        let (first, second) = (request("first"), request("second"));
        store.insert(&first, &[]).await.unwrap();
        store.insert(&second, &[]).await.unwrap();

        let found = store
            .get_by_transfer_id(&second.user_transfer_id)
            .await
            .unwrap();

        assert_eq!(found.map(|found| found.id).as_deref(), Some("second"));
    }

    pub(crate) async fn no_request_is_found_under_an_id_or_a_transfer_the_store_does_not_have(
        store: &dyn CoopExitStore,
    ) {
        store.insert(&request("stored"), &[]).await.unwrap();

        let by_id = store.get("another").await.unwrap();
        let by_transfer = store
            .get_by_transfer_id(&TransferId::generate())
            .await
            .unwrap();

        assert!(by_id.is_none());
        assert!(by_transfer.is_none());
    }

    pub(crate) async fn a_second_request_paid_for_with_the_same_transfer_is_not_stored(
        store: &dyn CoopExitStore,
    ) {
        let first = request("first");
        store.insert(&first, &leaves(&["a"])).await.unwrap();
        let second = CoopExitRecord {
            user_transfer_id: first.user_transfer_id.clone(),
            ..request("second")
        };

        let first_again = store.insert(&first, &[]).await;
        let second_stored = store.insert(&second, &leaves(&["b"])).await;

        assert!(first_again.is_err());
        assert!(second_stored.is_err());
        assert_eq!(ids(store.list(10).await.unwrap()), ["first"]);
        assert!(!in_open_request(store, &["b"]).await);
    }

    pub(crate) async fn of_concurrent_requests_paid_for_with_one_transfer_one_is_stored(
        store: &dyn CoopExitStore,
    ) {
        let transfer = TransferId::generate();
        let requests = ["a", "b", "c", "d", "e", "f"].map(|id| CoopExitRecord {
            user_transfer_id: transfer.clone(),
            ..request(id)
        });

        let stored = join_all(requests.iter().map(|request| store.insert(request, &[]))).await;

        assert_eq!(stored.iter().filter(|stored| stored.is_ok()).count(), 1);
        assert_eq!(store.list(10).await.unwrap().len(), 1);
    }

    pub(crate) async fn a_request_reads_back_in_each_state_it_was_changed_to(
        store: &dyn CoopExitStore,
    ) {
        let request = request("a");
        store.insert(&request, &[]).await.unwrap();
        let txid = Some(request.coop_exit_txid.clone());
        let state = async || {
            let stored = read(store, "a").await;
            (
                stored.completed,
                stored.broadcast_txid,
                stored.leaves_claimed,
                stored.settled,
            )
        };
        assert_eq!(state().await, (false, None, false, false));

        store.set_completed("a").await.unwrap();
        assert_eq!(state().await, (true, None, false, false));

        store
            .set_broadcast_txid("a", &request.coop_exit_txid)
            .await
            .unwrap();
        assert_eq!(state().await, (true, txid.clone(), false, false));

        store.set_leaves_claimed("a").await.unwrap();
        assert_eq!(state().await, (true, txid.clone(), true, false));

        store.set_settled("a").await.unwrap();
        assert_eq!(state().await, (true, txid, true, true));
    }

    pub(crate) async fn a_request_stays_as_it_was_when_another_changes_state(
        store: &dyn CoopExitStore,
    ) {
        let changed = request("changed");
        store.insert(&changed, &leaves(&["a"])).await.unwrap();
        store
            .insert(&request("other"), &leaves(&["b"]))
            .await
            .unwrap();
        let before = fields(&read(store, "other").await);

        store.set_completed("changed").await.unwrap();
        store
            .set_broadcast_txid("changed", &changed.coop_exit_txid)
            .await
            .unwrap();
        store.set_leaves_claimed("changed").await.unwrap();
        store.set_settled("changed").await.unwrap();

        assert_eq!(fields(&read(store, "other").await), before);
        assert!(in_open_request(store, &["b"]).await);
    }

    pub(crate) async fn nothing_is_stored_for_a_change_to_a_request_the_store_does_not_have(
        store: &dyn CoopExitStore,
    ) {
        let stored = request("stored");
        store.insert(&stored, &leaves(&["a"])).await.unwrap();
        let before = fields(&read(store, "stored").await);

        store.set_completed("another").await.unwrap();
        store
            .set_broadcast_txid("another", &stored.coop_exit_txid)
            .await
            .unwrap();
        store.set_leaves_claimed("another").await.unwrap();
        store.set_settled("another").await.unwrap();
        store.abandon("another").await.unwrap();

        assert_eq!(ids(store.list(10).await.unwrap()), ["stored"]);
        assert_eq!(fields(&read(store, "stored").await), before);
        assert!(in_open_request(store, &["a"]).await);
    }

    pub(crate) async fn the_store_sets_the_time_of_the_last_update_at_each_change_of_state(
        store: &dyn CoopExitStore,
    ) {
        let requests = ["completed", "broadcast", "claimed", "settled"].map(request);
        let mut stored = Vec::new();
        for request in &requests {
            store.insert(request, &[]).await.unwrap();
            stored.push(read(store, &request.id).await);
        }
        let [_, broadcast, ..] = &requests;

        store.set_completed("completed").await.unwrap();
        store
            .set_broadcast_txid("broadcast", &broadcast.coop_exit_txid)
            .await
            .unwrap();
        store.set_leaves_claimed("claimed").await.unwrap();
        store.set_settled("settled").await.unwrap();

        for before in stored {
            let after = read(store, &before.id).await;
            assert!(after.updated_at > before.updated_at, "{}", before.id);
            assert_eq!(after.created_at, before.created_at, "{}", before.id);
        }
    }

    pub(crate) async fn only_requests_that_are_not_completed_are_returned_as_incomplete(
        store: &dyn CoopExitStore,
    ) {
        a_request_in_each_state(store).await;

        let incomplete = store.incomplete_before(tomorrow()).await.unwrap();

        assert_eq!(ids(incomplete), ["incomplete"]);
    }

    pub(crate) async fn only_completed_requests_that_are_not_settled_are_returned_as_pending(
        store: &dyn CoopExitStore,
    ) {
        a_request_in_each_state(store).await;

        let pending = store.pending().await.unwrap();

        assert_eq!(sorted_ids(pending), ["broadcast", "claimed", "completed"]);
    }

    pub(crate) async fn a_request_is_listed_in_every_state(store: &dyn CoopExitStore) {
        a_request_in_each_state(store).await;

        let listed = store.list(10).await.unwrap();

        assert_eq!(
            sorted_ids(listed),
            ["broadcast", "claimed", "completed", "incomplete", "settled"]
        );
    }

    pub(crate) async fn only_requests_created_before_the_cutoff_are_returned_as_incomplete(
        store: &dyn CoopExitStore,
    ) {
        for id in ["completed", "first", "second"] {
            store.insert(&request(id), &[]).await.unwrap();
        }
        store.set_completed("completed").await.unwrap();
        let first = read(store, "first").await.created_at;
        let second = read(store, "second").await.created_at;
        let before = async |cutoff: DateTime<Utc>| {
            sorted_ids(store.incomplete_before(cutoff).await.unwrap())
        };

        assert!(before(first).await.is_empty());
        assert_eq!(before(second).await, ["first"]);
        let after_second = second.checked_add_signed(Duration::microseconds(1));
        assert_eq!(before(after_second.unwrap()).await, ["first", "second"]);
    }

    pub(crate) async fn the_latest_requests_are_listed_newest_first(store: &dyn CoopExitStore) {
        for id in ["first", "second", "third"] {
            store.insert(&request(id), &[]).await.unwrap();
        }
        let listed = async |limit: u32| ids(store.list(limit).await.unwrap());

        assert!(listed(0).await.is_empty());
        assert_eq!(listed(2).await, ["third", "second"]);
        assert_eq!(listed(3).await, ["third", "second", "first"]);
        assert_eq!(listed(10).await, ["third", "second", "first"]);
    }

    pub(crate) async fn a_leaf_of_an_open_request_is_found_in_one(store: &dyn CoopExitStore) {
        store
            .insert(&request("first"), &leaves(&["a", "b"]))
            .await
            .unwrap();
        store
            .insert(&request("second"), &leaves(&["c"]))
            .await
            .unwrap();

        assert!(!in_open_request(store, &[]).await);
        assert!(in_open_request(store, &["a"]).await);
        assert!(in_open_request(store, &["b", "c"]).await);
        assert!(!in_open_request(store, &["d"]).await);
        assert!(in_open_request(store, &["d", "a"]).await);
        assert!(in_open_request(store, &["a", "a"]).await);
    }

    pub(crate) async fn a_request_naming_a_leaf_of_another_open_request_is_not_stored(
        store: &dyn CoopExitStore,
    ) {
        store
            .insert(&request("first"), &leaves(&["a", "b"]))
            .await
            .unwrap();
        let second = request("second");

        let stored = store.insert(&second, &leaves(&["c", "b"])).await;

        assert!(stored.is_err());
        assert!(store.get("second").await.unwrap().is_none());
        assert!(!in_open_request(store, &["c"]).await);
        assert!(in_open_request(store, &["a"]).await);
        assert!(in_open_request(store, &["b"]).await);
    }

    pub(crate) async fn of_concurrent_requests_naming_one_leaf_one_is_stored(
        store: &dyn CoopExitStore,
    ) {
        let requests = ["a", "b", "c", "d", "e", "f"].map(request);
        let leaf = leaves(&["a"]);

        let stored = join_all(requests.iter().map(|request| store.insert(request, &leaf))).await;

        assert_eq!(stored.iter().filter(|stored| stored.is_ok()).count(), 1);
        assert_eq!(store.list(10).await.unwrap().len(), 1);
        assert!(in_open_request(store, &["a"]).await);
    }

    pub(crate) async fn a_leaf_of_a_completed_request_is_in_no_open_request(
        store: &dyn CoopExitStore,
    ) {
        store
            .insert(&request("completed"), &leaves(&["a", "b"]))
            .await
            .unwrap();
        store
            .insert(&request("open"), &leaves(&["c"]))
            .await
            .unwrap();

        store.set_completed("completed").await.unwrap();

        assert!(!in_open_request(store, &["a", "b"]).await);
        assert!(in_open_request(store, &["c"]).await);
        store
            .insert(&request("next"), &leaves(&["a", "b"]))
            .await
            .unwrap();
    }

    pub(crate) async fn an_abandoned_request_is_no_longer_stored(store: &dyn CoopExitStore) {
        let abandoned = request("abandoned");
        store.insert(&abandoned, &leaves(&["a"])).await.unwrap();
        store
            .insert(&request("kept"), &leaves(&["b"]))
            .await
            .unwrap();

        store.abandon("abandoned").await.unwrap();

        assert!(store.get("abandoned").await.unwrap().is_none());
        let by_transfer = store.get_by_transfer_id(&abandoned.user_transfer_id).await;
        assert!(by_transfer.unwrap().is_none());
        assert_eq!(ids(store.list(10).await.unwrap()), ["kept"]);
        let incomplete = store.incomplete_before(tomorrow()).await.unwrap();
        assert_eq!(ids(incomplete), ["kept"]);
        assert!(in_open_request(store, &["b"]).await);
    }

    pub(crate) async fn the_transfer_and_the_leaves_of_an_abandoned_request_are_free_for_a_new_one(
        store: &dyn CoopExitStore,
    ) {
        let abandoned = request("abandoned");
        store
            .insert(&abandoned, &leaves(&["a", "b"]))
            .await
            .unwrap();
        store.abandon("abandoned").await.unwrap();
        assert!(!in_open_request(store, &["a", "b"]).await);
        let next = CoopExitRecord {
            user_transfer_id: abandoned.user_transfer_id,
            ..request("next")
        };

        store.insert(&next, &leaves(&["a", "b"])).await.unwrap();

        let found = store.get_by_transfer_id(&next.user_transfer_id).await;
        assert_eq!(
            found.unwrap().map(|found| found.id).as_deref(),
            Some("next")
        );
        assert!(in_open_request(store, &["a"]).await);
        assert!(in_open_request(store, &["b"]).await);
    }

    pub(crate) async fn a_completed_request_is_not_abandoned(store: &dyn CoopExitStore) {
        store.insert(&request("a"), &[]).await.unwrap();
        store.set_completed("a").await.unwrap();

        store.abandon("a").await.unwrap();

        assert_eq!(ids(store.pending().await.unwrap()), ["a"]);
    }

    pub(crate) async fn pending_requests_are_returned_oldest_first(store: &dyn CoopExitStore) {
        for id in ["oldest", "middle", "newest"] {
            store.insert(&request(id), &[]).await.unwrap();
        }
        for id in ["newest", "oldest", "middle"] {
            store.set_completed(id).await.unwrap();
        }

        let pending = store.pending().await.unwrap();

        assert_eq!(ids(pending), ["oldest", "middle", "newest"]);
    }

    pub(crate) async fn the_store_sets_the_state_and_the_times_of_a_new_request_itself(
        store: &dyn CoopExitStore,
    ) {
        let mut settled = request("a");
        settled.completed = true;
        settled.broadcast_txid = Some(settled.coop_exit_txid.clone());
        settled.leaves_claimed = true;
        settled.settled = true;
        store.insert(&settled, &[]).await.unwrap();

        let stored = read(store, "a").await;

        assert!(stored.created_at > settled.created_at);
        assert_eq!(stored.updated_at, stored.created_at);
        let state = (
            stored.completed,
            stored.broadcast_txid,
            stored.leaves_claimed,
            stored.settled,
        );
        assert_eq!(state, (false, None, false, false));
    }

    pub(crate) async fn a_request_whose_transactions_the_store_cannot_read_is_not_stored(
        store: &dyn CoopExitStore,
    ) {
        let mut exit = request("exit");
        exit.coop_exit_txid = "not a txid".to_string();
        let mut coin = request("coin");
        coin.prevouts[0].txid = "not a txid".to_string();
        let mut connector = request("connector");
        connector.raw_connector_tx.truncate(10);

        for unreadable in [exit, coin, connector] {
            let id = unreadable.id.as_str();
            let stored = store.insert(&unreadable, &leaves(&[id])).await;
            assert!(stored.is_err(), "{id}");
        }

        assert!(store.list(10).await.unwrap().is_empty());
        assert!(!in_open_request(store, &["exit", "coin", "connector"]).await);
    }

    pub(crate) async fn a_request_for_more_sats_than_the_store_can_hold_is_not_stored(
        store: &dyn CoopExitStore,
    ) {
        let most = i64::MAX.cast_unsigned();
        let largest = CoopExitRecord {
            amount_sats: most,
            fee_sats: most,
            ..request("largest")
        };
        store.insert(&largest, &[]).await.unwrap();
        let too_many = most.saturating_add(1);
        let amount = CoopExitRecord {
            amount_sats: too_many,
            ..request("amount")
        };
        let fee = CoopExitRecord {
            fee_sats: too_many,
            ..request("fee")
        };

        let amount_stored = store.insert(&amount, &[]).await;
        let fee_stored = store.insert(&fee, &[]).await;

        assert!(amount_stored.is_err());
        assert!(fee_stored.is_err());
        let stored = read(store, "largest").await;
        assert_eq!((stored.amount_sats, stored.fee_sats), (most, most));
        assert_eq!(ids(store.list(10).await.unwrap()), ["largest"]);
    }

    pub(crate) async fn a_request_under_the_id_of_another_is_not_stored(store: &dyn CoopExitStore) {
        let first = request("a");
        store.insert(&first, &leaves(&["a"])).await.unwrap();
        let second = request("a");

        let stored = store.insert(&second, &leaves(&["b"])).await;

        assert!(stored.is_err());
        let kept = read(store, "a").await;
        assert_eq!(kept.user_transfer_id, first.user_transfer_id);
        let by_transfer = store.get_by_transfer_id(&second.user_transfer_id).await;
        assert!(by_transfer.unwrap().is_none());
        assert!(!in_open_request(store, &["b"]).await);
    }

    pub(crate) async fn a_request_naming_a_leaf_twice_is_not_stored(store: &dyn CoopExitStore) {
        let stored = store.insert(&request("a"), &leaves(&["a", "a"])).await;

        assert!(stored.is_err());
        assert!(store.get("a").await.unwrap().is_none());
        assert!(!in_open_request(store, &["a"]).await);
    }
}
