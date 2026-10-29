#![allow(clippy::needless_raw_string_hashes)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use deadpool_postgres::Pool;
use tokio_postgres::Row;

use super::{execute, query_opt};
use crate::static_deposit::repository::{
    InstantStaticDepositQuoteRecord, InstantStaticDepositQuoteStore,
};

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

impl TryFrom<&Row> for InstantQuoteRow {
    type Error = tokio_postgres::Error;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            txid: row.try_get("txid")?,
            vout: row.try_get("vout")?,
            network: row.try_get("network")?,
            deposit_amount_sats: row.try_get("deposit_amount_sats")?,
            credit_amount_sats: row.try_get("credit_amount_sats")?,
            destination_address: row.try_get("destination_address")?,
            quote_signature: row.try_get("quote_signature")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
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
    pool: Arc<Pool>,
}

impl PostgresInstantStaticDepositQuoteStore {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

fn sats_to_i64(sats: u64) -> Result<i64, String> {
    i64::try_from(sats).map_err(|e| format!("amount exceeds i64: {e}"))
}

fn quote(row: &Row) -> Result<InstantStaticDepositQuoteRecord, String> {
    InstantQuoteRow::try_from(row)
        .map_err(|e| e.to_string())?
        .try_into()
}

#[async_trait]
impl InstantStaticDepositQuoteStore for PostgresInstantStaticDepositQuoteStore {
    async fn insert(&self, record: &InstantStaticDepositQuoteRecord) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"INSERT INTO brz_ssp_static_deposit_instant_quotes
               (id, txid, vout, network, deposit_amount_sats, credit_amount_sats,
                destination_address, quote_signature)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
            &[
                &record.id,
                &record.txid,
                &record.vout.cast_signed(),
                &record.network.to_string(),
                &sats_to_i64(record.deposit_amount_sats)?,
                &sats_to_i64(record.credit_amount_sats)?,
                &record.destination_address,
                &record.quote_signature,
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<InstantStaticDepositQuoteRecord>, String> {
        query_opt(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"SELECT * FROM brz_ssp_static_deposit_instant_quotes WHERE id = $1"#,
            &[&id],
        )
        .await
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(quote)
        .transpose()
    }

    async fn delete_created_before(
        &self,
        time: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"DELETE FROM brz_ssp_static_deposit_instant_quotes WHERE created_at < $1"#,
            &[&time],
        )
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
