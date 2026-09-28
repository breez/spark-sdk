#![allow(clippy::needless_raw_string_hashes)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;

use crate::static_deposit::repository::{
    InstantStaticDepositQuoteRecord, InstantStaticDepositQuoteStore,
};

#[derive(sqlx::FromRow)]
struct InstantQuoteRow {
    id: String,
    txid: String,
    vout: i64,
    network: String,
    deposit_amount_sats: i64,
    credit_amount_sats: i64,
    destination_address: String,
    quote_signature: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<InstantQuoteRow> for InstantStaticDepositQuoteRecord {
    type Error = String;

    fn try_from(row: InstantQuoteRow) -> Result<Self, String> {
        Ok(InstantStaticDepositQuoteRecord {
            id: row.id,
            txid: row.txid,
            vout: u32::try_from(row.vout).map_err(|e| format!("invalid vout: {e}"))?,
            network: spark::Network::from_str(&row.network)
                .map_err(|e| format!("invalid network: {e}"))?,
            deposit_amount_sats: u64::try_from(row.deposit_amount_sats)
                .map_err(|e| format!("invalid deposit amount: {e}"))?,
            credit_amount_sats: u64::try_from(row.credit_amount_sats)
                .map_err(|e| format!("invalid credit amount: {e}"))?,
            destination_address: row.destination_address,
            quote_signature: row.quote_signature,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

pub struct PostgresInstantStaticDepositQuoteStore {
    pool: Arc<PgPool>,
}

impl PostgresInstantStaticDepositQuoteStore {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

fn sats_to_i64(sats: u64) -> Result<i64, String> {
    i64::try_from(sats).map_err(|e| format!("amount exceeds i64: {e}"))
}

#[async_trait]
impl InstantStaticDepositQuoteStore for PostgresInstantStaticDepositQuoteStore {
    async fn insert(&self, record: &InstantStaticDepositQuoteRecord) -> Result<(), String> {
        sqlx::query(
            r#"INSERT INTO brz_ssp_instant_static_deposit_quotes
               (id, txid, vout, network, deposit_amount_sats, credit_amount_sats,
                destination_address, quote_signature)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(&record.id)
        .bind(&record.txid)
        .bind(i64::from(record.vout))
        .bind(record.network.to_string())
        .bind(sats_to_i64(record.deposit_amount_sats)?)
        .bind(sats_to_i64(record.credit_amount_sats)?)
        .bind(&record.destination_address)
        .bind(&record.quote_signature)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<InstantStaticDepositQuoteRecord>, String> {
        sqlx::query_as::<_, InstantQuoteRow>(
            r#"SELECT * FROM brz_ssp_instant_static_deposit_quotes WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(InstantStaticDepositQuoteRecord::try_from)
        .transpose()
    }

    async fn delete_created_before(
        &self,
        time: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), String> {
        sqlx::query(r#"DELETE FROM brz_ssp_instant_static_deposit_quotes WHERE created_at < $1"#)
            .bind(time)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
