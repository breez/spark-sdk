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
