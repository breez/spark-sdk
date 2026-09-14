use bitcoin::Transaction;
use bitcoin::secp256k1::PublicKey;
use spark::operator::rpc::spark as pb;
use spark::services::TransferId;
use spark::signer::FrostSigningCommitmentsWithNonces;

use crate::handover::HandoverReservation;

/// The operators' answer to co-signing a claim's spend. Their signature shares make
/// a valid signature only with a share made from the prep's nonce.
#[derive(Debug, Clone)]
pub struct StaticDepositSpendContext {
    pub verifying_public_key: PublicKey,
    pub signing_result: pb::SigningResult,
}

/// Built with the claim, at the fee rate its fee check used. Kept across retries of
/// the call that co-signs it: the operators answer a repeated claim of a completed
/// INSTANT swap with the signing result they made first.
#[derive(Debug, Clone)]
pub struct StaticDepositSpendPrep {
    pub spend_tx: Transaction,
    pub nonce: FrostSigningCommitmentsWithNonces,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCredit {
    pub transfer_id: TransferId,
    pub reservation: HandoverReservation,
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
    pub spend_prep: StaticDepositSpendPrep,
    pub spend_context: Option<StaticDepositSpendContext>,
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
    async fn set_spend_context(
        &self,
        id: &str,
        transfer_id: &str,
        context: &StaticDepositSpendContext,
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

    async fn set_spend_context(
        &self,
        id: &str,
        transfer_id: &str,
        context: &StaticDepositSpendContext,
    ) -> Result<(), String> {
        if let Some(record) = self.records.write().await.get_mut(id) {
            record.transfer_id = Some(transfer_id.to_string());
            record.pending_credit = None;
            record.spend_context = Some(context.clone());
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
mod tests {
    use super::{
        InMemoryInstantStaticDepositQuoteStore, InMemoryStaticDepositClaimStore,
        InstantStaticDepositQuoteRecord, InstantStaticDepositQuoteStore, StaticDepositClaimRecord,
        StaticDepositClaimStore, StaticDepositSpendContext,
    };
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use spark::operator::rpc::spark as pb;
    use spark::signer::{DefaultSigner, Signer};

    use super::StaticDepositSpendPrep;

    fn pubkey(byte: u8) -> PublicKey {
        let secp = Secp256k1::new();
        PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).expect("secret"))
    }

    async fn record(id: &str, txid: &str) -> StaticDepositClaimRecord {
        StaticDepositClaimRecord {
            id: id.to_string(),
            user_identity_public_key: pubkey(1),
            txid: txid.to_string(),
            vout: 0,
            network: spark::Network::Regtest,
            deposit_address: "bcrt1pdeposit".to_string(),
            credit_amount_sats: 10_000,
            is_instant: false,
            deposit_amount_sats: 11_000,
            encrypted_deposit_secret_key: "deadbeef".to_string(),
            quote_signature: "3045".to_string(),
            user_signature: "3044".to_string(),
            transfer_id: None,
            pending_credit: None,
            utxo_swap_id: None,
            spend_prep: StaticDepositSpendPrep {
                spend_tx: bitcoin::Transaction {
                    version: bitcoin::transaction::Version::non_standard(3),
                    lock_time: bitcoin::absolute::LockTime::ZERO,
                    input: Vec::new(),
                    output: Vec::new(),
                },
                nonce: DefaultSigner::new(&[7u8; 32], spark::Network::Regtest)
                    .expect("signer")
                    .generate_random_signing_commitment()
                    .await
                    .expect("nonce"),
            },
            spend_context: None,
            spend_broadcast_txid: None,
            spend_confirmed: false,
            deposit_lost: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn spend_context() -> StaticDepositSpendContext {
        StaticDepositSpendContext {
            verifying_public_key: pubkey(2),
            signing_result: pb::SigningResult::default(),
        }
    }

    fn instant_quote(id: &str, txid: &str) -> InstantStaticDepositQuoteRecord {
        InstantStaticDepositQuoteRecord {
            id: id.to_string(),
            txid: txid.to_string(),
            vout: 1,
            network: spark::Network::Regtest,
            deposit_amount_sats: 50_000,
            credit_amount_sats: 49_000,
            destination_address: "bcrt1pexampleaddress".to_string(),
            quote_signature: "3045".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn lifecycle_and_pending_filter() {
        let store = InMemoryStaticDepositClaimStore::default();
        store.insert(&record("s1", "aa").await).await.unwrap();

        assert!(store.pending().await.unwrap().is_empty());

        assert_eq!(
            store.get_by_utxo("aa", 0).await.unwrap().unwrap().id,
            "s1".to_string()
        );
        assert!(store.get_by_utxo("aa", 1).await.unwrap().is_none());

        store.set_transfer_id("s1", "transfer-1").await.unwrap();
        assert_eq!(store.pending().await.unwrap().len(), 1);
        let stored = store.get("s1").await.unwrap().unwrap();
        assert_eq!(stored.transfer_id.as_deref(), Some("transfer-1"));

        assert_eq!(
            store
                .get_by_transfer_id("transfer-1")
                .await
                .unwrap()
                .unwrap()
                .id,
            "s1".to_string()
        );
        assert!(store.get_by_transfer_id("nope").await.unwrap().is_none());

        store
            .set_spend_context("s1", "transfer-1", &spend_context())
            .await
            .unwrap();
        assert_eq!(store.pending().await.unwrap().len(), 1);
        store
            .set_spend_broadcast_txid("s1", "spend-txid")
            .await
            .unwrap();
        assert_eq!(store.pending().await.unwrap().len(), 1);
        store.set_spend_confirmed("s1").await.unwrap();
        assert!(store.pending().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn instant_claim_state_machine() {
        let store = InMemoryStaticDepositClaimStore::default();
        let mut rec = record("i1", "bb").await;
        rec.is_instant = true;
        store.insert(&rec).await.unwrap();

        assert!(store.pending().await.unwrap().is_empty());

        store
            .set_reserved("i1", "transfer-i1", "swap-i1")
            .await
            .unwrap();
        let reserved = store.get("i1").await.unwrap().unwrap();
        assert_eq!(reserved.transfer_id.as_deref(), Some("transfer-i1"));
        assert_eq!(reserved.utxo_swap_id.as_deref(), Some("swap-i1"));
        assert!(reserved.spend_context.is_none());
        assert_eq!(store.pending().await.unwrap().len(), 1);

        store.set_deposit_lost("i1").await.unwrap();
        assert!(store.pending().await.unwrap().is_empty());

        assert_eq!(
            store.get_by_utxo("bb", 0).await.unwrap().unwrap().id,
            "i1".to_string()
        );
    }

    #[tokio::test]
    async fn instant_quote_store_round_trip() {
        let store = InMemoryInstantStaticDepositQuoteStore::default();
        assert!(store.get("q1").await.unwrap().is_none());
        store.insert(&instant_quote("q1", "cc")).await.unwrap();
        let loaded = store.get("q1").await.unwrap().unwrap();
        assert_eq!(loaded.txid, "cc");
        assert_eq!(loaded.vout, 1);
        assert_eq!(loaded.deposit_amount_sats, 50_000);
        assert_eq!(loaded.credit_amount_sats, 49_000);
        assert_eq!(loaded.destination_address, "bcrt1pexampleaddress");
    }
}
