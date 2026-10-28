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
    vout: i32,
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
            vout: row.vout.cast_unsigned(),
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
            r#"INSERT INTO brz_ssp_static_deposit_instant_quotes
               (id, txid, vout, network, deposit_amount_sats, credit_amount_sats,
                destination_address, quote_signature)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(&record.id)
        .bind(&record.txid)
        .bind(record.vout.cast_signed())
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
            r#"SELECT * FROM brz_ssp_static_deposit_instant_quotes WHERE id = $1"#,
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
        sqlx::query(r#"DELETE FROM brz_ssp_static_deposit_instant_quotes WHERE created_at < $1"#)
            .bind(time)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::PostgresInstantStaticDepositQuoteStore;
    use crate::postgresql::testing::TestDatabase;
    use crate::static_deposit::repository::scenarios::quotes as scenarios;

    struct Fixture {
        store: PostgresInstantStaticDepositQuoteStore,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            let database = TestDatabase::new().await;
            Self {
                store: PostgresInstantStaticDepositQuoteStore::new(Arc::clone(&database.pool)),
                _database: database,
            }
        }
    }

    #[tokio::test]
    async fn a_quote_reads_back_as_it_was_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_quote_reads_back_as_it_was_stored(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_quote_the_store_was_never_given_is_not_found() {
        let fixture = Fixture::new().await;
        scenarios::a_quote_the_store_was_never_given_is_not_found(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_quote_with_the_id_of_a_stored_one_is_refused() {
        let fixture = Fixture::new().await;
        scenarios::a_quote_with_the_id_of_a_stored_one_is_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn two_quotes_of_one_deposit_output_are_both_kept() {
        let fixture = Fixture::new().await;
        scenarios::two_quotes_of_one_deposit_output_are_both_kept(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_store_sets_the_times_of_a_new_quote() {
        let fixture = Fixture::new().await;
        scenarios::the_store_sets_the_times_of_a_new_quote(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_quote_created_before_the_given_time_is_deleted() {
        let fixture = Fixture::new().await;
        scenarios::a_quote_created_before_the_given_time_is_deleted(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_quote_created_at_the_given_time_is_kept() {
        let fixture = Fixture::new().await;
        scenarios::a_quote_created_at_the_given_time_is_kept(&fixture.store).await;
    }

    #[tokio::test]
    async fn only_the_quotes_created_before_the_given_time_are_deleted() {
        let fixture = Fixture::new().await;
        scenarios::only_the_quotes_created_before_the_given_time_are_deleted(&fixture.store).await;
    }
}
