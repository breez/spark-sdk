#![allow(clippy::needless_raw_string_hashes)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use spark::services::{Preimage, TransferId};
use sqlx::PgPool;
use uuid::Uuid;

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
    user_transfer_id: Uuid,
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
            user_transfer_id: TransferId::from_str(&row.user_transfer_id.to_string())
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
    transfer_id: Option<Uuid>,
    transfer_amount_sats: Option<i64>,
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
                .map(|id| TransferId::from_str(&id.to_string()))
                .transpose()
                .map_err(|e| format!("invalid transfer id: {e}"))?,
            transfer_amount_sats: row
                .transfer_amount_sats
                .map(u64::try_from)
                .transpose()
                .map_err(|e| format!("invalid transfer amount: {e}"))?,
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
               VALUES ($1, $2, $3, $4, $5, $6, $7::uuid, $8, $9)"#,
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
            r#"INSERT INTO brz_ssp_chain_watch_addresses (address) VALUES ($1) ON CONFLICT DO NOTHING"#,
        )
        .bind(&record.htlc_address)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())
    }

    async fn unswept_htlc_outputs(&self) -> Result<Vec<HtlcOutput>, String> {
        let rows: Vec<(String, Vec<u8>, Vec<u8>, Option<String>, String, i32, i64)> =
            sqlx::query_as(
                r#"SELECT s.id, s.user_identity_public_key, s.preimage, s.htlc_sweep_address,
                          o.tx_id, o.output_index, o.amount
                   FROM brz_ssp_chain_tx_outputs o
                   INNER JOIN brz_ssp_lightning_send_requests s ON s.htlc_address = o.address
                   INNER JOIN brz_ssp_chain_tx_blocks tb ON tb.tx_id = o.tx_id
                   WHERE s.preimage IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1
                       FROM brz_ssp_chain_tx_inputs i
                       INNER JOIN brz_ssp_chain_tx_blocks itb ON itb.tx_id = i.spending_tx_id
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
                            vout: output_index.cast_unsigned(),
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
            r#"SELECT * FROM brz_ssp_lightning_send_requests WHERE user_transfer_id = $1::uuid"#,
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
    ) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_receive_requests
               SET transfer_id = $2::uuid, transfer_amount_sats = $3, updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(transfer_id.to_string())
        .bind(sats_to_i64(amount_sats)?)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn clear_receive_handover(&self, id: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_lightning_receive_requests
               SET transfer_id = NULL, transfer_amount_sats = NULL, updated_at = NOW()
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
            r#"SELECT * FROM brz_ssp_lightning_receive_requests WHERE transfer_id = $1::uuid"#,
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

#[cfg(test)]
mod tests {
    use bitcoin::address::NetworkUnchecked;
    use bitcoin::{Address, Amount, BlockHash, Network, OutPoint, TxOut, Txid};

    use super::*;
    use crate::chain::{AddressUtxo, BlockHeader, ChainRepository as _, SpentTxo, Txo};
    use crate::lightning::repository::scenarios::{self, new_send, preimage};
    use crate::postgresql::ChainRepository;
    use crate::postgresql::testing::TestDatabase;

    struct Fixture {
        store: PostgresLightningStore,
        chain: ChainRepository,
        pool: Arc<PgPool>,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            let database = TestDatabase::new().await;
            let pool = Arc::clone(&database.pool);
            Self {
                store: PostgresLightningStore::new(Arc::clone(&pool)),
                chain: ChainRepository::new(Arc::clone(&pool), Network::Regtest),
                pool,
                _database: database,
            }
        }

        async fn paid_send(&self, number: u8) -> LightningSendRecord {
            let send = new_send(number);
            self.store.insert_send(&send).await.unwrap();
            self.store
                .set_send_succeeded(&send.id, &preimage(number))
                .await
                .unwrap();
            send
        }

        async fn confirm_outputs(
            &self,
            height: u8,
            outputs: &[(&LightningSendRecord, OutPoint, u64)],
        ) -> BlockHash {
            let block = block(height);
            let outputs: Vec<AddressUtxo> = outputs
                .iter()
                .map(|(send, outpoint, sats)| {
                    let address: Address<NetworkUnchecked> = send.htlc_address.parse().unwrap();
                    let address = address.assume_checked();
                    AddressUtxo {
                        utxo: Txo {
                            block_height: block.height,
                            outpoint: *outpoint,
                            tx_out: TxOut {
                                value: Amount::from_sat(*sats),
                                script_pubkey: address.script_pubkey(),
                            },
                        },
                        address,
                    }
                })
                .collect();
            self.chain.add_block(&block, &outputs, &[]).await.unwrap();
            block.hash
        }

        async fn confirm_spend(&self, height: u8, spent: OutPoint, spender: Txid) -> BlockHash {
            let block = block(height);
            let input = SpentTxo {
                outpoint: spent,
                spending_tx: spender,
                spending_input_index: 0,
            };
            self.chain.add_block(&block, &[], &[input]).await.unwrap();
            block.hash
        }

        /// The function sorts the outpoints because the store lists them in no
        /// set order.
        async fn unswept(&self) -> Vec<OutPoint> {
            let outputs = self.store.unswept_htlc_outputs().await.unwrap();
            let mut outpoints: Vec<OutPoint> =
                outputs.iter().map(|output| output.outpoint).collect();
            outpoints.sort();
            outpoints
        }

        async fn watched_addresses(&self) -> Vec<String> {
            sqlx::query_scalar("SELECT address FROM brz_ssp_chain_watch_addresses ORDER BY address")
                .fetch_all(&*self.pool)
                .await
                .unwrap()
        }
    }

    fn block(height: u8) -> BlockHeader {
        BlockHeader {
            hash: BlockHash::from_byte_array([height; 32]),
            height: u64::from(height),
        }
    }

    fn txid(number: u8) -> Txid {
        Txid::from_byte_array([number; 32])
    }

    #[tokio::test]
    async fn a_send_reads_back_as_it_was_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_send_reads_back_as_it_was_stored(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_new_send_is_pending_without_a_payment() {
        let fixture = Fixture::new().await;
        scenarios::a_new_send_is_pending_without_a_payment(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_store_sets_the_state_and_times_of_a_new_send() {
        let fixture = Fixture::new().await;
        scenarios::the_store_sets_the_state_and_times_of_a_new_send(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_send_is_found_by_its_idempotency_key() {
        let fixture = Fixture::new().await;
        scenarios::a_send_is_found_by_its_idempotency_key(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_send_is_found_by_the_transfer_that_paid_for_it() {
        let fixture = Fixture::new().await;
        scenarios::a_send_is_found_by_the_transfer_that_paid_for_it(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_send_is_found_by_its_payment_hash_unless_it_failed() {
        let fixture = Fixture::new().await;
        scenarios::a_send_is_found_by_its_payment_hash_unless_it_failed(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_send_reads_back_the_payment_made_for_it() {
        let fixture = Fixture::new().await;
        scenarios::a_send_reads_back_the_payment_made_for_it(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_paid_send_reads_back_its_preimage() {
        let fixture = Fixture::new().await;
        scenarios::a_paid_send_reads_back_its_preimage(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_paid_send_is_complete_once_its_leaves_are_claimed() {
        let fixture = Fixture::new().await;
        scenarios::a_paid_send_is_complete_once_its_leaves_are_claimed(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_failed_send_is_complete() {
        let fixture = Fixture::new().await;
        scenarios::a_failed_send_is_complete(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_pending_sends_are_those_that_are_not_complete() {
        let fixture = Fixture::new().await;
        scenarios::the_pending_sends_are_those_that_are_not_complete(&fixture.store).await;
    }

    #[tokio::test]
    async fn pending_sends_are_listed_in_order_of_creation() {
        let fixture = Fixture::new().await;
        scenarios::pending_sends_are_listed_in_order_of_creation(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_second_send_with_a_stored_id_is_refused() {
        let fixture = Fixture::new().await;
        scenarios::a_second_send_with_a_stored_id_is_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_second_send_for_a_transfer_is_refused() {
        let fixture = Fixture::new().await;
        scenarios::a_second_send_for_a_transfer_is_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_second_send_with_a_stored_idempotency_key_is_refused() {
        let fixture = Fixture::new().await;
        scenarios::a_second_send_with_a_stored_idempotency_key_is_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn sends_without_an_idempotency_key_do_not_collide() {
        let fixture = Fixture::new().await;
        scenarios::sends_without_an_idempotency_key_do_not_collide(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_second_send_of_an_invoice_is_refused_while_the_first_has_not_failed() {
        let fixture = Fixture::new().await;
        scenarios::a_second_send_of_an_invoice_is_refused_while_the_first_has_not_failed(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn an_invoice_can_be_sent_again_after_its_send_failed() {
        let fixture = Fixture::new().await;
        scenarios::an_invoice_can_be_sent_again_after_its_send_failed(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_write_to_one_send_leaves_another_as_it_was() {
        let fixture = Fixture::new().await;
        scenarios::a_write_to_one_send_leaves_another_as_it_was(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_write_to_a_send_the_store_does_not_have_stores_nothing() {
        let fixture = Fixture::new().await;
        scenarios::a_write_to_a_send_the_store_does_not_have_stores_nothing(&fixture.store).await;
    }

    #[tokio::test]
    async fn each_write_to_a_send_moves_its_update_time_forward() {
        let fixture = Fixture::new().await;
        scenarios::each_write_to_a_send_moves_its_update_time_forward(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_paid_send_with_nothing_on_chain_has_no_htlc_output_to_sweep() {
        let fixture = Fixture::new().await;
        scenarios::a_paid_send_with_nothing_on_chain_has_no_htlc_output_to_sweep(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn a_send_and_a_receive_can_have_the_same_payment_hash() {
        let fixture = Fixture::new().await;
        scenarios::a_send_and_a_receive_can_have_the_same_payment_hash(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_receive_reads_back_as_it_was_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_receive_reads_back_as_it_was_stored(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_receive_without_a_memo_reads_back_without_one() {
        let fixture = Fixture::new().await;
        scenarios::a_receive_without_a_memo_reads_back_without_one(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_new_receive_awaits_payment() {
        let fixture = Fixture::new().await;
        scenarios::a_new_receive_awaits_payment(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_store_sets_the_state_and_times_of_a_new_receive() {
        let fixture = Fixture::new().await;
        scenarios::the_store_sets_the_state_and_times_of_a_new_receive(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_receive_is_found_by_its_payment_hash() {
        let fixture = Fixture::new().await;
        scenarios::a_receive_is_found_by_its_payment_hash(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_receive_is_found_by_its_payment_hash_in_every_state() {
        let fixture = Fixture::new().await;
        scenarios::a_receive_is_found_by_its_payment_hash_in_every_state(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_second_receive_with_a_stored_id_is_refused() {
        let fixture = Fixture::new().await;
        scenarios::a_second_receive_with_a_stored_id_is_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_second_receive_for_a_payment_hash_is_always_refused() {
        let fixture = Fixture::new().await;
        scenarios::a_second_receive_for_a_payment_hash_is_always_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_receive_reads_back_its_handover() {
        let fixture = Fixture::new().await;
        scenarios::a_receive_reads_back_its_handover(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_cleared_handover_leaves_the_receive_awaiting_payment() {
        let fixture = Fixture::new().await;
        scenarios::a_cleared_handover_leaves_the_receive_awaiting_payment(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_receive_with_a_preimage_is_ready_to_settle() {
        let fixture = Fixture::new().await;
        scenarios::a_receive_with_a_preimage_is_ready_to_settle(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_receive_reads_back_how_it_ended() {
        let fixture = Fixture::new().await;
        scenarios::a_receive_reads_back_how_it_ended(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_settled_receive_keeps_its_handover_and_preimage() {
        let fixture = Fixture::new().await;
        scenarios::a_settled_receive_keeps_its_handover_and_preimage(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_pending_receives_are_those_whose_invoice_is_pending() {
        let fixture = Fixture::new().await;
        scenarios::the_pending_receives_are_those_whose_invoice_is_pending(&fixture.store).await;
    }

    #[tokio::test]
    async fn pending_receives_are_listed_in_order_of_creation() {
        let fixture = Fixture::new().await;
        scenarios::pending_receives_are_listed_in_order_of_creation(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_receives_handing_over_are_the_pending_ones_with_a_handover() {
        let fixture = Fixture::new().await;
        scenarios::the_receives_handing_over_are_the_pending_ones_with_a_handover(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn receives_handing_over_are_listed_in_order_of_creation() {
        let fixture = Fixture::new().await;
        scenarios::receives_handing_over_are_listed_in_order_of_creation(&fixture.store).await;
    }

    #[tokio::test]
    async fn no_payment_hashes_list_no_unhanded_receives() {
        let fixture = Fixture::new().await;
        scenarios::no_payment_hashes_list_no_unhanded_receives(&fixture.store).await;
    }

    #[tokio::test]
    async fn one_payment_hash_lists_its_unhanded_receive() {
        let fixture = Fixture::new().await;
        scenarios::one_payment_hash_lists_its_unhanded_receive(&fixture.store).await;
    }

    #[tokio::test]
    async fn several_payment_hashes_list_each_of_their_unhanded_receives() {
        let fixture = Fixture::new().await;
        scenarios::several_payment_hashes_list_each_of_their_unhanded_receives(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn a_payment_hash_without_a_receive_lists_nothing() {
        let fixture = Fixture::new().await;
        scenarios::a_payment_hash_without_a_receive_lists_nothing(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_repeated_payment_hash_lists_its_receive_once() {
        let fixture = Fixture::new().await;
        scenarios::a_repeated_payment_hash_lists_its_receive_once(&fixture.store).await;
    }

    #[tokio::test]
    async fn only_pending_receives_without_a_handover_are_listed_as_unhanded() {
        let fixture = Fixture::new().await;
        scenarios::only_pending_receives_without_a_handover_are_listed_as_unhanded(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn only_a_receive_that_expired_before_the_time_is_listed() {
        let fixture = Fixture::new().await;
        scenarios::only_a_receive_that_expired_before_the_time_is_listed(&fixture.store).await;
    }

    #[tokio::test]
    async fn only_pending_receives_without_a_handover_are_listed_as_expired() {
        let fixture = Fixture::new().await;
        scenarios::only_pending_receives_without_a_handover_are_listed_as_expired(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn no_more_expired_receives_are_listed_than_the_limit() {
        let fixture = Fixture::new().await;
        scenarios::no_more_expired_receives_are_listed_than_the_limit(&fixture.store).await;
    }

    #[tokio::test]
    async fn expired_receives_are_listed_in_order_of_expiry() {
        let fixture = Fixture::new().await;
        scenarios::expired_receives_are_listed_in_order_of_expiry(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_limit_keeps_the_receives_that_expired_first() {
        let fixture = Fixture::new().await;
        scenarios::a_limit_keeps_the_receives_that_expired_first(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_write_to_one_receive_leaves_another_as_it_was() {
        let fixture = Fixture::new().await;
        scenarios::a_write_to_one_receive_leaves_another_as_it_was(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_write_to_a_receive_the_store_does_not_have_stores_nothing() {
        let fixture = Fixture::new().await;
        scenarios::a_write_to_a_receive_the_store_does_not_have_stores_nothing(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn each_write_to_a_receive_moves_its_update_time_forward() {
        let fixture = Fixture::new().await;
        scenarios::each_write_to_a_receive_moves_its_update_time_forward(&fixture.store).await;
    }

    #[tokio::test]
    async fn an_amount_too_large_for_the_database_is_refused() {
        let fixture = Fixture::new().await;
        scenarios::an_amount_too_large_for_the_database_is_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_htlc_address_of_a_send_is_watched() {
        let fixture = Fixture::new().await;
        let send = new_send(1);

        fixture.store.insert_send(&send).await.unwrap();

        assert_eq!(fixture.watched_addresses().await, [send.htlc_address]);
    }

    #[tokio::test]
    async fn an_htlc_address_is_watched_once_for_two_sends() {
        let fixture = Fixture::new().await;
        let first = new_send(1);
        fixture.store.insert_send(&first).await.unwrap();
        fixture.store.set_send_failed(&first.id).await.unwrap();
        let second = LightningSendRecord {
            payment_hash: first.payment_hash,
            htlc_address: first.htlc_address.clone(),
            ..new_send(2)
        };

        fixture.store.insert_send(&second).await.unwrap();

        assert!(fixture.store.get_send(&second.id).await.unwrap().is_some());
        assert_eq!(fixture.watched_addresses().await, [first.htlc_address]);
    }

    #[tokio::test]
    async fn the_htlc_address_of_a_refused_send_is_not_watched() {
        let fixture = Fixture::new().await;
        let first = new_send(1);
        fixture.store.insert_send(&first).await.unwrap();
        let refused = LightningSendRecord {
            user_transfer_id: first.user_transfer_id.clone(),
            ..new_send(2)
        };

        assert!(fixture.store.insert_send(&refused).await.is_err());

        assert_eq!(fixture.watched_addresses().await, [first.htlc_address]);
    }

    #[tokio::test]
    async fn the_confirmed_htlc_output_of_a_paid_send_is_listed() {
        let fixture = Fixture::new().await;
        let send = fixture.paid_send(1).await;
        let output = OutPoint::new(txid(1), 3);
        fixture.confirm_outputs(1, &[(&send, output, 900)]).await;

        let listed = fixture.store.unswept_htlc_outputs().await.unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].send_id, send.id);
        assert_eq!(
            listed[0].user_identity_public_key,
            send.user_identity_public_key
        );
        assert_eq!(listed[0].preimage, preimage(1));
        assert_eq!(listed[0].sweep_address, None);
        assert_eq!(listed[0].outpoint, output);
        assert_eq!(listed[0].value_sats, 900);
    }

    #[tokio::test]
    async fn an_htlc_output_is_listed_only_once_its_send_is_paid() {
        let fixture = Fixture::new().await;
        let send = new_send(1);
        fixture.store.insert_send(&send).await.unwrap();
        let output = OutPoint::new(txid(1), 0);
        fixture.confirm_outputs(1, &[(&send, output, 900)]).await;
        assert!(fixture.unswept().await.is_empty());

        fixture
            .store
            .set_send_succeeded(&send.id, &preimage(1))
            .await
            .unwrap();

        assert_eq!(fixture.unswept().await, [output]);
    }

    #[tokio::test]
    async fn an_htlc_output_is_not_listed_once_its_block_is_undone() {
        let fixture = Fixture::new().await;
        let send = fixture.paid_send(1).await;
        let output = OutPoint::new(txid(1), 0);
        let block = fixture.confirm_outputs(1, &[(&send, output, 900)]).await;
        assert_eq!(fixture.unswept().await, [output]);

        fixture.chain.undo_block(block).await.unwrap();

        assert!(fixture.unswept().await.is_empty());
    }

    #[tokio::test]
    async fn an_htlc_output_is_not_listed_once_a_confirmed_transaction_spent_it() {
        let fixture = Fixture::new().await;
        let send = fixture.paid_send(1).await;
        let (spent, unspent) = (OutPoint::new(txid(1), 0), OutPoint::new(txid(1), 1));
        fixture
            .confirm_outputs(1, &[(&send, spent, 900), (&send, unspent, 800)])
            .await;

        fixture.confirm_spend(2, spent, txid(2)).await;

        assert_eq!(fixture.unswept().await, [unspent]);
    }

    #[tokio::test]
    async fn an_htlc_output_is_listed_again_once_the_block_of_its_spend_is_undone() {
        let fixture = Fixture::new().await;
        let send = fixture.paid_send(1).await;
        let output = OutPoint::new(txid(1), 0);
        fixture.confirm_outputs(1, &[(&send, output, 900)]).await;
        let block = fixture.confirm_spend(2, output, txid(2)).await;
        assert!(fixture.unswept().await.is_empty());

        fixture.chain.undo_block(block).await.unwrap();

        assert_eq!(fixture.unswept().await, [output]);
    }

    #[tokio::test]
    async fn every_htlc_output_of_a_paid_send_is_listed() {
        let fixture = Fixture::new().await;
        let send = fixture.paid_send(1).await;
        let first = OutPoint::new(txid(1), 0);
        let second = OutPoint::new(txid(1), 1);
        let third = OutPoint::new(txid(2), 0);
        fixture
            .confirm_outputs(1, &[(&send, first, 900), (&send, second, 800)])
            .await;
        fixture.confirm_outputs(2, &[(&send, third, 700)]).await;

        let listed = fixture.store.unswept_htlc_outputs().await.unwrap();

        let mut listed: Vec<(OutPoint, u64)> = listed
            .iter()
            .map(|output| (output.outpoint, output.value_sats))
            .collect();
        listed.sort_unstable();
        assert_eq!(listed, [(first, 900), (second, 800), (third, 700)]);
    }

    #[tokio::test]
    async fn an_htlc_output_is_listed_with_its_own_send() {
        let fixture = Fixture::new().await;
        let (first, second) = (fixture.paid_send(1).await, fixture.paid_send(2).await);
        let outputs = [OutPoint::new(txid(1), 0), OutPoint::new(txid(2), 0)];
        fixture
            .confirm_outputs(1, &[(&first, outputs[0], 900), (&second, outputs[1], 800)])
            .await;

        let mut listed = fixture.store.unswept_htlc_outputs().await.unwrap();

        listed.sort_by_key(|output| output.outpoint);
        assert_eq!(listed.len(), 2);
        for (output, (send, number)) in listed.iter().zip([(&first, 1), (&second, 2)]) {
            assert_eq!(output.send_id, send.id);
            assert_eq!(
                output.user_identity_public_key,
                send.user_identity_public_key
            );
            assert_eq!(output.preimage, preimage(number));
        }
    }

    #[tokio::test]
    async fn an_output_at_no_sends_htlc_address_is_not_listed() {
        let fixture = Fixture::new().await;
        fixture.paid_send(1).await;

        fixture
            .confirm_outputs(1, &[(&new_send(2), OutPoint::new(txid(1), 0), 900)])
            .await;

        assert!(fixture.unswept().await.is_empty());
    }

    #[tokio::test]
    async fn a_sweep_address_is_listed_with_every_htlc_output_of_its_send() {
        let fixture = Fixture::new().await;
        let send = fixture.paid_send(1).await;
        let outputs = [OutPoint::new(txid(1), 0), OutPoint::new(txid(1), 1)];
        fixture
            .confirm_outputs(1, &[(&send, outputs[0], 900), (&send, outputs[1], 800)])
            .await;

        fixture
            .store
            .set_htlc_sweep_address(&send.id, "bcrt1p-sweep")
            .await
            .unwrap();

        let listed = fixture.store.unswept_htlc_outputs().await.unwrap();
        assert_eq!(listed.len(), 2);
        for output in listed {
            assert_eq!(output.sweep_address.as_deref(), Some("bcrt1p-sweep"));
        }
    }

    #[tokio::test]
    async fn the_sweep_address_of_a_send_is_set_once() {
        let fixture = Fixture::new().await;
        let send = fixture.paid_send(1).await;
        fixture
            .confirm_outputs(1, &[(&send, OutPoint::new(txid(1), 0), 900)])
            .await;
        let set = async |address: &str| {
            fixture
                .store
                .set_htlc_sweep_address(&send.id, address)
                .await
                .unwrap();
        };

        set("bcrt1p-first").await;
        set("bcrt1p-second").await;

        let listed = fixture.store.unswept_htlc_outputs().await.unwrap();
        assert_eq!(listed[0].sweep_address.as_deref(), Some("bcrt1p-first"));
    }

    #[tokio::test]
    async fn a_sweep_address_is_set_for_the_send_it_names_only() {
        let fixture = Fixture::new().await;
        let (named, other) = (fixture.paid_send(1).await, fixture.paid_send(2).await);
        let outputs = [OutPoint::new(txid(1), 0), OutPoint::new(txid(2), 0)];
        fixture
            .confirm_outputs(1, &[(&named, outputs[0], 900), (&other, outputs[1], 800)])
            .await;

        fixture
            .store
            .set_htlc_sweep_address(&named.id, "bcrt1p-sweep")
            .await
            .unwrap();

        let mut listed = fixture.store.unswept_htlc_outputs().await.unwrap();
        listed.sort_by_key(|output| output.outpoint);
        assert_eq!(listed[0].sweep_address.as_deref(), Some("bcrt1p-sweep"));
        assert_eq!(listed[1].sweep_address, None);
    }
}
