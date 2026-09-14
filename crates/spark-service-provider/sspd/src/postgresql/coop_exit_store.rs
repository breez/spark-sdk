#![allow(clippy::needless_raw_string_hashes)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bitcoin::secp256k1::PublicKey;
use spark::services::TransferId;
use sqlx::PgPool;

use crate::coop_exit::repository::{CoopExitRecord, CoopExitStore};
use crate::wallet::onchain::record_spends;

#[derive(sqlx::FromRow)]
struct CoopExitRow {
    id: String,
    user_identity_public_key: Vec<u8>,
    user_transfer_id: String,
    withdrawal_address: String,
    amount_sats: i64,
    fee_sats: i64,
    raw_coop_exit_tx: Vec<u8>,
    raw_connector_tx: Vec<u8>,
    coop_exit_txid: String,
    prevouts: String,
    completed: bool,
    broadcast_txid: Option<String>,
    leaves_claimed: bool,
    settled: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<CoopExitRow> for CoopExitRecord {
    type Error = String;

    fn try_from(row: CoopExitRow) -> Result<Self, String> {
        Ok(CoopExitRecord {
            id: row.id,
            user_identity_public_key: PublicKey::from_slice(&row.user_identity_public_key)
                .map_err(|e| format!("invalid user public key: {e}"))?,
            user_transfer_id: TransferId::from_str(&row.user_transfer_id)
                .map_err(|e| format!("invalid transfer id: {e}"))?,
            withdrawal_address: row.withdrawal_address,
            amount_sats: u64::try_from(row.amount_sats)
                .map_err(|e| format!("invalid amount: {e}"))?,
            fee_sats: u64::try_from(row.fee_sats).map_err(|e| format!("invalid fee: {e}"))?,
            raw_coop_exit_tx: row.raw_coop_exit_tx,
            raw_connector_tx: row.raw_connector_tx,
            coop_exit_txid: row.coop_exit_txid,
            prevouts: serde_json::from_str(&row.prevouts)
                .map_err(|e| format!("invalid prevouts json: {e}"))?,
            completed: row.completed,
            broadcast_txid: row.broadcast_txid,
            leaves_claimed: row.leaves_claimed,
            settled: row.settled,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn sats_to_i64(sats: u64) -> Result<i64, String> {
    i64::try_from(sats).map_err(|e| format!("amount exceeds i64: {e}"))
}

pub struct PostgresCoopExitStore {
    pool: Arc<PgPool>,
}

impl PostgresCoopExitStore {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CoopExitStore for PostgresCoopExitStore {
    async fn insert(&self, record: &CoopExitRecord, leaf_ids: &[String]) -> Result<(), String> {
        let prevouts_json = serde_json::to_string(&record.prevouts)
            .map_err(|e| format!("failed to serialize prevouts: {e}"))?;
        let txid = bitcoin::Txid::from_str(&record.coop_exit_txid)
            .map_err(|e| format!("invalid coop exit txid: {e}"))?;
        let outpoints = record
            .prevouts
            .iter()
            .map(|p| {
                Ok(bitcoin::OutPoint {
                    txid: bitcoin::Txid::from_str(&p.txid)
                        .map_err(|e| format!("invalid prevout txid: {e}"))?,
                    vout: p.vout,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut db = self.pool.begin().await.map_err(|e| e.to_string())?;
        sqlx::query(
            r#"INSERT INTO brz_ssp_coop_exit_requests
               (id, user_identity_public_key, user_transfer_id, withdrawal_address,
                amount_sats, fee_sats, raw_coop_exit_tx, raw_connector_tx, coop_exit_txid,
                prevouts)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        )
        .bind(&record.id)
        .bind(record.user_identity_public_key.serialize().to_vec())
        .bind(record.user_transfer_id.to_string())
        .bind(&record.withdrawal_address)
        .bind(sats_to_i64(record.amount_sats)?)
        .bind(sats_to_i64(record.fee_sats)?)
        .bind(record.raw_coop_exit_tx.as_slice())
        .bind(record.raw_connector_tx.as_slice())
        .bind(&record.coop_exit_txid)
        .bind(prevouts_json)
        .execute(&mut *db)
        .await
        .map_err(|e| e.to_string())?;
        record_spends(&mut db, &txid, &outpoints)
            .await
            .map_err(|e| e.to_string())?;
        // The connector tx's spend is recorded too: if the wallet spent the exit's
        // connector output, the connector tx and the leaf refunds built on it could
        // never confirm.
        let connector = connector_tx(&record.raw_connector_tx)?;
        let connector_inputs: Vec<bitcoin::OutPoint> = connector
            .input
            .iter()
            .map(|input| input.previous_output)
            .collect();
        record_spends(&mut db, &connector.compute_txid(), &connector_inputs)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query(
            r#"INSERT INTO brz_ssp_coop_exit_leaves (leaf_id, request_id)
               SELECT UNNEST($1::text[]), $2"#,
        )
        .bind(leaf_ids)
        .bind(&record.id)
        .execute(&mut *db)
        .await
        .map_err(|e| e.to_string())?;
        db.commit().await.map_err(|e| e.to_string())
    }

    async fn any_leaf_in_open_request(&self, leaf_ids: &[String]) -> Result<bool, String> {
        sqlx::query_scalar(
            r#"SELECT EXISTS (SELECT 1 FROM brz_ssp_coop_exit_leaves WHERE leaf_id = ANY($1))"#,
        )
        .bind(leaf_ids)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())
    }

    async fn abandon(&self, id: &str) -> Result<(), String> {
        let mut db = self.pool.begin().await.map_err(|e| e.to_string())?;
        // Deleting the request first locks its row, so a concurrent completion
        // either lands first and keeps the request and its coins, or finds the
        // request gone.
        let abandoned: Option<(String, Vec<u8>)> = sqlx::query_as(
            r#"DELETE FROM brz_ssp_coop_exit_requests WHERE id = $1 AND NOT completed
               RETURNING coop_exit_txid, raw_connector_tx"#,
        )
        .bind(id)
        .fetch_optional(&mut *db)
        .await
        .map_err(|e| e.to_string())?;
        if let Some((txid, raw_connector_tx)) = abandoned {
            let connector_txid = connector_tx(&raw_connector_tx)?.compute_txid().to_string();
            sqlx::query(r#"DELETE FROM brz_ssp_wallet_spends WHERE spending_txid = ANY($1)"#)
                .bind(vec![txid, connector_txid])
                .execute(&mut *db)
                .await
                .map_err(|e| e.to_string())?;
        }
        db.commit().await.map_err(|e| e.to_string())
    }

    async fn get_by_transfer_id(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<CoopExitRecord>, String> {
        sqlx::query_as::<_, CoopExitRow>(
            r#"SELECT * FROM brz_ssp_coop_exit_requests WHERE user_transfer_id = $1"#,
        )
        .bind(transfer_id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(CoopExitRecord::try_from)
        .transpose()
    }

    async fn incomplete_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<CoopExitRecord>, String> {
        sqlx::query_as::<_, CoopExitRow>(
            r#"SELECT * FROM brz_ssp_coop_exit_requests
               WHERE NOT completed AND created_at < $1
               ORDER BY created_at"#,
        )
        .bind(cutoff)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(CoopExitRecord::try_from)
        .collect()
    }

    async fn set_completed(&self, id: &str) -> Result<(), String> {
        let mut db = self.pool.begin().await.map_err(|e| e.to_string())?;
        sqlx::query(
            r#"UPDATE brz_ssp_coop_exit_requests SET completed = TRUE, updated_at = NOW() WHERE id = $1"#,
        )
        .bind(id)
        .execute(&mut *db)
        .await
        .map_err(|e| e.to_string())?;
        sqlx::query(r#"DELETE FROM brz_ssp_coop_exit_leaves WHERE request_id = $1"#)
            .bind(id)
            .execute(&mut *db)
            .await
            .map_err(|e| e.to_string())?;
        db.commit().await.map_err(|e| e.to_string())
    }

    async fn set_settled(&self, id: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_coop_exit_requests SET settled = TRUE, updated_at = NOW() WHERE id = $1"#,
        )
        .bind(id)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_broadcast_txid(&self, id: &str, txid: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_coop_exit_requests SET broadcast_txid = $2, updated_at = NOW() WHERE id = $1"#,
        )
        .bind(id)
        .bind(txid)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_leaves_claimed(&self, id: &str) -> Result<(), String> {
        let mut db = self.pool.begin().await.map_err(|e| e.to_string())?;
        let raw_connector_tx: Option<Vec<u8>> = sqlx::query_scalar(
            r#"UPDATE brz_ssp_coop_exit_requests SET leaves_claimed = TRUE, updated_at = NOW()
               WHERE id = $1 RETURNING raw_connector_tx"#,
        )
        .bind(id)
        .fetch_optional(&mut *db)
        .await
        .map_err(|e| e.to_string())?;
        if let Some(raw_connector_tx) = raw_connector_tx {
            sqlx::query(r#"DELETE FROM brz_ssp_wallet_spends WHERE spending_txid = $1"#)
                .bind(connector_tx(&raw_connector_tx)?.compute_txid().to_string())
                .execute(&mut *db)
                .await
                .map_err(|e| e.to_string())?;
        }
        db.commit().await.map_err(|e| e.to_string())
    }

    async fn get(&self, id: &str) -> Result<Option<CoopExitRecord>, String> {
        sqlx::query_as::<_, CoopExitRow>(
            r#"SELECT * FROM brz_ssp_coop_exit_requests WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(CoopExitRecord::try_from)
        .transpose()
    }

    async fn pending(&self) -> Result<Vec<CoopExitRecord>, String> {
        sqlx::query_as::<_, CoopExitRow>(
            r#"SELECT * FROM brz_ssp_coop_exit_requests
               WHERE completed AND NOT settled
               ORDER BY created_at"#,
        )
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(CoopExitRecord::try_from)
        .collect()
    }

    async fn list(&self, limit: u32) -> Result<Vec<CoopExitRecord>, String> {
        sqlx::query_as::<_, CoopExitRow>(
            r#"SELECT * FROM brz_ssp_coop_exit_requests
               ORDER BY created_at DESC
               LIMIT $1"#,
        )
        .bind(i64::from(limit))
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(CoopExitRecord::try_from)
        .collect()
    }
}

fn connector_tx(raw: &[u8]) -> Result<bitcoin::Transaction, String> {
    bitcoin::consensus::deserialize(raw).map_err(|e| format!("invalid connector tx: {e}"))
}
