#![allow(clippy::needless_raw_string_hashes)]

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bitcoin::Transaction;
use bitcoin::secp256k1::PublicKey;
use deadpool_postgres::Pool;
use tokio_postgres::Row;
use uuid::Uuid;

use spark::services::TransferId;

use super::{execute, query, query_opt, transfer_uuid};
use crate::static_deposit::repository::{
    PendingCredit, StaticDepositClaimRecord, StaticDepositClaimStore,
};

struct StaticDepositClaimRow {
    id: String,
    user_identity_public_key: Vec<u8>,
    txid: String,
    vout: i32,
    network: String,
    deposit_address: String,
    credit_amount_sats: i64,
    is_instant: bool,
    deposit_amount_sats: i64,
    encrypted_deposit_secret_key: String,
    quote_signature: String,
    user_signature: String,
    transfer_id: Option<Uuid>,
    pending_transfer_id: Option<Uuid>,
    utxo_swap_id: Option<String>,
    prep_spend_tx: Vec<u8>,
    signed_spend_transaction: Option<Vec<u8>>,
    spend_broadcast_txid: Option<String>,
    spend_confirmed: bool,
    deposit_lost: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<&Row> for StaticDepositClaimRow {
    type Error = tokio_postgres::Error;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            user_identity_public_key: row.try_get("user_identity_public_key")?,
            txid: row.try_get("txid")?,
            vout: row.try_get("vout")?,
            network: row.try_get("network")?,
            deposit_address: row.try_get("deposit_address")?,
            credit_amount_sats: row.try_get("credit_amount_sats")?,
            is_instant: row.try_get("is_instant")?,
            deposit_amount_sats: row.try_get("deposit_amount_sats")?,
            encrypted_deposit_secret_key: row.try_get("encrypted_deposit_secret_key")?,
            quote_signature: row.try_get("quote_signature")?,
            user_signature: row.try_get("user_signature")?,
            transfer_id: row.try_get("transfer_id")?,
            pending_transfer_id: row.try_get("pending_transfer_id")?,
            utxo_swap_id: row.try_get("utxo_swap_id")?,
            prep_spend_tx: row.try_get("prep_spend_tx")?,
            signed_spend_transaction: row.try_get("signed_spend_transaction")?,
            spend_broadcast_txid: row.try_get("spend_broadcast_txid")?,
            spend_confirmed: row.try_get("spend_confirmed")?,
            deposit_lost: row.try_get("deposit_lost")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

impl TryFrom<StaticDepositClaimRow> for StaticDepositClaimRecord {
    type Error = String;

    fn try_from(row: StaticDepositClaimRow) -> Result<Self, String> {
        let spend_tx = bitcoin::consensus::deserialize(&row.prep_spend_tx)
            .map_err(|err| format!("invalid prep spend tx: {err}"))?;
        let signed_spend_transaction = row
            .signed_spend_transaction
            .as_deref()
            .map(bitcoin::consensus::deserialize)
            .transpose()
            .map_err(|err| format!("invalid signed spend transaction: {err}"))?;
        let pending_credit = row
            .pending_transfer_id
            .map(|transfer_id| {
                TransferId::from_str(&transfer_id.to_string())
                    .map(|transfer_id| PendingCredit { transfer_id })
                    .map_err(|e| format!("invalid pending transfer id: {e}"))
            })
            .transpose()?;
        Ok(StaticDepositClaimRecord {
            id: row.id,
            user_identity_public_key: PublicKey::from_slice(&row.user_identity_public_key)
                .map_err(|e| format!("invalid user public key: {e}"))?,
            txid: row.txid,
            vout: row.vout.cast_unsigned(),
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
            transfer_id: row.transfer_id.map(|id| id.to_string()),
            pending_credit,
            utxo_swap_id: row.utxo_swap_id,
            spend_tx,
            signed_spend_transaction,
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

fn claim(row: &Row) -> Result<StaticDepositClaimRecord, String> {
    StaticDepositClaimRow::try_from(row)
        .map_err(|e| e.to_string())?
        .try_into()
}

fn transfer_uuids(transfer_ids: &[String]) -> Result<Vec<Uuid>, String> {
    transfer_ids
        .iter()
        .map(|id| Uuid::parse_str(id).map_err(|e| e.to_string()))
        .collect()
}

pub struct PostgresStaticDepositClaimStore {
    pool: Arc<Pool>,
}

impl PostgresStaticDepositClaimStore {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl StaticDepositClaimStore for PostgresStaticDepositClaimStore {
    async fn insert(&self, record: &StaticDepositClaimRecord) -> Result<(), String> {
        let credit = record.pending_credit.as_ref();
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"INSERT INTO brz_ssp_static_deposit_claims
               (id, user_identity_public_key, txid, vout, network, deposit_address,
                credit_amount_sats, is_instant, deposit_amount_sats,
                encrypted_deposit_secret_key, quote_signature, user_signature,
                pending_transfer_id, prep_spend_tx)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"#,
            &[
                &record.id,
                &record.user_identity_public_key.serialize().to_vec(),
                &record.txid,
                &record.vout.cast_signed(),
                &record.network.to_string(),
                &record.deposit_address,
                &sats_to_i64(record.credit_amount_sats)?,
                &record.is_instant,
                &sats_to_i64(record.deposit_amount_sats)?,
                &record.encrypted_deposit_secret_key,
                &record.quote_signature,
                &record.user_signature,
                &credit.map(|credit| transfer_uuid(&credit.transfer_id)),
                &bitcoin::consensus::serialize(&record.spend_tx),
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<StaticDepositClaimRecord>, String> {
        query_opt(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"SELECT * FROM brz_ssp_static_deposit_claims WHERE id = $1"#,
            &[&id],
        )
        .await
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(claim)
        .transpose()
    }

    async fn get_by_utxo(
        &self,
        txid: &str,
        vout: u32,
    ) -> Result<Option<StaticDepositClaimRecord>, String> {
        query_opt(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"SELECT * FROM brz_ssp_static_deposit_claims WHERE txid = $1 AND vout = $2"#,
            &[&txid, &vout.cast_signed()],
        )
        .await
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(claim)
        .transpose()
    }

    async fn get_by_transfer_ids(
        &self,
        transfer_ids: &[String],
    ) -> Result<Vec<StaticDepositClaimRecord>, String> {
        query(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"SELECT * FROM brz_ssp_static_deposit_claims
               WHERE transfer_id = ANY($1)"#,
            &[&transfer_uuids(transfer_ids)?],
        )
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(claim)
        .collect()
    }

    async fn transfers_among(&self, transfer_ids: &[String]) -> Result<HashSet<String>, String> {
        let ids = query(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"SELECT transfer_id::text FROM brz_ssp_static_deposit_claims
               WHERE transfer_id = ANY($1)
             UNION
               SELECT pending_transfer_id::text FROM brz_ssp_static_deposit_claims
               WHERE pending_transfer_id = ANY($1)"#,
            &[&transfer_uuids(transfer_ids)?],
        )
        .await
        .map_err(|e| e.to_string())?;
        ids.iter()
            .map(|row| row.try_get(0))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())
    }

    async fn set_transfer_id(&self, id: &str, transfer_id: &str) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"UPDATE brz_ssp_static_deposit_claims
               SET transfer_id = $2,
                   pending_transfer_id = NULL,
                   updated_at = NOW()
               WHERE id = $1"#,
            &[
                &id,
                &Uuid::parse_str(transfer_id).map_err(|e| e.to_string())?,
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"DELETE FROM brz_ssp_static_deposit_claims WHERE id = $1"#,
            &[&id],
        )
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
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"UPDATE brz_ssp_static_deposit_claims
               SET transfer_id = $2, utxo_swap_id = $3,
                   pending_transfer_id = NULL,
                   updated_at = NOW()
               WHERE id = $1"#,
            &[
                &id,
                &Uuid::parse_str(transfer_id).map_err(|e| e.to_string())?,
                &utxo_swap_id,
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_signed_spend_transaction(
        &self,
        id: &str,
        transfer_id: &str,
        transaction: &Transaction,
    ) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"UPDATE brz_ssp_static_deposit_claims
               SET transfer_id = $2,
                   signed_spend_transaction = $3,
                   pending_transfer_id = NULL,
                   updated_at = NOW()
               WHERE id = $1"#,
            &[
                &id,
                &Uuid::parse_str(transfer_id).map_err(|e| e.to_string())?,
                &bitcoin::consensus::serialize(transaction),
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_spend_broadcast_txid(&self, id: &str, txid: &str) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"UPDATE brz_ssp_static_deposit_claims SET spend_broadcast_txid = $2, updated_at = NOW() WHERE id = $1"#,
            &[&id, &txid],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_spend_confirmed(&self, id: &str) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"UPDATE brz_ssp_static_deposit_claims SET spend_confirmed = TRUE, updated_at = NOW() WHERE id = $1"#,
            &[&id],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn set_deposit_lost(&self, id: &str) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"UPDATE brz_ssp_static_deposit_claims SET deposit_lost = TRUE, updated_at = NOW() WHERE id = $1"#,
            &[&id],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn pending(&self) -> Result<Vec<StaticDepositClaimRecord>, String> {
        // Must match `StaticDepositClaimRecord::is_pending`.
        query(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"SELECT * FROM brz_ssp_static_deposit_claims
               WHERE pending_transfer_id IS NOT NULL
                  OR (transfer_id IS NOT NULL AND NOT spend_confirmed AND NOT deposit_lost)
               ORDER BY created_at"#,
            &[],
        )
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(claim)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::PostgresStaticDepositClaimStore;
    use crate::postgresql::testing::TestDatabase;
    use crate::static_deposit::repository::scenarios::claims as scenarios;

    struct Fixture {
        store: PostgresStaticDepositClaimStore,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            let database = TestDatabase::new().await;
            Self {
                store: PostgresStaticDepositClaimStore::new(Arc::clone(&database.pool)),
                _database: database,
            }
        }
    }

    #[tokio::test]
    async fn a_claim_reads_back_as_it_was_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_reads_back_as_it_was_stored(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_claim_the_store_was_never_given_is_not_found() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_the_store_was_never_given_is_not_found(&fixture.store).await;
    }

    #[tokio::test]
    async fn each_claim_is_found_by_its_own_deposit_output() {
        let fixture = Fixture::new().await;
        scenarios::each_claim_is_found_by_its_own_deposit_output(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_claim_with_the_id_of_a_stored_one_is_refused() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_with_the_id_of_a_stored_one_is_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_second_claim_of_a_deposit_output_is_refused() {
        let fixture = Fixture::new().await;
        scenarios::a_second_claim_of_a_deposit_output_is_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_new_claim_is_stored_as_not_yet_credited() {
        let fixture = Fixture::new().await;
        scenarios::a_new_claim_is_stored_as_not_yet_credited(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_store_sets_the_times_of_a_new_claim() {
        let fixture = Fixture::new().await;
        scenarios::the_store_sets_the_times_of_a_new_claim(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_claim_with_a_pending_credit_is_pending() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_with_a_pending_credit_is_pending(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_claim_with_no_credit_is_not_pending() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_with_no_credit_is_not_pending(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_credited_claim_has_its_transfer_and_no_pending_credit() {
        let fixture = Fixture::new().await;
        scenarios::a_credited_claim_has_its_transfer_and_no_pending_credit(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_reserved_claim_has_its_transfer_and_swap_and_no_pending_credit() {
        let fixture = Fixture::new().await;
        scenarios::a_reserved_claim_has_its_transfer_and_swap_and_no_pending_credit(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn a_claim_whose_spend_is_co_signed_has_its_transfer_and_signed_spend() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_whose_spend_is_co_signed_has_its_transfer_and_signed_spend(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn a_reserved_claim_keeps_its_swap_when_its_spend_is_co_signed() {
        let fixture = Fixture::new().await;
        scenarios::a_reserved_claim_keeps_its_swap_when_its_spend_is_co_signed(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn a_claim_reads_the_same_however_the_store_finds_it() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_reads_the_same_however_the_store_finds_it(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_transfer_id_that_is_not_a_uuid_is_refused() {
        let fixture = Fixture::new().await;
        scenarios::a_transfer_id_that_is_not_a_uuid_is_refused(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_claim_stays_pending_until_its_spend_confirms() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_stays_pending_until_its_spend_confirms(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_claim_whose_spend_was_broadcast_has_the_txid_of_the_spend() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_whose_spend_was_broadcast_has_the_txid_of_the_spend(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn a_claim_whose_deposit_is_lost_is_no_longer_pending() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_whose_deposit_is_lost_is_no_longer_pending(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_claim_no_longer_pending_is_still_found_by_its_output_and_transfer() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_no_longer_pending_is_still_found_by_its_output_and_transfer(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn the_pending_claims_are_those_the_record_calls_pending() {
        let fixture = Fixture::new().await;
        scenarios::the_pending_claims_are_those_the_record_calls_pending(&fixture.store).await;
    }

    #[tokio::test]
    async fn pending_claims_are_listed_oldest_first() {
        let fixture = Fixture::new().await;
        scenarios::pending_claims_are_listed_oldest_first(&fixture.store).await;
    }

    #[tokio::test]
    async fn no_claim_is_found_by_no_transfers() {
        let fixture = Fixture::new().await;
        scenarios::no_claim_is_found_by_no_transfers(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_credited_claim_is_found_by_its_transfer() {
        let fixture = Fixture::new().await;
        scenarios::a_credited_claim_is_found_by_its_transfer(&fixture.store).await;
    }

    #[tokio::test]
    async fn claims_are_found_by_several_transfers_at_once() {
        let fixture = Fixture::new().await;
        scenarios::claims_are_found_by_several_transfers_at_once(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_transfer_of_no_claim_finds_no_claim() {
        let fixture = Fixture::new().await;
        scenarios::a_transfer_of_no_claim_finds_no_claim(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_transfer_given_twice_finds_its_claim_once() {
        let fixture = Fixture::new().await;
        scenarios::a_transfer_given_twice_finds_its_claim_once(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_claim_is_not_found_by_the_transfer_of_its_pending_credit() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_is_not_found_by_the_transfer_of_its_pending_credit(&fixture.store).await;
    }

    #[tokio::test]
    async fn no_credit_is_found_among_no_transfers() {
        let fixture = Fixture::new().await;
        scenarios::no_credit_is_found_among_no_transfers(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_pending_credit_is_found_among_transfers() {
        let fixture = Fixture::new().await;
        scenarios::a_pending_credit_is_found_among_transfers(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_made_credit_is_found_among_transfers() {
        let fixture = Fixture::new().await;
        scenarios::a_made_credit_is_found_among_transfers(&fixture.store).await;
    }

    #[tokio::test]
    async fn every_credit_among_several_transfers_is_found() {
        let fixture = Fixture::new().await;
        scenarios::every_credit_among_several_transfers_is_found(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_transfer_that_is_no_claims_credit_is_left_out() {
        let fixture = Fixture::new().await;
        scenarios::a_transfer_that_is_no_claims_credit_is_left_out(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_transfer_given_twice_is_found_as_one_credit() {
        let fixture = Fixture::new().await;
        scenarios::a_transfer_given_twice_is_found_as_one_credit(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_pending_credit_is_forgotten_once_another_transfer_credits_its_claim() {
        let fixture = Fixture::new().await;
        scenarios::a_pending_credit_is_forgotten_once_another_transfer_credits_its_claim(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn a_deleted_claim_is_gone() {
        let fixture = Fixture::new().await;
        scenarios::a_deleted_claim_is_gone(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_deposit_output_of_a_deleted_claim_can_be_claimed_again() {
        let fixture = Fixture::new().await;
        scenarios::the_deposit_output_of_a_deleted_claim_can_be_claimed_again(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_write_to_one_claim_leaves_another_as_it_was() {
        let fixture = Fixture::new().await;
        scenarios::a_write_to_one_claim_leaves_another_as_it_was(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_write_to_a_claim_the_store_does_not_have_stores_nothing() {
        let fixture = Fixture::new().await;
        scenarios::a_write_to_a_claim_the_store_does_not_have_stores_nothing(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_claim_keeps_the_time_it_was_created() {
        let fixture = Fixture::new().await;
        scenarios::a_claim_keeps_the_time_it_was_created(&fixture.store).await;
    }

    #[tokio::test]
    async fn each_write_to_a_claim_moves_the_time_it_last_changed() {
        let fixture = Fixture::new().await;
        scenarios::each_write_to_a_claim_moves_the_time_it_last_changed(&fixture.store).await;
    }
}
