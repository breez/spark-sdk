#![allow(clippy::needless_raw_string_hashes)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use spark::services::{Preimage, TransferId};
use sqlx::PgPool;

use crate::handover::HandoverReservation;
use crate::lightning::node::LightningPaymentId;
use crate::lightning::repository::{
    HoldInvoiceStatus, HtlcOutput, LightningReceiveRecord, LightningSendRecord, LightningStore,
    SendPaymentStatus,
};

const SEND_PENDING: &str = "pending";
const SEND_SUCCEEDED: &str = "succeeded";
const SEND_FAILED: &str = "failed";
const INVOICE_PENDING: &str = "pending";
const INVOICE_SETTLED: &str = "settled";
const INVOICE_CANCELLED: &str = "cancelled";
const INVOICE_FAILED: &str = "failed";

fn invoice_status_str(status: HoldInvoiceStatus) -> &'static str {
    match status {
        HoldInvoiceStatus::Pending => INVOICE_PENDING,
        HoldInvoiceStatus::Settled => INVOICE_SETTLED,
        HoldInvoiceStatus::Cancelled => INVOICE_CANCELLED,
        HoldInvoiceStatus::Failed => INVOICE_FAILED,
    }
}

#[derive(sqlx::FromRow)]
struct SendRow {
    id: String,
    user_identity_public_key: Vec<u8>,
    encoded_invoice: String,
    payment_hash: Vec<u8>,
    amount_sats: i64,
    fee_sats: i64,
    user_transfer_id: String,
    htlc_address: String,
    idempotency_key: Option<String>,
    ln_payment_id: Option<String>,
    preimage: Option<Vec<u8>>,
    payment_status: String,
    leaves_claimed: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<SendRow> for LightningSendRecord {
    type Error = String;

    fn try_from(row: SendRow) -> Result<Self, String> {
        let payment_status = match row.payment_status.as_str() {
            SEND_PENDING => SendPaymentStatus::Pending,
            SEND_SUCCEEDED => SendPaymentStatus::Succeeded,
            SEND_FAILED => SendPaymentStatus::Failed,
            other => return Err(format!("invalid send payment_status: {other}")),
        };
        Ok(LightningSendRecord {
            id: row.id,
            user_identity_public_key: PublicKey::from_slice(&row.user_identity_public_key)
                .map_err(|e| format!("invalid user public key: {e}"))?,
            encoded_invoice: row.encoded_invoice,
            payment_hash: sha256::Hash::from_slice(&row.payment_hash)
                .map_err(|e| format!("invalid payment hash: {e}"))?,
            amount_sats: u64::try_from(row.amount_sats)
                .map_err(|e| format!("invalid amount: {e}"))?,
            fee_sats: u64::try_from(row.fee_sats).map_err(|e| format!("invalid fee: {e}"))?,
            user_transfer_id: TransferId::from_str(&row.user_transfer_id)
                .map_err(|e| format!("invalid transfer id: {e}"))?,
            htlc_address: row.htlc_address,
            idempotency_key: row.idempotency_key,
            ln_payment_id: row.ln_payment_id.map(LightningPaymentId),
            preimage: row
                .preimage
                .map(Preimage::try_from)
                .transpose()
                .map_err(|e| format!("invalid preimage: {e}"))?,
            payment_status,
            leaves_claimed: row.leaves_claimed,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ReceiveRow {
    id: String,
    user_identity_public_key: Vec<u8>,
    requester_identity_public_key: Vec<u8>,
    payment_hash: Vec<u8>,
    amount_sats: i64,
    encoded_invoice: String,
    transfer_id: Option<String>,
    transfer_amount_sats: Option<i64>,
    reservation_id: Option<String>,
    reserved_leaf_ids: Option<Vec<String>>,
    preimage: Option<Vec<u8>>,
    invoice_status: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    memo: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<ReceiveRow> for LightningReceiveRecord {
    type Error = String;

    fn try_from(row: ReceiveRow) -> Result<Self, String> {
        let invoice_status = match row.invoice_status.as_str() {
            INVOICE_PENDING => HoldInvoiceStatus::Pending,
            INVOICE_SETTLED => HoldInvoiceStatus::Settled,
            INVOICE_CANCELLED => HoldInvoiceStatus::Cancelled,
            INVOICE_FAILED => HoldInvoiceStatus::Failed,
            other => return Err(format!("invalid invoice_status: {other}")),
        };
        Ok(LightningReceiveRecord {
            id: row.id,
            user_identity_public_key: PublicKey::from_slice(&row.user_identity_public_key)
                .map_err(|e| format!("invalid user public key: {e}"))?,
            requester_identity_public_key: PublicKey::from_slice(
                &row.requester_identity_public_key,
            )
            .map_err(|e| format!("invalid requester public key: {e}"))?,
            payment_hash: sha256::Hash::from_slice(&row.payment_hash)
                .map_err(|e| format!("invalid payment hash: {e}"))?,
            amount_sats: u64::try_from(row.amount_sats)
                .map_err(|e| format!("invalid amount: {e}"))?,
            encoded_invoice: row.encoded_invoice,
            transfer_id: row
                .transfer_id
                .map(|id| TransferId::from_str(&id))
                .transpose()
                .map_err(|e| format!("invalid transfer id: {e}"))?,
            transfer_amount_sats: row
                .transfer_amount_sats
                .map(u64::try_from)
                .transpose()
                .map_err(|e| format!("invalid transfer amount: {e}"))?,
            reservation: row
                .reservation_id
                .map(|id| {
                    let leaf_ids = row
                        .reserved_leaf_ids
                        .unwrap_or_default()
                        .iter()
                        .map(|leaf_id| leaf_id.parse())
                        .collect::<Result<_, _>>()
                        .map_err(|e| format!("invalid reserved leaf id: {e}"))?;
                    Ok::<_, String>(HandoverReservation { id, leaf_ids })
                })
                .transpose()?,
            preimage: row
                .preimage
                .map(Preimage::try_from)
                .transpose()
                .map_err(|e| format!("invalid preimage: {e}"))?,
            invoice_status,
            expires_at: row.expires_at,
            memo: row.memo,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn sats_to_i64(sats: u64) -> Result<i64, String> {
    i64::try_from(sats).map_err(|e| format!("amount exceeds i64: {e}"))
}

pub struct PostgresLightningStore {
    pool: Arc<PgPool>,
}

impl PostgresLightningStore {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LightningStore for PostgresLightningStore {
    /// Watches the send's HTLC address with the same commit.
    async fn insert_send(&self, record: &LightningSendRecord) -> Result<(), String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        sqlx::query(
            r#"INSERT INTO brz_ssp_lightning_send_requests
               (id, user_identity_public_key, encoded_invoice, payment_hash, amount_sats,
                fee_sats, user_transfer_id, htlc_address, idempotency_key)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        )
        .bind(&record.id)
        .bind(record.user_identity_public_key.serialize().to_vec())
        .bind(&record.encoded_invoice)
        .bind(record.payment_hash.as_byte_array().to_vec())
        .bind(sats_to_i64(record.amount_sats)?)
        .bind(sats_to_i64(record.fee_sats)?)
        .bind(record.user_transfer_id.to_string())
        .bind(&record.htlc_address)
        .bind(record.idempotency_key.as_deref())
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        sqlx::query(
            r#"INSERT INTO brz_ssp_watch_addresses (address) VALUES ($1) ON CONFLICT DO NOTHING"#,
        )
        .bind(&record.htlc_address)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())
    }

    async fn unswept_htlc_outputs(&self) -> Result<Vec<HtlcOutput>, String> {
        let rows: Vec<(String, Vec<u8>, Vec<u8>, Option<String>, String, i64, i64)> =
            sqlx::query_as(
                r#"SELECT s.id, s.user_identity_public_key, s.preimage, s.htlc_sweep_address,
                          o.tx_id, o.output_index, o.amount
                   FROM brz_ssp_tx_outputs o
                   INNER JOIN brz_ssp_lightning_send_requests s ON s.htlc_address = o.address
                   INNER JOIN brz_ssp_tx_blocks tb ON tb.tx_id = o.tx_id
                   WHERE s.preimage IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1
                       FROM brz_ssp_tx_inputs i
                       INNER JOIN brz_ssp_tx_blocks itb ON itb.tx_id = i.spending_tx_id
                       WHERE i.tx_id = o.tx_id AND i.output_index = o.output_index)"#,
            )
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        rows.into_iter()
            .map(
                |(send_id, user, preimage, sweep_address, tx_id, output_index, amount)| {
                    Ok(HtlcOutput {
                        send_id,
                        user_identity_public_key: PublicKey::from_slice(&user)
                            .map_err(|e| format!("invalid user public key: {e}"))?,
                        preimage: Preimage::try_from(preimage)
                            .map_err(|e| format!("invalid preimage: {e}"))?,
                        sweep_address,
                        outpoint: bitcoin::OutPoint {
                            txid: bitcoin::Txid::from_str(&tx_id)
                                .map_err(|e| format!("invalid txid: {e}"))?,
                            vout: u32::try_from(output_index)
                                .map_err(|e| format!("invalid output index: {e}"))?,
                        },
                        value_sats: u64::try_from(amount)
                            .map_err(|e| format!("invalid amount: {e}"))?,
                    })
                },
            )
            .collect()
    }

    async fn set_htlc_sweep_address(&self, send_id: &str, address: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_send_requests
               SET htlc_sweep_address = $2, updated_at = NOW()
               WHERE id = $1 AND htlc_sweep_address IS NULL"#,
        )
        .bind(send_id)
        .bind(address)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_send_payment_id(
        &self,
        id: &str,
        ln_payment_id: &LightningPaymentId,
    ) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_send_requests SET ln_payment_id = $2, updated_at = NOW() WHERE id = $1"#,
        )
            .bind(id)
            .bind(&ln_payment_id.0)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_send_succeeded(&self, id: &str, preimage: &Preimage) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_send_requests
               SET preimage = $2, payment_status = $3, updated_at = NOW() WHERE id = $1"#,
        )
        .bind(id)
        .bind(preimage.to_vec())
        .bind(SEND_SUCCEEDED)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_send_failed(&self, id: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_send_requests SET payment_status = $2, updated_at = NOW() WHERE id = $1"#,
        )
        .bind(id)
        .bind(SEND_FAILED)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_send_leaves_claimed(&self, id: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_send_requests SET leaves_claimed = TRUE, updated_at = NOW() WHERE id = $1"#,
        )
            .bind(id)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get_send(&self, id: &str) -> Result<Option<LightningSendRecord>, String> {
        sqlx::query_as::<_, SendRow>(
            r#"SELECT * FROM brz_ssp_lightning_send_requests WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(LightningSendRecord::try_from)
        .transpose()
    }

    async fn get_send_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<LightningSendRecord>, String> {
        sqlx::query_as::<_, SendRow>(
            r#"SELECT * FROM brz_ssp_lightning_send_requests WHERE idempotency_key = $1"#,
        )
        .bind(idempotency_key)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(LightningSendRecord::try_from)
        .transpose()
    }

    async fn get_send_by_transfer_id(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<LightningSendRecord>, String> {
        sqlx::query_as::<_, SendRow>(
            r#"SELECT * FROM brz_ssp_lightning_send_requests WHERE user_transfer_id = $1"#,
        )
        .bind(transfer_id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(LightningSendRecord::try_from)
        .transpose()
    }

    async fn get_unfailed_send_by_payment_hash(
        &self,
        payment_hash: &sha256::Hash,
    ) -> Result<Option<LightningSendRecord>, String> {
        sqlx::query_as::<_, SendRow>(
            r#"SELECT * FROM brz_ssp_lightning_send_requests
               WHERE payment_hash = $1 AND payment_status <> $2"#,
        )
        .bind(payment_hash.as_byte_array().to_vec())
        .bind(SEND_FAILED)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(LightningSendRecord::try_from)
        .transpose()
    }

    async fn pending_sends(&self) -> Result<Vec<LightningSendRecord>, String> {
        sqlx::query_as::<_, SendRow>(
            r#"SELECT * FROM brz_ssp_lightning_send_requests
               WHERE payment_status = 'pending'
                  OR (payment_status = 'succeeded' AND NOT leaves_claimed)
               ORDER BY created_at"#,
        )
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(LightningSendRecord::try_from)
        .collect()
    }

    async fn insert_receive(&self, record: &LightningReceiveRecord) -> Result<(), String> {
        sqlx::query(
            r#"INSERT INTO brz_ssp_lightning_receive_requests
               (id, user_identity_public_key, requester_identity_public_key, payment_hash,
                amount_sats, encoded_invoice, expires_at, memo)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(&record.id)
        .bind(record.user_identity_public_key.serialize().to_vec())
        .bind(record.requester_identity_public_key.serialize().to_vec())
        .bind(record.payment_hash.as_byte_array().to_vec())
        .bind(sats_to_i64(record.amount_sats)?)
        .bind(&record.encoded_invoice)
        .bind(record.expires_at)
        .bind(record.memo.as_deref())
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_receive_handover(
        &self,
        id: &str,
        transfer_id: &TransferId,
        amount_sats: u64,
        reservation: &HandoverReservation,
    ) -> Result<(), String> {
        let leaf_ids: Vec<String> = reservation
            .leaf_ids
            .iter()
            .map(ToString::to_string)
            .collect();
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_receive_requests
               SET transfer_id = $2, transfer_amount_sats = $3, reservation_id = $4,
                   reserved_leaf_ids = $5, updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(transfer_id.to_string())
        .bind(sats_to_i64(amount_sats)?)
        .bind(&reservation.id)
        .bind(leaf_ids)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn clear_receive_reservation(&self, id: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_receive_requests
               SET reservation_id = NULL, reserved_leaf_ids = NULL, updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn clear_receive_handover(&self, id: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_receive_requests
               SET transfer_id = NULL, transfer_amount_sats = NULL, reservation_id = NULL,
                   reserved_leaf_ids = NULL, updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_receive_preimage(&self, id: &str, preimage: &Preimage) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_receive_requests SET preimage = $2, updated_at = NOW() WHERE id = $1"#,
        )
            .bind(id)
            .bind(preimage.to_vec())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_receive_invoice_status(
        &self,
        id: &str,
        status: HoldInvoiceStatus,
    ) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_receive_requests SET invoice_status = $2, updated_at = NOW() WHERE id = $1"#,
        )
            .bind(id)
            .bind(invoice_status_str(status))
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get_receive(&self, id: &str) -> Result<Option<LightningReceiveRecord>, String> {
        sqlx::query_as::<_, ReceiveRow>(
            r#"SELECT * FROM brz_ssp_lightning_receive_requests WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(LightningReceiveRecord::try_from)
        .transpose()
    }

    async fn get_receive_by_payment_hash(
        &self,
        payment_hash: &sha256::Hash,
    ) -> Result<Option<LightningReceiveRecord>, String> {
        sqlx::query_as::<_, ReceiveRow>(
            r#"SELECT * FROM brz_ssp_lightning_receive_requests WHERE payment_hash = $1"#,
        )
        .bind(payment_hash.as_byte_array().to_vec())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(LightningReceiveRecord::try_from)
        .transpose()
    }

    async fn get_receive_by_transfer_id(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<LightningReceiveRecord>, String> {
        sqlx::query_as::<_, ReceiveRow>(
            r#"SELECT * FROM brz_ssp_lightning_receive_requests WHERE transfer_id = $1"#,
        )
        .bind(transfer_id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(LightningReceiveRecord::try_from)
        .transpose()
    }

    async fn pending_receives(&self) -> Result<Vec<LightningReceiveRecord>, String> {
        sqlx::query_as::<_, ReceiveRow>(
            r#"SELECT * FROM brz_ssp_lightning_receive_requests
               WHERE invoice_status = 'pending' ORDER BY created_at"#,
        )
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(LightningReceiveRecord::try_from)
        .collect()
    }

    async fn unhanded_receives(
        &self,
        payment_hashes: &[sha256::Hash],
    ) -> Result<Vec<LightningReceiveRecord>, String> {
        let hashes: Vec<Vec<u8>> = payment_hashes
            .iter()
            .map(|hash| hash.as_byte_array().to_vec())
            .collect();
        receives(
            sqlx::query_as::<_, ReceiveRow>(
                r#"SELECT * FROM brz_ssp_lightning_receive_requests
                   WHERE payment_hash = ANY($1)
                     AND invoice_status = 'pending' AND transfer_id IS NULL"#,
            )
            .bind(hashes)
            .fetch_all(self.pool.as_ref())
            .await,
        )
    }

    async fn handing_over_receives(&self) -> Result<Vec<LightningReceiveRecord>, String> {
        receives(
            sqlx::query_as::<_, ReceiveRow>(
                r#"SELECT * FROM brz_ssp_lightning_receive_requests
                   WHERE invoice_status = 'pending' AND transfer_id IS NOT NULL
                   ORDER BY created_at"#,
            )
            .fetch_all(self.pool.as_ref())
            .await,
        )
    }

    async fn unhanded_receives_expired_before(
        &self,
        time: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<LightningReceiveRecord>, String> {
        receives(
            sqlx::query_as::<_, ReceiveRow>(
                r#"SELECT * FROM brz_ssp_lightning_receive_requests
                   WHERE invoice_status = 'pending' AND transfer_id IS NULL AND expires_at < $1
                   ORDER BY expires_at
                   LIMIT $2"#,
            )
            .bind(time)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await,
        )
    }
}

fn receives(
    rows: Result<Vec<ReceiveRow>, sqlx::Error>,
) -> Result<Vec<LightningReceiveRecord>, String> {
    rows.map_err(|e| e.to_string())?
        .into_iter()
        .map(LightningReceiveRecord::try_from)
        .collect()
}
