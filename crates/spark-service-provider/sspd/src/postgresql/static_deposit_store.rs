#![allow(clippy::needless_raw_string_hashes)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::round1::SigningCommitments;
use prost::Message as _;
use sqlx::PgPool;

use spark::operator::rpc::spark as pb;
use spark::services::TransferId;
use spark::signer::FrostSigningCommitmentsWithNonces;

use crate::handover::HandoverReservation;
use crate::static_deposit::repository::{
    PendingCredit, StaticDepositClaimRecord, StaticDepositClaimStore, StaticDepositSpendContext,
    StaticDepositSpendPrep,
};

#[derive(sqlx::FromRow)]
struct StaticDepositClaimRow {
    id: String,
    user_identity_public_key: Vec<u8>,
    txid: String,
    vout: i64,
    network: String,
    deposit_address: String,
    credit_amount_sats: i64,
    is_instant: bool,
    deposit_amount_sats: i64,
    encrypted_deposit_secret_key: String,
    quote_signature: String,
    user_signature: String,
    transfer_id: Option<String>,
    pending_transfer_id: Option<String>,
    reservation_id: Option<String>,
    reserved_leaf_ids: Option<Vec<String>>,
    utxo_swap_id: Option<String>,
    prep_spend_tx: Vec<u8>,
    prep_spend_nonce_commitments: Vec<u8>,
    prep_spend_nonce_ciphertext: Vec<u8>,
    verifying_public_key: Option<Vec<u8>>,
    spend_tx_signing_result: Option<Vec<u8>>,
    spend_broadcast_txid: Option<String>,
    spend_confirmed: bool,
    deposit_lost: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<StaticDepositClaimRow> for StaticDepositClaimRecord {
    type Error = String;

    fn try_from(row: StaticDepositClaimRow) -> Result<Self, String> {
        let spend_prep = row_spend_prep(&row)?;
        let spend_context = row_spend_context(&row)?;
        let pending_credit = match (row.pending_transfer_id, row.reservation_id) {
            (Some(transfer_id), Some(id)) => Some(PendingCredit {
                transfer_id: TransferId::from_str(&transfer_id)
                    .map_err(|e| format!("invalid pending transfer id: {e}"))?,
                reservation: HandoverReservation {
                    id,
                    leaf_ids: row
                        .reserved_leaf_ids
                        .unwrap_or_default()
                        .iter()
                        .map(|leaf_id| leaf_id.parse())
                        .collect::<Result<_, _>>()
                        .map_err(|e| format!("invalid reserved leaf id: {e}"))?,
                },
            }),
            (None, None) => None,
            _ => {
                return Err(format!(
                    "static deposit claim {} has a partial pending credit",
                    row.id
                ));
            }
        };
        Ok(StaticDepositClaimRecord {
            id: row.id,
            user_identity_public_key: PublicKey::from_slice(&row.user_identity_public_key)
                .map_err(|e| format!("invalid user public key: {e}"))?,
            txid: row.txid,
            vout: u32::try_from(row.vout).map_err(|e| format!("invalid vout: {e}"))?,
            network: spark::Network::from_str(&row.network)
                .map_err(|e| format!("invalid network: {e}"))?,
            deposit_address: row.deposit_address,
            credit_amount_sats: u64::try_from(row.credit_amount_sats)
                .map_err(|e| format!("invalid credit amount: {e}"))?,
            is_instant: row.is_instant,
            deposit_amount_sats: u64::try_from(row.deposit_amount_sats)
                .map_err(|e| format!("invalid deposit amount: {e}"))?,
            encrypted_deposit_secret_key: row.encrypted_deposit_secret_key,
            quote_signature: row.quote_signature,
            user_signature: row.user_signature,
            transfer_id: row.transfer_id,
            pending_credit,
            utxo_swap_id: row.utxo_swap_id,
            spend_prep,
            spend_context,
            spend_broadcast_txid: row.spend_broadcast_txid,
            spend_confirmed: row.spend_confirmed,
            deposit_lost: row.deposit_lost,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn sats_to_i64(sats: u64) -> Result<i64, String> {
    i64::try_from(sats).map_err(|e| format!("amount exceeds i64: {e}"))
}

fn row_spend_prep(row: &StaticDepositClaimRow) -> Result<StaticDepositSpendPrep, String> {
    Ok(StaticDepositSpendPrep {
        spend_tx: bitcoin::consensus::deserialize(&row.prep_spend_tx)
            .map_err(|err| format!("invalid prep spend tx: {err}"))?,
        nonce: FrostSigningCommitmentsWithNonces {
            commitments: SigningCommitments::deserialize(&row.prep_spend_nonce_commitments)
                .map_err(|err| format!("invalid prep nonce commitments: {err}"))?,
            nonces_ciphertext: row.prep_spend_nonce_ciphertext.clone(),
        },
    })
}

fn row_spend_context(
    row: &StaticDepositClaimRow,
) -> Result<Option<StaticDepositSpendContext>, String> {
    match (
        row.verifying_public_key.as_ref(),
        row.spend_tx_signing_result.as_ref(),
    ) {
        (None, None) => Ok(None),
        (Some(verifying_public_key), Some(signing_result)) => Ok(Some(StaticDepositSpendContext {
            verifying_public_key: PublicKey::from_slice(verifying_public_key)
                .map_err(|err| format!("invalid verifying key: {err}"))?,
            signing_result: pb::SigningResult::decode(signing_result.as_slice())
                .map_err(|err| format!("invalid signing result: {err}"))?,
        })),
        _ => Err(format!(
            "static deposit claim {} has a partially-populated spend context",
            row.id
        )),
    }
}

pub struct PostgresStaticDepositClaimStore {
    pool: Arc<PgPool>,
}

impl PostgresStaticDepositClaimStore {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl StaticDepositClaimStore for PostgresStaticDepositClaimStore {
    async fn insert(&self, record: &StaticDepositClaimRecord) -> Result<(), String> {
        let credit = record.pending_credit.as_ref();
        let prep = &record.spend_prep;
        sqlx::query(
            r#"INSERT INTO brz_ssp_static_deposit_claims
               (id, user_identity_public_key, txid, vout, network, deposit_address,
                credit_amount_sats, is_instant, deposit_amount_sats,
                encrypted_deposit_secret_key, quote_signature, user_signature,
                pending_transfer_id, reservation_id, reserved_leaf_ids,
                prep_spend_tx, prep_spend_nonce_commitments, prep_spend_nonce_ciphertext)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                       $15, $16, $17, $18)"#,
        )
        .bind(&record.id)
        .bind(record.user_identity_public_key.serialize().to_vec())
        .bind(&record.txid)
        .bind(i64::from(record.vout))
        .bind(record.network.to_string())
        .bind(&record.deposit_address)
        .bind(sats_to_i64(record.credit_amount_sats)?)
        .bind(record.is_instant)
        .bind(sats_to_i64(record.deposit_amount_sats)?)
        .bind(&record.encrypted_deposit_secret_key)
        .bind(&record.quote_signature)
        .bind(&record.user_signature)
        .bind(credit.map(|credit| credit.transfer_id.to_string()))
        .bind(credit.map(|credit| credit.reservation.id.clone()))
        .bind(credit.map(|credit| {
            credit
                .reservation
                .leaf_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        }))
        .bind(bitcoin::consensus::serialize(&prep.spend_tx))
        .bind(
            prep.nonce
                .commitments
                .serialize()
                .map_err(|e| format!("failed to serialize prep nonce commitments: {e}"))?,
        )
        .bind(&prep.nonce.nonces_ciphertext)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<StaticDepositClaimRecord>, String> {
        sqlx::query_as::<_, StaticDepositClaimRow>(
            r#"SELECT * FROM brz_ssp_static_deposit_claims WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(StaticDepositClaimRecord::try_from)
        .transpose()
    }

    async fn get_by_utxo(
        &self,
        txid: &str,
        vout: u32,
    ) -> Result<Option<StaticDepositClaimRecord>, String> {
        sqlx::query_as::<_, StaticDepositClaimRow>(
            r#"SELECT * FROM brz_ssp_static_deposit_claims WHERE txid = $1 AND vout = $2"#,
        )
        .bind(txid)
        .bind(i64::from(vout))
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(StaticDepositClaimRecord::try_from)
        .transpose()
    }

    async fn get_by_transfer_id(
        &self,
        transfer_id: &str,
    ) -> Result<Option<StaticDepositClaimRecord>, String> {
        sqlx::query_as::<_, StaticDepositClaimRow>(
            r#"SELECT * FROM brz_ssp_static_deposit_claims WHERE transfer_id = $1"#,
        )
        .bind(transfer_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(StaticDepositClaimRecord::try_from)
        .transpose()
    }

    async fn set_transfer_id(&self, id: &str, transfer_id: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_static_deposit_claims
               SET transfer_id = $2,
                   pending_transfer_id = NULL, reservation_id = NULL, reserved_leaf_ids = NULL,
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(transfer_id)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), String> {
        sqlx::query(r#"DELETE FROM brz_ssp_static_deposit_claims WHERE id = $1"#)
            .bind(id)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_reserved(
        &self,
        id: &str,
        transfer_id: &str,
        utxo_swap_id: &str,
    ) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_static_deposit_claims
               SET transfer_id = $2, utxo_swap_id = $3,
                   pending_transfer_id = NULL, reservation_id = NULL, reserved_leaf_ids = NULL,
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(transfer_id)
        .bind(utxo_swap_id)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_spend_context(
        &self,
        id: &str,
        transfer_id: &str,
        context: &StaticDepositSpendContext,
    ) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_static_deposit_claims
               SET transfer_id = $2,
                   verifying_public_key = $3,
                   spend_tx_signing_result = $4,
                   pending_transfer_id = NULL, reservation_id = NULL, reserved_leaf_ids = NULL,
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(transfer_id)
        .bind(context.verifying_public_key.serialize().to_vec())
        .bind(context.signing_result.encode_to_vec())
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_spend_broadcast_txid(&self, id: &str, txid: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_static_deposit_claims SET spend_broadcast_txid = $2, updated_at = NOW() WHERE id = $1"#,
        )
        .bind(id)
        .bind(txid)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_spend_confirmed(&self, id: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_static_deposit_claims SET spend_confirmed = TRUE, updated_at = NOW() WHERE id = $1"#,
        )
        .bind(id)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_deposit_lost(&self, id: &str) -> Result<(), String> {
        sqlx::query(
            r#"UPDATE brz_ssp_static_deposit_claims SET deposit_lost = TRUE, updated_at = NOW() WHERE id = $1"#,
        )
        .bind(id)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn pending(&self) -> Result<Vec<StaticDepositClaimRecord>, String> {
        // Must match `StaticDepositClaimRecord::is_pending`.
        sqlx::query_as::<_, StaticDepositClaimRow>(
            r#"SELECT * FROM brz_ssp_static_deposit_claims
               WHERE pending_transfer_id IS NOT NULL
                  OR (transfer_id IS NOT NULL AND NOT spend_confirmed AND NOT deposit_lost)
               ORDER BY created_at"#,
        )
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(StaticDepositClaimRecord::try_from)
        .collect()
    }
}
