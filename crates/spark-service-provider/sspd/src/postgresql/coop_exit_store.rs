#![allow(clippy::needless_raw_string_hashes)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bitcoin::secp256k1::PublicKey;
use spark::services::TransferId;
use sqlx::PgPool;
use uuid::Uuid;

use crate::coop_exit::repository::{CoopExitRecord, CoopExitStore};
use crate::wallet::onchain::record_spends;

#[derive(sqlx::FromRow)]
struct CoopExitRow {
    id: String,
    user_identity_public_key: Vec<u8>,
    user_transfer_id: Uuid,
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
            user_transfer_id: TransferId::from_str(&row.user_transfer_id.to_string())
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
               VALUES ($1, $2, $3::uuid, $4, $5, $6, $7, $8, $9, $10)"#,
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
            sqlx::query(
                r#"DELETE FROM brz_ssp_onchain_wallet_spends WHERE spending_txid = ANY($1)"#,
            )
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
            r#"SELECT * FROM brz_ssp_coop_exit_requests WHERE user_transfer_id = $1::uuid"#,
        )
        .bind(transfer_id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .map(CoopExitRecord::try_from)
        .transpose()
    }

    async fn get_by_transfer_ids(
        &self,
        transfer_ids: &[TransferId],
    ) -> Result<Vec<CoopExitRecord>, String> {
        let ids: Vec<String> = transfer_ids.iter().map(ToString::to_string).collect();
        sqlx::query_as::<_, CoopExitRow>(
            r#"SELECT * FROM brz_ssp_coop_exit_requests
               WHERE user_transfer_id = ANY($1::uuid[])"#,
        )
        .bind(&ids)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(CoopExitRecord::try_from)
        .collect()
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
            sqlx::query(r#"DELETE FROM brz_ssp_onchain_wallet_spends WHERE spending_txid = $1"#)
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

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;
    use crate::coop_exit::repository::scenarios::{self, leaves, request};
    use crate::postgresql::testing::TestDatabase;

    /// A wallet coin, with the transaction that has it as an input.
    type Spend = (String, i32, String);

    struct Fixture {
        store: PostgresCoopExitStore,
        pool: Arc<PgPool>,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            let database = TestDatabase::new().await;
            let pool = Arc::clone(&database.pool);
            Self {
                store: PostgresCoopExitStore::new(Arc::clone(&pool)),
                pool,
                _database: database,
            }
        }

        async fn created(&self, id: &str, minutes_ago: i32) {
            sqlx::query(
                "UPDATE brz_ssp_coop_exit_requests
                 SET created_at = NOW() - make_interval(mins => $2) WHERE id = $1",
            )
            .bind(id)
            .bind(minutes_ago)
            .execute(&*self.pool)
            .await
            .unwrap();
        }

        async fn withheld(&self) -> Vec<Spend> {
            let mut spends: Vec<Spend> = sqlx::query_as(
                "SELECT tx_id, output_index, spending_txid FROM brz_ssp_onchain_wallet_spends",
            )
            .fetch_all(&*self.pool)
            .await
            .unwrap();
            spends.sort();
            spends
        }
    }

    fn exit_spends(request: &CoopExitRecord) -> Vec<Spend> {
        let exit_txid = &request.coop_exit_txid;
        request
            .prevouts
            .iter()
            .map(|coin| {
                (
                    coin.txid.clone(),
                    coin.vout.cast_signed(),
                    exit_txid.clone(),
                )
            })
            .collect()
    }

    fn connector_spend(request: &CoopExitRecord) -> Spend {
        let connector: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&request.raw_connector_tx).unwrap();
        let funding = connector.input[0].previous_output;
        (
            funding.txid.to_string(),
            funding.vout.cast_signed(),
            connector.compute_txid().to_string(),
        )
    }

    fn spends(requests: &[&CoopExitRecord]) -> Vec<Spend> {
        let mut spends: Vec<Spend> = requests
            .iter()
            .flat_map(|request| {
                let mut spends = exit_spends(request);
                spends.push(connector_spend(request));
                spends
            })
            .collect();
        spends.sort();
        spends
    }

    #[tokio::test]
    async fn a_request_reads_back_as_it_was_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_request_reads_back_as_it_was_stored(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_request_is_found_by_the_transfer_the_user_paid_for_it_with() {
        let fixture = Fixture::new().await;
        scenarios::a_request_is_found_by_the_transfer_the_user_paid_for_it_with(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn no_request_is_found_under_an_id_or_a_transfer_the_store_does_not_have() {
        let fixture = Fixture::new().await;
        scenarios::no_request_is_found_under_an_id_or_a_transfer_the_store_does_not_have(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn the_requests_paid_for_with_the_given_transfers_are_returned() {
        let fixture = Fixture::new().await;
        scenarios::the_requests_paid_for_with_the_given_transfers_are_returned(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn the_request_of_a_transfer_given_twice_is_returned_once() {
        let fixture = Fixture::new().await;
        scenarios::the_request_of_a_transfer_given_twice_is_returned_once(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_second_request_paid_for_with_the_same_transfer_is_not_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_second_request_paid_for_with_the_same_transfer_is_not_stored(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn of_concurrent_requests_paid_for_with_one_transfer_one_is_stored() {
        let fixture = Fixture::new().await;
        scenarios::of_concurrent_requests_paid_for_with_one_transfer_one_is_stored(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn a_request_reads_back_in_each_state_it_was_changed_to() {
        let fixture = Fixture::new().await;
        scenarios::a_request_reads_back_in_each_state_it_was_changed_to(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_request_stays_as_it_was_when_another_changes_state() {
        let fixture = Fixture::new().await;
        scenarios::a_request_stays_as_it_was_when_another_changes_state(&fixture.store).await;
    }

    #[tokio::test]
    async fn nothing_is_stored_for_a_change_to_a_request_the_store_does_not_have() {
        let fixture = Fixture::new().await;
        scenarios::nothing_is_stored_for_a_change_to_a_request_the_store_does_not_have(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn the_store_sets_the_time_of_the_last_update_at_each_change_of_state() {
        let fixture = Fixture::new().await;
        scenarios::the_store_sets_the_time_of_the_last_update_at_each_change_of_state(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn only_requests_that_are_not_completed_are_returned_as_incomplete() {
        let fixture = Fixture::new().await;
        scenarios::only_requests_that_are_not_completed_are_returned_as_incomplete(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn only_completed_requests_that_are_not_settled_are_returned_as_pending() {
        let fixture = Fixture::new().await;
        scenarios::only_completed_requests_that_are_not_settled_are_returned_as_pending(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn a_request_is_listed_in_every_state() {
        let fixture = Fixture::new().await;
        scenarios::a_request_is_listed_in_every_state(&fixture.store).await;
    }

    #[tokio::test]
    async fn only_requests_created_before_the_cutoff_are_returned_as_incomplete() {
        let fixture = Fixture::new().await;
        scenarios::only_requests_created_before_the_cutoff_are_returned_as_incomplete(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn the_latest_requests_are_listed_newest_first() {
        let fixture = Fixture::new().await;
        scenarios::the_latest_requests_are_listed_newest_first(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_leaf_of_an_open_request_is_found_in_one() {
        let fixture = Fixture::new().await;
        scenarios::a_leaf_of_an_open_request_is_found_in_one(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_request_naming_a_leaf_of_another_open_request_is_not_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_request_naming_a_leaf_of_another_open_request_is_not_stored(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn of_concurrent_requests_naming_one_leaf_one_is_stored() {
        let fixture = Fixture::new().await;
        scenarios::of_concurrent_requests_naming_one_leaf_one_is_stored(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_leaf_of_a_completed_request_is_in_no_open_request() {
        let fixture = Fixture::new().await;
        scenarios::a_leaf_of_a_completed_request_is_in_no_open_request(&fixture.store).await;
    }

    #[tokio::test]
    async fn an_abandoned_request_is_no_longer_stored() {
        let fixture = Fixture::new().await;
        scenarios::an_abandoned_request_is_no_longer_stored(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_transfer_and_the_leaves_of_an_abandoned_request_are_free_for_a_new_one() {
        let fixture = Fixture::new().await;
        scenarios::the_transfer_and_the_leaves_of_an_abandoned_request_are_free_for_a_new_one(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn a_completed_request_is_not_abandoned() {
        let fixture = Fixture::new().await;
        scenarios::a_completed_request_is_not_abandoned(&fixture.store).await;
    }

    #[tokio::test]
    async fn pending_requests_are_returned_oldest_first() {
        let fixture = Fixture::new().await;
        scenarios::pending_requests_are_returned_oldest_first(&fixture.store).await;
    }

    // This test is not a scenario because it writes the creation times to the
    // table. It inserts the requests in an order other than the order of those
    // times.
    #[tokio::test]
    async fn incomplete_requests_are_returned_oldest_first() {
        let fixture = Fixture::new().await;
        for (id, minutes_ago) in [("middle", 20), ("oldest", 30), ("newest", 10)] {
            fixture.store.insert(&request(id), &[]).await.unwrap();
            fixture.created(id, minutes_ago).await;
        }

        let tomorrow = Utc::now() + Duration::days(1);
        let incomplete = fixture.store.incomplete_before(tomorrow).await.unwrap();

        let ids: Vec<&str> = incomplete.iter().map(|found| found.id.as_str()).collect();
        assert_eq!(ids, ["oldest", "middle", "newest"]);
    }

    #[tokio::test]
    async fn the_store_sets_the_state_and_the_times_of_a_new_request_itself() {
        let fixture = Fixture::new().await;
        scenarios::the_store_sets_the_state_and_the_times_of_a_new_request_itself(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn a_request_whose_transactions_the_store_cannot_read_is_not_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_request_whose_transactions_the_store_cannot_read_is_not_stored(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn a_request_for_more_sats_than_the_store_can_hold_is_not_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_request_for_more_sats_than_the_store_can_hold_is_not_stored(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn a_request_under_the_id_of_another_is_not_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_request_under_the_id_of_another_is_not_stored(&fixture.store).await;
    }

    #[tokio::test]
    async fn a_request_naming_a_leaf_twice_is_not_stored() {
        let fixture = Fixture::new().await;
        scenarios::a_request_naming_a_leaf_twice_is_not_stored(&fixture.store).await;
    }

    #[tokio::test]
    async fn the_wallet_coins_a_requests_transactions_spend_are_withheld() {
        let fixture = Fixture::new().await;
        let request = request("a");

        fixture.store.insert(&request, &[]).await.unwrap();

        let withheld = fixture.withheld().await;
        assert_eq!(withheld, spends(&[&request]));
        assert_eq!(withheld.len(), 3);
    }

    #[tokio::test]
    async fn the_coins_of_an_abandoned_request_are_released_and_no_others() {
        let fixture = Fixture::new().await;
        let (abandoned, kept) = (request("abandoned"), request("kept"));
        fixture.store.insert(&abandoned, &[]).await.unwrap();
        fixture.store.insert(&kept, &[]).await.unwrap();

        fixture.store.abandon("abandoned").await.unwrap();

        assert_eq!(fixture.withheld().await, spends(&[&kept]));
    }

    #[tokio::test]
    async fn the_coins_of_a_completed_request_stay_withheld_when_it_is_abandoned() {
        let fixture = Fixture::new().await;
        let request = request("a");
        fixture.store.insert(&request, &[]).await.unwrap();
        fixture.store.set_completed("a").await.unwrap();

        fixture.store.abandon("a").await.unwrap();

        assert_eq!(fixture.withheld().await, spends(&[&request]));
    }

    #[tokio::test]
    async fn a_request_abandoned_while_it_is_completed_is_gone_or_completed_with_its_coins() {
        let fixture = Fixture::new().await;
        let store = &fixture.store;
        let mut completed = Vec::new();
        for (round, abandon_first) in [true, false].repeat(5).into_iter().enumerate() {
            let request = request(&round.to_string());
            store.insert(&request, &[]).await.unwrap();

            // The test starts the abandon and the completion in alternating
            // order, to vary which of them Postgres runs first.
            let abandon = store.abandon(&request.id);
            let complete = store.set_completed(&request.id);
            if abandon_first {
                let (abandoned, set_completed) = tokio::join!(abandon, complete);
                abandoned.and(set_completed).unwrap();
            } else {
                let (set_completed, abandoned) = tokio::join!(complete, abandon);
                set_completed.and(abandoned).unwrap();
            }

            if let Some(stored) = store.get(&request.id).await.unwrap() {
                assert!(stored.completed);
                completed.push(request);
            }
        }

        let completed: Vec<&CoopExitRecord> = completed.iter().collect();
        assert_eq!(fixture.withheld().await, spends(&completed));
    }

    #[tokio::test]
    async fn the_coin_the_connector_transaction_spends_is_released_once_the_leaves_are_claimed() {
        let fixture = Fixture::new().await;
        let (claimed, other) = (request("claimed"), request("other"));
        for request in [&claimed, &other] {
            fixture.store.insert(request, &[]).await.unwrap();
            fixture.store.set_completed(&request.id).await.unwrap();
        }

        fixture.store.set_leaves_claimed("claimed").await.unwrap();

        let mut expected = exit_spends(&claimed);
        expected.extend(spends(&[&other]));
        expected.sort();
        assert_eq!(fixture.withheld().await, expected);
    }

    #[tokio::test]
    async fn no_coins_are_withheld_for_a_request_that_is_not_stored() {
        let fixture = Fixture::new().await;
        let (stored, refused) = (request("stored"), request("refused"));
        let leaf = leaves(&["a"]);
        fixture.store.insert(&stored, &leaf).await.unwrap();

        let result = fixture.store.insert(&refused, &leaf).await;

        assert!(result.is_err());
        assert_eq!(fixture.withheld().await, spends(&[&stored]));
    }
}
