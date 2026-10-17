use bitcoin::Transaction;
use bitcoin::secp256k1::PublicKey;
use spark::services::TransferId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCredit {
    pub transfer_id: TransferId,
}

#[derive(Debug, Clone)]
pub struct StaticDepositClaimRecord {
    pub id: String,
    pub user_identity_public_key: PublicKey,
    pub txid: String,
    pub vout: u32,
    pub network: spark::Network,
    pub deposit_address: String,
    pub credit_amount_sats: u64,
    pub is_instant: bool,
    pub deposit_amount_sats: u64,
    pub encrypted_deposit_secret_key: String,
    pub quote_signature: String,
    pub user_signature: String,
    pub transfer_id: Option<String>,
    /// Stored before the operators are asked for the credit, so the worker can
    /// settle it if their answer is lost.
    pub pending_credit: Option<PendingCredit>,
    pub utxo_swap_id: Option<String>,
    /// Built with the claim, at the fee rate its fee check used.
    pub spend_tx: Transaction,
    pub signed_spend_transaction: Option<Transaction>,
    pub spend_broadcast_txid: Option<String>,
    pub spend_confirmed: bool,
    /// Set once the SSP gives up on an INSTANT claim whose deposit vanished before it
    /// confirmed.
    pub deposit_lost: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl StaticDepositClaimRecord {
    pub fn is_pending(&self) -> bool {
        self.pending_credit.is_some()
            || (self.transfer_id.is_some() && !self.spend_confirmed && !self.deposit_lost)
    }
}

#[async_trait::async_trait]
pub trait StaticDepositClaimStore: Send + Sync {
    async fn insert(&self, record: &StaticDepositClaimRecord) -> Result<(), String>;
    async fn get(&self, id: &str) -> Result<Option<StaticDepositClaimRecord>, String>;
    async fn get_by_utxo(
        &self,
        txid: &str,
        vout: u32,
    ) -> Result<Option<StaticDepositClaimRecord>, String>;
    async fn get_by_transfer_id(
        &self,
        transfer_id: &str,
    ) -> Result<Option<StaticDepositClaimRecord>, String>;
    /// Also forgets the pending credit.
    async fn set_transfer_id(&self, id: &str, transfer_id: &str) -> Result<(), String>;
    async fn delete(&self, id: &str) -> Result<(), String>;
    /// Also forgets the pending credit.
    async fn set_reserved(
        &self,
        id: &str,
        transfer_id: &str,
        utxo_swap_id: &str,
    ) -> Result<(), String>;
    /// Also forgets the pending credit.
    async fn set_signed_spend_transaction(
        &self,
        id: &str,
        transfer_id: &str,
        transaction: &Transaction,
    ) -> Result<(), String>;
    async fn set_spend_broadcast_txid(&self, id: &str, txid: &str) -> Result<(), String>;
    async fn set_spend_confirmed(&self, id: &str) -> Result<(), String>;
    async fn set_deposit_lost(&self, id: &str) -> Result<(), String>;
    /// The claims for which [`StaticDepositClaimRecord::is_pending`] holds.
    async fn pending(&self) -> Result<Vec<StaticDepositClaimRecord>, String>;
}

#[cfg(test)]
#[derive(Default)]
pub struct InMemoryStaticDepositClaimStore {
    records: tokio::sync::RwLock<std::collections::HashMap<String, StaticDepositClaimRecord>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl StaticDepositClaimStore for InMemoryStaticDepositClaimStore {
    async fn insert(&self, record: &StaticDepositClaimRecord) -> Result<(), String> {
        self.records
            .write()
            .await
            .insert(record.id.clone(), record.clone());
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<StaticDepositClaimRecord>, String> {
        Ok(self.records.read().await.get(id).cloned())
    }

    async fn get_by_utxo(
        &self,
        txid: &str,
        vout: u32,
    ) -> Result<Option<StaticDepositClaimRecord>, String> {
        Ok(self
            .records
            .read()
            .await
            .values()
            .find(|record| record.txid == txid && record.vout == vout)
            .cloned())
    }

    async fn get_by_transfer_id(
        &self,
        transfer_id: &str,
    ) -> Result<Option<StaticDepositClaimRecord>, String> {
        Ok(self
            .records
            .read()
            .await
            .values()
            .find(|record| record.transfer_id.as_deref() == Some(transfer_id))
            .cloned())
    }

    async fn set_transfer_id(&self, id: &str, transfer_id: &str) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.transfer_id = Some(transfer_id.to_string());
            record.pending_credit = None;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), String> {
        self.records.write().await.remove(id);
        Ok(())
    }

    async fn set_reserved(
        &self,
        id: &str,
        transfer_id: &str,
        utxo_swap_id: &str,
    ) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.transfer_id = Some(transfer_id.to_string());
            record.pending_credit = None;
            record.utxo_swap_id = Some(utxo_swap_id.to_string());
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_signed_spend_transaction(
        &self,
        id: &str,
        transfer_id: &str,
        transaction: &Transaction,
    ) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.transfer_id = Some(transfer_id.to_string());
            record.pending_credit = None;
            record.signed_spend_transaction = Some(transaction.clone());
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_spend_broadcast_txid(&self, id: &str, txid: &str) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.spend_broadcast_txid = Some(txid.to_string());
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_spend_confirmed(&self, id: &str) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.spend_confirmed = true;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_deposit_lost(&self, id: &str) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.deposit_lost = true;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn pending(&self) -> Result<Vec<StaticDepositClaimRecord>, String> {
        Ok(self
            .records
            .read()
            .await
            .values()
            .filter(|record| record.is_pending())
            .cloned()
            .collect())
    }
}

#[derive(Debug, Clone)]
pub struct InstantStaticDepositQuoteRecord {
    pub id: String,
    pub txid: String,
    pub vout: u32,
    pub network: spark::Network,
    pub deposit_amount_sats: u64,
    pub credit_amount_sats: u64,
    pub destination_address: String,
    pub quote_signature: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait::async_trait]
pub trait InstantStaticDepositQuoteStore: Send + Sync {
    async fn insert(&self, record: &InstantStaticDepositQuoteRecord) -> Result<(), String>;
    async fn get(&self, id: &str) -> Result<Option<InstantStaticDepositQuoteRecord>, String>;
    async fn delete_created_before(
        &self,
        time: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), String>;
}

#[cfg(test)]
#[derive(Default)]
pub struct InMemoryInstantStaticDepositQuoteStore {
    quotes: tokio::sync::RwLock<std::collections::HashMap<String, InstantStaticDepositQuoteRecord>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl InstantStaticDepositQuoteStore for InMemoryInstantStaticDepositQuoteStore {
    async fn insert(&self, record: &InstantStaticDepositQuoteRecord) -> Result<(), String> {
        self.quotes
            .write()
            .await
            .insert(record.id.clone(), record.clone());
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<InstantStaticDepositQuoteRecord>, String> {
        Ok(self.quotes.read().await.get(id).cloned())
    }

    async fn delete_created_before(
        &self,
        time: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), String> {
        self.quotes
            .write()
            .await
            .retain(|_, quote| quote.created_at >= time);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod scenarios {
    use std::time::Duration;

    use bitcoin::Txid;
    use bitcoin::hashes::{Hash as _, sha256d};

    fn deposit_txid(name: &str) -> Txid {
        Txid::from_raw_hash(sha256d::Hash::hash(name.as_bytes()))
    }

    /// Waits long enough for the store's clock to show a later time.
    async fn later() {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    pub(crate) mod claims {
        use bitcoin::absolute::LockTime;
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        use bitcoin::transaction::Version;
        use bitcoin::{Amount, OutPoint, ScriptBuf, Transaction, TxIn, TxOut, Witness};
        use chrono::{DateTime, Utc};
        use futures::future::BoxFuture;
        use spark::services::TransferId;

        use super::{deposit_txid, later};
        use crate::static_deposit::repository::{
            PendingCredit, StaticDepositClaimRecord, StaticDepositClaimStore,
        };

        #[derive(Debug, Clone, Copy)]
        enum Credit {
            None,
            Pending,
            Made,
        }

        fn key(byte: u8) -> PublicKey {
            SecretKey::from_slice(&[byte; 32])
                .unwrap()
                .public_key(&Secp256k1::new())
        }

        fn new_transfer() -> String {
            TransferId::generate().to_string()
        }

        fn claim(id: &str) -> StaticDepositClaimRecord {
            let deposit = OutPoint::new(deposit_txid(id), 0);
            let now = Utc::now();
            StaticDepositClaimRecord {
                id: id.to_string(),
                user_identity_public_key: key(1),
                txid: deposit.txid.to_string(),
                vout: deposit.vout,
                network: spark::Network::Regtest,
                deposit_address: "bcrt1pdeposit".to_string(),
                credit_amount_sats: 10_000,
                is_instant: false,
                deposit_amount_sats: 11_000,
                encrypted_deposit_secret_key: "deadbeef".to_string(),
                quote_signature: "3045".to_string(),
                user_signature: "3044".to_string(),
                transfer_id: None,
                pending_credit: Some(PendingCredit {
                    transfer_id: TransferId::generate(),
                }),
                utxo_swap_id: None,
                spend_tx: Transaction {
                    version: Version::non_standard(3),
                    lock_time: LockTime::ZERO,
                    input: vec![TxIn {
                        previous_output: deposit,
                        ..Default::default()
                    }],
                    output: vec![TxOut {
                        value: Amount::from_sat(10_700),
                        script_pubkey: ScriptBuf::new(),
                    }],
                },
                signed_spend_transaction: None,
                spend_broadcast_txid: None,
                spend_confirmed: false,
                deposit_lost: false,
                created_at: now,
                updated_at: now,
            }
        }

        fn pending_transfer(claim: &StaticDepositClaimRecord) -> String {
            let credit = claim.pending_credit.as_ref().unwrap();
            credit.transfer_id.to_string()
        }

        fn signed_spend() -> Transaction {
            Transaction {
                version: Version::non_standard(3),
                lock_time: LockTime::ZERO,
                input: vec![TxIn {
                    witness: Witness::from_slice(&[[5; 64]]),
                    ..Default::default()
                }],
                output: Vec::new(),
            }
        }

        async fn credited(store: &dyn StaticDepositClaimStore, id: &str) -> String {
            let claim = claim(id);
            let transfer = pending_transfer(&claim);
            store.insert(&claim).await.unwrap();
            store.set_transfer_id(id, &transfer).await.unwrap();
            transfer
        }

        async fn stored_with(
            store: &dyn StaticDepositClaimStore,
            credit: Credit,
            confirmed: bool,
            lost: bool,
        ) -> String {
            let id = format!("credit {credit:?}, confirmed {confirmed}, lost {lost}");
            let mut claim = claim(&id);
            if matches!(credit, Credit::None) {
                claim.pending_credit = None;
            }
            store.insert(&claim).await.unwrap();
            if matches!(credit, Credit::Made) {
                store.set_transfer_id(&id, &new_transfer()).await.unwrap();
            }
            if confirmed {
                store.set_spend_confirmed(&id).await.unwrap();
            }
            if lost {
                store.set_deposit_lost(&id).await.unwrap();
            }
            id
        }

        fn ids(claims: &[StaticDepositClaimRecord]) -> Vec<String> {
            let mut ids: Vec<String> = claims.iter().map(|claim| claim.id.clone()).collect();
            ids.sort_unstable();
            ids
        }

        async fn pending_ids(store: &dyn StaticDepositClaimStore) -> Vec<String> {
            ids(&store.pending().await.unwrap())
        }

        /// Every write that updates a stored claim.
        fn writes<'a>(
            store: &'a dyn StaticDepositClaimStore,
            id: &'a str,
            transfer: &'a str,
            signed: &'a Transaction,
        ) -> [BoxFuture<'a, Result<(), String>>; 6] {
            [
                store.set_reserved(id, transfer, "swap"),
                store.set_transfer_id(id, transfer),
                store.set_signed_spend_transaction(id, transfer, signed),
                store.set_spend_broadcast_txid(id, "spend"),
                store.set_spend_confirmed(id),
                store.set_deposit_lost(id),
            ]
        }

        pub(crate) async fn a_claim_reads_back_as_it_was_stored(
            store: &dyn StaticDepositClaimStore,
        ) {
            let claim = StaticDepositClaimRecord {
                vout: 2,
                is_instant: true,
                ..claim("a")
            };
            store.insert(&claim).await.unwrap();

            let stored = store.get("a").await.unwrap().unwrap();

            assert_eq!(stored.id, claim.id);
            assert_eq!(
                stored.user_identity_public_key,
                claim.user_identity_public_key
            );
            assert_eq!(stored.txid, claim.txid);
            assert_eq!(stored.vout, claim.vout);
            assert_eq!(stored.network, claim.network);
            assert_eq!(stored.deposit_address, claim.deposit_address);
            assert_eq!(stored.credit_amount_sats, claim.credit_amount_sats);
            assert_eq!(stored.is_instant, claim.is_instant);
            assert_eq!(stored.deposit_amount_sats, claim.deposit_amount_sats);
            assert_eq!(
                stored.encrypted_deposit_secret_key,
                claim.encrypted_deposit_secret_key
            );
            assert_eq!(stored.quote_signature, claim.quote_signature);
            assert_eq!(stored.user_signature, claim.user_signature);
            assert_eq!(stored.pending_credit, claim.pending_credit);
            assert_eq!(stored.spend_tx, claim.spend_tx);
            assert_eq!(stored.transfer_id, None);
            assert_eq!(stored.utxo_swap_id, None);
            assert!(stored.signed_spend_transaction.is_none());
            assert_eq!(stored.spend_broadcast_txid, None);
            assert!(!stored.spend_confirmed);
            assert!(!stored.deposit_lost);
        }

        pub(crate) async fn a_claim_the_store_was_never_given_is_not_found(
            store: &dyn StaticDepositClaimStore,
        ) {
            assert!(store.get("a").await.unwrap().is_none());

            store.insert(&claim("a")).await.unwrap();

            assert!(store.get("b").await.unwrap().is_none());
        }

        pub(crate) async fn each_claim_is_found_by_its_own_deposit_output(
            store: &dyn StaticDepositClaimStore,
        ) {
            let first = claim("first");
            let same_transaction = StaticDepositClaimRecord {
                txid: first.txid.clone(),
                vout: 1,
                ..claim("same transaction")
            };
            let same_index = claim("same index");
            let claims = [&first, &same_transaction, &same_index];
            for claim in claims {
                store.insert(claim).await.unwrap();
            }

            for claim in claims {
                let found = store.get_by_utxo(&claim.txid, claim.vout).await.unwrap();
                assert_eq!(found.unwrap().id, claim.id);
            }
            let unclaimed = store.get_by_utxo(&same_index.txid, 1).await.unwrap();
            assert!(unclaimed.is_none());
        }

        pub(crate) async fn a_claim_with_the_id_of_a_stored_one_is_refused(
            store: &dyn StaticDepositClaimStore,
        ) {
            let first = claim("a");
            store.insert(&first).await.unwrap();
            let second = StaticDepositClaimRecord {
                id: first.id.clone(),
                ..claim("b")
            };

            assert!(store.insert(&second).await.is_err());

            assert_eq!(store.get("a").await.unwrap().unwrap().txid, first.txid);
        }

        pub(crate) async fn a_second_claim_of_a_deposit_output_is_refused(
            store: &dyn StaticDepositClaimStore,
        ) {
            let first = claim("first");
            store.insert(&first).await.unwrap();
            let second = StaticDepositClaimRecord {
                txid: first.txid.clone(),
                vout: first.vout,
                ..claim("second")
            };

            assert!(store.insert(&second).await.is_err());

            assert!(store.get("second").await.unwrap().is_none());
            let claimed = store.get_by_utxo(&first.txid, first.vout).await.unwrap();
            assert_eq!(claimed.unwrap().id, "first");
        }

        pub(crate) async fn a_new_claim_is_stored_as_not_yet_credited(
            store: &dyn StaticDepositClaimStore,
        ) {
            let claim = StaticDepositClaimRecord {
                transfer_id: Some(new_transfer()),
                utxo_swap_id: Some("swap".to_string()),
                signed_spend_transaction: Some(signed_spend()),
                spend_broadcast_txid: Some("spend".to_string()),
                spend_confirmed: true,
                deposit_lost: true,
                ..claim("a")
            };
            store.insert(&claim).await.unwrap();

            let stored = store.get("a").await.unwrap().unwrap();

            assert_eq!(stored.pending_credit, claim.pending_credit);
            assert_eq!(stored.transfer_id, None);
            assert_eq!(stored.utxo_swap_id, None);
            assert!(stored.signed_spend_transaction.is_none());
            assert_eq!(stored.spend_broadcast_txid, None);
            assert!(!stored.spend_confirmed);
            assert!(!stored.deposit_lost);
        }

        pub(crate) async fn the_store_sets_the_times_of_a_new_claim(
            store: &dyn StaticDepositClaimStore,
        ) {
            let claim = StaticDepositClaimRecord {
                created_at: DateTime::UNIX_EPOCH,
                updated_at: DateTime::UNIX_EPOCH,
                ..claim("a")
            };
            store.insert(&claim).await.unwrap();

            let stored = store.get("a").await.unwrap().unwrap();

            let age = Utc::now().signed_duration_since(stored.created_at);
            assert!(age.abs() < chrono::Duration::days(1), "{age}");
            assert_eq!(stored.updated_at, stored.created_at);
        }

        pub(crate) async fn a_claim_with_a_pending_credit_is_pending(
            store: &dyn StaticDepositClaimStore,
        ) {
            let claim = claim("a");
            store.insert(&claim).await.unwrap();

            let pending = store.pending().await.unwrap();

            assert_eq!(ids(&pending), ["a"]);
            assert_eq!(pending[0].pending_credit, claim.pending_credit);
        }

        pub(crate) async fn a_claim_with_no_credit_is_not_pending(
            store: &dyn StaticDepositClaimStore,
        ) {
            let claim = StaticDepositClaimRecord {
                pending_credit: None,
                ..claim("a")
            };
            store.insert(&claim).await.unwrap();

            assert!(store.pending().await.unwrap().is_empty());
            let stored = store.get("a").await.unwrap().unwrap();
            assert_eq!(stored.pending_credit, None);
        }

        pub(crate) async fn a_credited_claim_has_its_transfer_and_no_pending_credit(
            store: &dyn StaticDepositClaimStore,
        ) {
            let transfer = credited(store, "a").await;

            let stored = store.get("a").await.unwrap().unwrap();

            assert_eq!(stored.transfer_id, Some(transfer));
            assert_eq!(stored.pending_credit, None);
        }

        pub(crate) async fn a_reserved_claim_has_its_transfer_and_swap_and_no_pending_credit(
            store: &dyn StaticDepositClaimStore,
        ) {
            store.insert(&claim("a")).await.unwrap();
            let transfer = new_transfer();

            store.set_reserved("a", &transfer, "swap").await.unwrap();

            let stored = store.get("a").await.unwrap().unwrap();
            assert_eq!(stored.transfer_id, Some(transfer));
            assert_eq!(stored.utxo_swap_id.as_deref(), Some("swap"));
            assert_eq!(stored.pending_credit, None);
            assert!(stored.signed_spend_transaction.is_none());
        }

        pub(crate) async fn a_claim_whose_spend_is_co_signed_has_its_transfer_and_signed_spend(
            store: &dyn StaticDepositClaimStore,
        ) {
            store.insert(&claim("a")).await.unwrap();
            let (transfer, signed) = (new_transfer(), signed_spend());

            store
                .set_signed_spend_transaction("a", &transfer, &signed)
                .await
                .unwrap();

            let stored = store.get("a").await.unwrap().unwrap();
            assert_eq!(stored.transfer_id, Some(transfer));
            assert_eq!(stored.pending_credit, None);
            assert_eq!(stored.signed_spend_transaction, Some(signed));
        }

        pub(crate) async fn a_reserved_claim_keeps_its_swap_when_its_spend_is_co_signed(
            store: &dyn StaticDepositClaimStore,
        ) {
            store.insert(&claim("a")).await.unwrap();
            let transfer = new_transfer();
            store.set_reserved("a", &transfer, "swap").await.unwrap();

            store
                .set_signed_spend_transaction("a", &transfer, &signed_spend())
                .await
                .unwrap();

            let stored = store.get("a").await.unwrap().unwrap();
            assert_eq!(stored.utxo_swap_id.as_deref(), Some("swap"));
            assert_eq!(stored.transfer_id, Some(transfer));
            assert!(stored.signed_spend_transaction.is_some());
        }

        pub(crate) async fn a_claim_stays_pending_until_its_spend_confirms(
            store: &dyn StaticDepositClaimStore,
        ) {
            let transfer = credited(store, "a").await;
            assert_eq!(pending_ids(store).await, ["a"]);

            store
                .set_signed_spend_transaction("a", &transfer, &signed_spend())
                .await
                .unwrap();
            assert_eq!(pending_ids(store).await, ["a"]);

            store.set_spend_broadcast_txid("a", "spend").await.unwrap();
            assert_eq!(pending_ids(store).await, ["a"]);

            store.set_spend_confirmed("a").await.unwrap();
            assert!(pending_ids(store).await.is_empty());
            assert!(store.get("a").await.unwrap().unwrap().spend_confirmed);
        }

        pub(crate) async fn a_claim_whose_spend_was_broadcast_has_the_txid_of_the_spend(
            store: &dyn StaticDepositClaimStore,
        ) {
            let claim = claim("a");
            let spend = claim.spend_tx.compute_txid().to_string();
            store.insert(&claim).await.unwrap();

            store.set_spend_broadcast_txid("a", &spend).await.unwrap();

            let stored = store.get("a").await.unwrap().unwrap();
            assert_eq!(stored.spend_broadcast_txid, Some(spend));
            assert!(!stored.spend_confirmed);
        }

        pub(crate) async fn a_claim_whose_deposit_is_lost_is_no_longer_pending(
            store: &dyn StaticDepositClaimStore,
        ) {
            let claim = StaticDepositClaimRecord {
                is_instant: true,
                ..claim("a")
            };
            store.insert(&claim).await.unwrap();
            store
                .set_reserved("a", &new_transfer(), "swap")
                .await
                .unwrap();
            assert_eq!(pending_ids(store).await, ["a"]);

            store.set_deposit_lost("a").await.unwrap();

            assert!(pending_ids(store).await.is_empty());
            assert!(store.get("a").await.unwrap().unwrap().deposit_lost);
        }

        pub(crate) async fn the_pending_claims_are_those_the_record_calls_pending(
            store: &dyn StaticDepositClaimStore,
        ) {
            let mut stored = Vec::new();
            for credit in [Credit::None, Credit::Pending, Credit::Made] {
                for (confirmed, lost) in
                    [(false, false), (true, false), (false, true), (true, true)]
                {
                    stored.push(stored_with(store, credit, confirmed, lost).await);
                }
            }

            let pending = pending_ids(store).await;

            for id in &stored {
                let claim = store.get(id).await.unwrap().unwrap();
                assert_eq!(pending.contains(id), claim.is_pending(), "{id}");
            }
            // The four claims with a pending credit, and the credited claim whose
            // spend is unconfirmed and whose deposit is not lost.
            assert_eq!(pending.len(), 5);
        }

        pub(crate) async fn pending_claims_are_listed_oldest_first(
            store: &dyn StaticDepositClaimStore,
        ) {
            for id in ["c", "a", "b"] {
                store.insert(&claim(id)).await.unwrap();
            }
            // After this write, the oldest claim is also the claim written to last.
            store.set_transfer_id("c", &new_transfer()).await.unwrap();

            let pending = store.pending().await.unwrap();

            let listed: Vec<&str> = pending.iter().map(|claim| claim.id.as_str()).collect();
            assert_eq!(listed, ["c", "a", "b"]);
        }

        pub(crate) async fn the_deposit_output_of_a_deleted_claim_can_be_claimed_again(
            store: &dyn StaticDepositClaimStore,
        ) {
            let first = claim("first");
            store.insert(&first).await.unwrap();
            store.delete("first").await.unwrap();
            let second = StaticDepositClaimRecord {
                txid: first.txid.clone(),
                vout: first.vout,
                ..claim("second")
            };

            store.insert(&second).await.unwrap();

            let claimed = store.get_by_utxo(&first.txid, first.vout).await.unwrap();
            assert_eq!(claimed.unwrap().id, "second");
        }

        pub(crate) async fn a_write_to_one_claim_leaves_another_as_it_was(
            store: &dyn StaticDepositClaimStore,
        ) {
            let other = claim("other");
            store.insert(&other).await.unwrap();
            store.insert(&claim("written")).await.unwrap();
            let before = store.get("other").await.unwrap().unwrap();
            let (transfer, signed) = (new_transfer(), signed_spend());

            for write in writes(store, "written", &transfer, &signed) {
                write.await.unwrap();
            }
            store.delete("written").await.unwrap();

            let after = store.get("other").await.unwrap().unwrap();
            assert_eq!(after.pending_credit, other.pending_credit);
            assert_eq!(after.transfer_id, None);
            assert_eq!(after.utxo_swap_id, None);
            assert!(after.signed_spend_transaction.is_none());
            assert_eq!(after.spend_broadcast_txid, None);
            assert!(!after.spend_confirmed);
            assert!(!after.deposit_lost);
            assert_eq!(after.updated_at, before.updated_at);
        }

        pub(crate) async fn a_claim_keeps_the_time_it_was_created(
            store: &dyn StaticDepositClaimStore,
        ) {
            store.insert(&claim("a")).await.unwrap();
            let created = store.get("a").await.unwrap().unwrap().created_at;
            let (transfer, signed) = (new_transfer(), signed_spend());

            for write in writes(store, "a", &transfer, &signed) {
                write.await.unwrap();
            }

            let stored = store.get("a").await.unwrap().unwrap();
            assert_eq!(stored.created_at, created);
        }

        pub(crate) async fn each_write_to_a_claim_moves_the_time_it_last_changed(
            store: &dyn StaticDepositClaimStore,
        ) {
            store.insert(&claim("a")).await.unwrap();
            let last_changed = async || store.get("a").await.unwrap().unwrap().updated_at;
            let mut changes: Vec<DateTime<Utc>> = vec![last_changed().await];
            let (transfer, signed) = (new_transfer(), signed_spend());

            for write in writes(store, "a", &transfer, &signed) {
                later().await;
                write.await.unwrap();
                changes.push(last_changed().await);
            }

            assert!(
                changes.is_sorted_by(|before, after| before < after),
                "{changes:?}"
            );
        }
    }

    pub(crate) mod quotes {
        use chrono::{DateTime, Utc};

        use super::{deposit_txid, later};
        use crate::static_deposit::repository::{
            InstantStaticDepositQuoteRecord, InstantStaticDepositQuoteStore,
        };

        /// Postgres stores times to the microsecond.
        const STEP: chrono::Duration = chrono::Duration::microseconds(1);

        fn quote(id: &str) -> InstantStaticDepositQuoteRecord {
            let now = Utc::now();
            InstantStaticDepositQuoteRecord {
                id: id.to_string(),
                txid: deposit_txid(id).to_string(),
                vout: 1,
                network: spark::Network::Regtest,
                deposit_amount_sats: 50_000,
                credit_amount_sats: 49_000,
                destination_address: "bcrt1pexampleaddress".to_string(),
                quote_signature: "3045".to_string(),
                created_at: now,
                updated_at: now,
            }
        }

        async fn stored(
            store: &dyn InstantStaticDepositQuoteStore,
            id: &str,
        ) -> InstantStaticDepositQuoteRecord {
            store.insert(&quote(id)).await.unwrap();
            store.get(id).await.unwrap().unwrap()
        }

        async fn kept<'a>(
            store: &dyn InstantStaticDepositQuoteStore,
            ids: &[&'a str],
        ) -> Vec<&'a str> {
            let mut kept = Vec::new();
            for id in ids {
                if store.get(id).await.unwrap().is_some() {
                    kept.push(*id);
                }
            }
            kept
        }

        pub(crate) async fn a_quote_reads_back_as_it_was_stored(
            store: &dyn InstantStaticDepositQuoteStore,
        ) {
            let quote = quote("a");
            store.insert(&quote).await.unwrap();

            let stored = store.get("a").await.unwrap().unwrap();

            assert_eq!(stored.id, quote.id);
            assert_eq!(stored.txid, quote.txid);
            assert_eq!(stored.vout, quote.vout);
            assert_eq!(stored.network, quote.network);
            assert_eq!(stored.deposit_amount_sats, quote.deposit_amount_sats);
            assert_eq!(stored.credit_amount_sats, quote.credit_amount_sats);
            assert_eq!(stored.destination_address, quote.destination_address);
            assert_eq!(stored.quote_signature, quote.quote_signature);
        }

        pub(crate) async fn a_quote_the_store_was_never_given_is_not_found(
            store: &dyn InstantStaticDepositQuoteStore,
        ) {
            assert!(store.get("a").await.unwrap().is_none());

            store.insert(&quote("a")).await.unwrap();

            assert!(store.get("b").await.unwrap().is_none());
        }

        pub(crate) async fn a_quote_with_the_id_of_a_stored_one_is_refused(
            store: &dyn InstantStaticDepositQuoteStore,
        ) {
            let first = quote("a");
            store.insert(&first).await.unwrap();
            let second = InstantStaticDepositQuoteRecord {
                credit_amount_sats: 48_000,
                ..quote("a")
            };

            assert!(store.insert(&second).await.is_err());

            let stored = store.get("a").await.unwrap().unwrap();
            assert_eq!(stored.credit_amount_sats, first.credit_amount_sats);
        }

        pub(crate) async fn two_quotes_of_one_deposit_output_are_both_kept(
            store: &dyn InstantStaticDepositQuoteStore,
        ) {
            let first = quote("first");
            let second = InstantStaticDepositQuoteRecord {
                txid: first.txid.clone(),
                credit_amount_sats: 48_000,
                ..quote("second")
            };

            store.insert(&first).await.unwrap();
            store.insert(&second).await.unwrap();

            let stored = store.get("first").await.unwrap().unwrap();
            assert_eq!(stored.credit_amount_sats, first.credit_amount_sats);
            let stored = store.get("second").await.unwrap().unwrap();
            assert_eq!(stored.credit_amount_sats, second.credit_amount_sats);
        }

        pub(crate) async fn the_store_sets_the_times_of_a_new_quote(
            store: &dyn InstantStaticDepositQuoteStore,
        ) {
            let quote = InstantStaticDepositQuoteRecord {
                created_at: DateTime::UNIX_EPOCH,
                updated_at: DateTime::UNIX_EPOCH,
                ..quote("a")
            };
            store.insert(&quote).await.unwrap();

            let stored = store.get("a").await.unwrap().unwrap();

            let age = Utc::now().signed_duration_since(stored.created_at);
            assert!(age.abs() < chrono::Duration::days(1), "{age}");
            assert_eq!(stored.updated_at, stored.created_at);
        }

        pub(crate) async fn a_quote_created_before_the_given_time_is_deleted(
            store: &dyn InstantStaticDepositQuoteStore,
        ) {
            let quote = stored(store, "a").await;
            let a_step_later = quote.created_at.checked_add_signed(STEP).unwrap();

            store.delete_created_before(a_step_later).await.unwrap();

            assert!(store.get("a").await.unwrap().is_none());
        }

        pub(crate) async fn a_quote_created_at_the_given_time_is_kept(
            store: &dyn InstantStaticDepositQuoteStore,
        ) {
            let quote = stored(store, "a").await;

            store.delete_created_before(quote.created_at).await.unwrap();

            assert!(store.get("a").await.unwrap().is_some());
        }

        pub(crate) async fn only_the_quotes_created_before_the_given_time_are_deleted(
            store: &dyn InstantStaticDepositQuoteStore,
        ) {
            store.insert(&quote("oldest")).await.unwrap();
            later().await;
            store.insert(&quote("older")).await.unwrap();
            later().await;
            let newer = stored(store, "newer").await;
            later().await;
            store.insert(&quote("newest")).await.unwrap();

            store.delete_created_before(newer.created_at).await.unwrap();

            let kept = kept(store, &["oldest", "older", "newer", "newest"]).await;
            assert_eq!(kept, ["newer", "newest"]);
        }
    }
}
