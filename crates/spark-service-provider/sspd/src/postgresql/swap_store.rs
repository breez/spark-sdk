#![allow(clippy::needless_raw_string_hashes)]

use std::collections::HashMap;
use std::sync::Arc;

use deadpool_postgres::Pool;
use spark::services::TransferId;
use tokio_postgres::Row;
use uuid::Uuid;

use super::{execute, query, query_one, query_opt, transfer_uuid};
use crate::swap::SwapStore;
use crate::swap::repository::{SwapDetail, SwapLeaf, SwapRecord};

const DIRECTION_OUTBOUND: &str = "outbound";
const DIRECTION_INBOUND: &str = "inbound";

pub struct SwapRepository {
    pool: Arc<Pool>,
}

impl SwapRepository {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

const SWAP_COLUMNS: &str = "id, user_identity_public_key, user_transfer_id, counter_transfer_id, \
     total_amount_sats, target_amount_sats, fee_sats";

fn swap_record(row: &Row) -> Result<SwapRecord, tokio_postgres::Error> {
    let user_transfer_id: Uuid = row.try_get(2)?;
    let counter_transfer_id: Uuid = row.try_get(3)?;
    Ok(SwapRecord {
        id: row.try_get(0)?,
        user_identity_public_key: row.try_get(1)?,
        user_transfer_id: TransferId::from_bytes(user_transfer_id.into_bytes()),
        counter_transfer_id: TransferId::from_bytes(counter_transfer_id.into_bytes()),
        total_amount_sats: row.try_get(4)?,
        target_amount_sats: row.try_get(5)?,
        fee_sats: row.try_get(6)?,
    })
}

fn swap_records(rows: &[Row]) -> Result<Vec<SwapRecord>, String> {
    rows.iter()
        .map(swap_record)
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}

fn leaf_columns(leaves: &[SwapLeaf]) -> (Vec<&str>, Vec<i64>) {
    leaves
        .iter()
        .map(|leaf| (leaf.leaf_id.as_str(), leaf.value_sats))
        .unzip()
}

impl SwapRepository {
    async fn with_leaves(&self, swaps: Vec<SwapRecord>) -> Result<Vec<SwapDetail>, String> {
        let ids: Vec<&str> = swaps.iter().map(|swap| swap.id.as_str()).collect();
        let leaves: Vec<(String, String, String, i64)> = query(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"SELECT swap_id, direction, leaf_id, value_sats FROM brz_ssp_swap_leaves
               WHERE swap_id = ANY($1)
               ORDER BY swap_id, direction, leaf_id"#,
            &[&ids],
        )
        .await
        .and_then(|rows| {
            rows.iter()
                .map(|row| {
                    Ok((
                        row.try_get(0)?,
                        row.try_get(1)?,
                        row.try_get(2)?,
                        row.try_get(3)?,
                    ))
                })
                .collect()
        })
        .map_err(|e| e.to_string())?;
        let mut by_swap: HashMap<String, (Vec<SwapLeaf>, Vec<SwapLeaf>)> = HashMap::new();
        for (swap_id, direction, leaf_id, value_sats) in leaves {
            let (outbound, inbound) = by_swap.entry(swap_id).or_default();
            let leaf = SwapLeaf {
                leaf_id,
                value_sats,
            };
            if direction == DIRECTION_INBOUND {
                inbound.push(leaf);
            } else {
                outbound.push(leaf);
            }
        }
        Ok(swaps
            .into_iter()
            .map(|swap| {
                let (outbound, inbound) = by_swap.remove(&swap.id).unwrap_or_default();
                SwapDetail {
                    swap,
                    outbound,
                    inbound,
                }
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl SwapStore for SwapRepository {
    async fn insert_swap(&self, swap: &SwapRecord) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"INSERT INTO brz_ssp_swaps
               (id, user_identity_public_key, user_transfer_id, counter_transfer_id,
                total_amount_sats, target_amount_sats, fee_sats)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            &[
                &swap.id,
                &swap.user_identity_public_key,
                &transfer_uuid(&swap.user_transfer_id),
                &transfer_uuid(&swap.counter_transfer_id),
                &swap.total_amount_sats,
                &swap.target_amount_sats,
                &swap.fee_sats,
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get_by_user_transfer_id(
        &self,
        user_transfer_id: &str,
    ) -> Result<Option<SwapRecord>, String> {
        let user_transfer_id = Uuid::parse_str(user_transfer_id).map_err(|e| e.to_string())?;
        let row = query_opt(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            &format!("SELECT {SWAP_COLUMNS} FROM brz_ssp_swaps WHERE user_transfer_id = $1"),
            &[&user_transfer_id],
        )
        .await
        .map_err(|e| e.to_string())?;
        row.as_ref()
            .map(swap_record)
            .transpose()
            .map_err(|e| e.to_string())
    }

    async fn delete_swap(&self, swap_id: &str) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"DELETE FROM brz_ssp_swaps WHERE id = $1"#,
            &[&swap_id],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get_unclaimed_swaps(&self) -> Result<Vec<SwapRecord>, String> {
        let rows = query(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            &format!(
                "SELECT {SWAP_COLUMNS} FROM brz_ssp_swaps WHERE NOT claimed ORDER BY created_at"
            ),
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
        swap_records(&rows)
    }

    async fn has_unclaimed_swaps(&self) -> Result<bool, String> {
        query_one(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            "SELECT EXISTS (SELECT 1 FROM brz_ssp_swaps WHERE NOT claimed)",
            &[],
        )
        .await
        .and_then(|row| row.try_get(0))
        .map_err(|e| e.to_string())
    }

    async fn list_swaps(&self, limit: u32) -> Result<Vec<SwapDetail>, String> {
        let rows = query(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            &format!("SELECT {SWAP_COLUMNS} FROM brz_ssp_swaps ORDER BY created_at DESC LIMIT $1"),
            &[&i64::from(limit)],
        )
        .await
        .map_err(|e| e.to_string())?;
        self.with_leaves(swap_records(&rows)?).await
    }

    async fn counter_transfers_among(
        &self,
        transfer_ids: &[String],
    ) -> Result<std::collections::HashSet<String>, String> {
        let transfer_ids = transfer_ids
            .iter()
            .map(|id| Uuid::parse_str(id))
            .collect::<Result<Vec<Uuid>, _>>()
            .map_err(|e| e.to_string())?;
        let ids = query(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            "SELECT counter_transfer_id::text FROM brz_ssp_swaps
             WHERE counter_transfer_id = ANY($1)",
            &[&transfer_ids],
        )
        .await
        .map_err(|e| e.to_string())?;
        ids.iter()
            .map(|row| row.try_get(0))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())
    }

    async fn record_outbound_leaves(
        &self,
        swap_id: &str,
        outbound_leaves: &[SwapLeaf],
    ) -> Result<(), String> {
        let (leaf_ids, values) = leaf_columns(outbound_leaves);
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"INSERT INTO brz_ssp_swap_leaves (swap_id, direction, leaf_id, value_sats)
               SELECT $1, $2, leaf.leaf_id, leaf.value_sats
               FROM UNNEST($3::text[], $4::bigint[]) AS leaf (leaf_id, value_sats)
               ON CONFLICT (swap_id, direction, leaf_id) DO NOTHING"#,
            &[&swap_id, &DIRECTION_OUTBOUND, &leaf_ids, &values],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn record_inbound_leaves(
        &self,
        swap_id: &str,
        inbound_leaves: &[SwapLeaf],
    ) -> Result<(), String> {
        let (leaf_ids, values) = leaf_columns(inbound_leaves);
        // Postgres stores the leaves and marks the swap claimed in one
        // statement, so it does both or neither.
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"WITH stored AS (
                   INSERT INTO brz_ssp_swap_leaves (swap_id, direction, leaf_id, value_sats)
                   SELECT $1, $2, leaf.leaf_id, leaf.value_sats
                   FROM UNNEST($3::text[], $4::bigint[]) AS leaf (leaf_id, value_sats)
                   ON CONFLICT (swap_id, direction, leaf_id) DO NOTHING
               )
               UPDATE brz_ssp_swaps SET claimed = TRUE WHERE id = $1"#,
            &[&swap_id, &DIRECTION_INBOUND, &leaf_ids, &values],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use futures::future::join_all;

    use super::*;
    use crate::postgresql::testing::TestDatabase;

    struct Fixture {
        store: SwapRepository,
        pool: Arc<Pool>,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            let database = TestDatabase::new().await;
            let pool = Arc::clone(&database.pool);
            Self {
                store: SwapRepository::new(Arc::clone(&pool)),
                pool,
                _database: database,
            }
        }

        async fn swap(&self) -> SwapRecord {
            let swap = new_swap();
            self.store.insert_swap(&swap).await.unwrap();
            swap
        }

        async fn swap_created(&self, minutes_ago: i32) -> SwapRecord {
            let swap = self.swap().await;
            let client = self.pool.get().await.unwrap();
            execute(
                &client,
                "UPDATE brz_ssp_swaps
                 SET created_at = NOW() - make_interval(mins => $2) WHERE id = $1",
                &[&swap.id, &minutes_ago],
            )
            .await
            .unwrap();
            swap
        }

        async fn add_check(&self, table: &str, check: &str) {
            let client = self.pool.get().await.unwrap();
            client
                .batch_execute(&format!("ALTER TABLE {table} ADD CHECK ({check})"))
                .await
                .unwrap();
        }

        async fn unclaimed(&self) -> Vec<String> {
            let swaps = self.store.get_unclaimed_swaps().await.unwrap();
            swaps.into_iter().map(|swap| swap.id).collect()
        }

        async fn listed(&self, limit: u32) -> Vec<String> {
            let swaps = self.store.list_swaps(limit).await.unwrap();
            swaps.into_iter().map(|detail| detail.swap.id).collect()
        }

        async fn detail(&self, swap: &SwapRecord) -> SwapDetail {
            let swaps = self.store.list_swaps(100).await.unwrap();
            let listed = swaps.into_iter().find(|detail| detail.swap.id == swap.id);
            listed.unwrap()
        }

        async fn stored_leaves(&self) -> i64 {
            let client = self.pool.get().await.unwrap();
            query_one(&client, "SELECT COUNT(*) FROM brz_ssp_swap_leaves", &[])
                .await
                .unwrap()
                .get(0)
        }
    }

    fn new_swap() -> SwapRecord {
        let user = SecretKey::from_slice(&[1; 32])
            .unwrap()
            .public_key(&Secp256k1::new());
        SwapRecord {
            id: Uuid::now_v7().to_string(),
            user_identity_public_key: user.serialize().to_vec(),
            user_transfer_id: TransferId::generate(),
            counter_transfer_id: TransferId::generate(),
            total_amount_sats: 3000,
            target_amount_sats: 3000,
            fee_sats: 0,
        }
    }

    fn leaf(leaf_id: &str, value_sats: i64) -> SwapLeaf {
        SwapLeaf {
            leaf_id: leaf_id.to_string(),
            value_sats,
        }
    }

    /// Swap leaves have no equality, so a test compares their ids and values.
    fn pairs(leaves: &[SwapLeaf]) -> Vec<(&str, i64)> {
        leaves
            .iter()
            .map(|leaf| (leaf.leaf_id.as_str(), leaf.value_sats))
            .collect()
    }

    /// Swap records have no equality, so a test compares them by how they
    /// print.
    fn fields(swap: &SwapRecord) -> String {
        format!("{swap:?}")
    }

    #[tokio::test]
    async fn a_swap_reads_back_as_it_was_stored() {
        let fixture = Fixture::new().await;
        let swap = SwapRecord {
            total_amount_sats: 5000,
            target_amount_sats: 4900,
            fee_sats: 100,
            ..new_swap()
        };
        fixture.store.insert_swap(&swap).await.unwrap();

        let by_transfer = fixture
            .store
            .get_by_user_transfer_id(&swap.user_transfer_id.to_string())
            .await
            .unwrap();
        let unclaimed = fixture.store.get_unclaimed_swaps().await.unwrap();
        let listed = fixture.store.list_swaps(10).await.unwrap();

        let read: Vec<String> = by_transfer
            .iter()
            .chain(&unclaimed)
            .chain(listed.iter().map(|detail| &detail.swap))
            .map(fields)
            .collect();
        assert_eq!(read, vec![fields(&swap); 3]);
    }

    #[tokio::test]
    async fn a_swap_is_found_by_the_users_transfer_and_by_no_other() {
        let fixture = Fixture::new().await;
        let (swap, other) = (fixture.swap().await, fixture.swap().await);
        let found = async |transfer_id: &TransferId| {
            let found = fixture
                .store
                .get_by_user_transfer_id(&transfer_id.to_string())
                .await;
            found.unwrap().map(|found| found.id)
        };

        assert_eq!(found(&swap.user_transfer_id).await, Some(swap.id.clone()));
        assert_eq!(found(&other.user_transfer_id).await, Some(other.id));
        assert_eq!(found(&swap.counter_transfer_id).await, None);
        assert_eq!(found(&TransferId::generate()).await, None);
    }

    #[tokio::test]
    async fn a_second_swap_for_the_same_user_transfer_is_not_stored() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;
        let second = SwapRecord {
            user_transfer_id: swap.user_transfer_id.clone(),
            ..new_swap()
        };

        let swap_again = fixture.store.insert_swap(&swap).await;
        let second_stored = fixture.store.insert_swap(&second).await;

        assert!(swap_again.is_err());
        assert!(second_stored.is_err());
        assert_eq!(fixture.listed(10).await, [swap.id]);
    }

    #[tokio::test]
    async fn of_concurrent_swaps_for_one_user_transfer_one_is_stored() {
        let fixture = Fixture::new().await;
        let user_transfer_id = TransferId::generate();
        let swaps = [(); 6].map(|()| SwapRecord {
            user_transfer_id: user_transfer_id.clone(),
            ..new_swap()
        });

        let stored = join_all(swaps.iter().map(|swap| fixture.store.insert_swap(swap))).await;

        assert_eq!(stored.iter().filter(|stored| stored.is_ok()).count(), 1);
        assert_eq!(fixture.listed(10).await.len(), 1);
    }

    #[tokio::test]
    async fn a_swap_is_unclaimed_until_its_inbound_leaves_are_stored() {
        let fixture = Fixture::new().await;
        assert!(fixture.unclaimed().await.is_empty());
        assert!(!fixture.store.has_unclaimed_swaps().await.unwrap());

        let swap = fixture.swap().await;
        assert_eq!(fixture.unclaimed().await, std::slice::from_ref(&swap.id));
        assert!(fixture.store.has_unclaimed_swaps().await.unwrap());

        fixture
            .store
            .record_inbound_leaves(&swap.id, &[leaf("a", 3000)])
            .await
            .unwrap();
        assert!(fixture.unclaimed().await.is_empty());
        assert!(!fixture.store.has_unclaimed_swaps().await.unwrap());
    }

    #[tokio::test]
    async fn the_unclaimed_swaps_are_returned_oldest_first() {
        let fixture = Fixture::new().await;
        let middle = fixture.swap_created(20).await;
        let oldest = fixture.swap_created(40).await;
        let claimed = fixture.swap_created(30).await;
        let newest = fixture.swap_created(10).await;
        fixture
            .store
            .record_inbound_leaves(&claimed.id, &[])
            .await
            .unwrap();

        let unclaimed = fixture.unclaimed().await;

        assert_eq!(unclaimed, [oldest.id, middle.id, newest.id]);
    }

    #[tokio::test]
    async fn a_swap_stays_unclaimed_when_another_is_claimed() {
        let fixture = Fixture::new().await;
        let (claimed, unclaimed) = (fixture.swap().await, fixture.swap().await);

        fixture
            .store
            .record_inbound_leaves(&claimed.id, &[leaf("a", 3000)])
            .await
            .unwrap();

        let still_unclaimed = std::slice::from_ref(&unclaimed.id);
        assert_eq!(fixture.unclaimed().await, still_unclaimed);
        assert!(fixture.store.has_unclaimed_swaps().await.unwrap());
        assert!(fixture.detail(&unclaimed).await.inbound.is_empty());
    }

    #[tokio::test]
    async fn the_inbound_leaves_of_a_claimed_swap_are_listed_with_it() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;
        let inbound = [leaf("a", 1000), leaf("b", 2000)];

        fixture
            .store
            .record_inbound_leaves(&swap.id, &inbound)
            .await
            .unwrap();

        let detail = fixture.detail(&swap).await;
        assert_eq!(pairs(&detail.inbound), [("a", 1000), ("b", 2000)]);
        assert!(detail.outbound.is_empty());
        assert!(fixture.unclaimed().await.is_empty());
    }

    #[tokio::test]
    async fn a_swap_is_marked_claimed_when_the_claim_found_no_leaves() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;

        fixture
            .store
            .record_inbound_leaves(&swap.id, &[])
            .await
            .unwrap();

        assert!(fixture.unclaimed().await.is_empty());
        assert!(fixture.detail(&swap).await.inbound.is_empty());
    }

    #[tokio::test]
    async fn an_inbound_leaf_stored_again_is_kept_once_with_its_first_value() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;
        let claimed = [leaf("a", 1000), leaf("a", 1000), leaf("b", 2000)];
        fixture
            .store
            .record_inbound_leaves(&swap.id, &claimed)
            .await
            .unwrap();

        let claimed_again = [leaf("a", 7000), leaf("b", 2000)];
        fixture
            .store
            .record_inbound_leaves(&swap.id, &claimed_again)
            .await
            .unwrap();

        let detail = fixture.detail(&swap).await;
        assert_eq!(pairs(&detail.inbound), [("a", 1000), ("b", 2000)]);
        assert!(fixture.unclaimed().await.is_empty());
    }

    #[tokio::test]
    async fn inbound_leaves_are_not_stored_for_a_swap_the_store_does_not_have() {
        let fixture = Fixture::new().await;
        let stored = fixture.swap().await;
        let not_stored = new_swap();

        let claimed = fixture
            .store
            .record_inbound_leaves(&not_stored.id, &[leaf("a", 3000)])
            .await;

        assert!(claimed.is_err());
        assert_eq!(fixture.stored_leaves().await, 0);
        assert_eq!(fixture.unclaimed().await, [stored.id]);
    }

    #[tokio::test]
    async fn no_inbound_leaves_are_stored_when_the_swap_cannot_be_marked_claimed() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;
        fixture.add_check("brz_ssp_swaps", "NOT claimed").await;

        let claimed = fixture
            .store
            .record_inbound_leaves(&swap.id, &[leaf("a", 3000)])
            .await;

        assert!(claimed.is_err());
        assert_eq!(fixture.stored_leaves().await, 0);
        assert_eq!(fixture.unclaimed().await, [swap.id]);
    }

    #[tokio::test]
    async fn a_swap_is_not_marked_claimed_when_its_inbound_leaves_cannot_be_stored() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;
        let outbound_only = format!("direction = '{DIRECTION_OUTBOUND}'");
        fixture
            .add_check("brz_ssp_swap_leaves", &outbound_only)
            .await;

        let claimed = fixture
            .store
            .record_inbound_leaves(&swap.id, &[leaf("a", 3000)])
            .await;

        assert!(claimed.is_err());
        assert_eq!(fixture.unclaimed().await, [swap.id]);
        assert_eq!(fixture.stored_leaves().await, 0);
    }

    #[tokio::test]
    async fn the_outbound_leaves_of_a_swap_are_listed_with_it() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;
        let outbound = [leaf("a", 1000), leaf("b", 2000)];

        fixture
            .store
            .record_outbound_leaves(&swap.id, &outbound)
            .await
            .unwrap();

        let detail = fixture.detail(&swap).await;
        assert_eq!(pairs(&detail.outbound), [("a", 1000), ("b", 2000)]);
        assert!(detail.inbound.is_empty());
    }

    #[tokio::test]
    async fn a_swap_stays_unclaimed_when_its_outbound_leaves_are_stored() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;

        fixture
            .store
            .record_outbound_leaves(&swap.id, &[leaf("a", 3000)])
            .await
            .unwrap();

        assert_eq!(fixture.unclaimed().await, [swap.id]);
    }

    #[tokio::test]
    async fn nothing_is_stored_for_a_swap_without_outbound_leaves() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;

        fixture
            .store
            .record_outbound_leaves(&swap.id, &[])
            .await
            .unwrap();

        assert_eq!(fixture.stored_leaves().await, 0);
    }

    #[tokio::test]
    async fn an_outbound_leaf_stored_again_is_kept_once_with_its_first_value() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;
        let reserved = [leaf("a", 1000), leaf("a", 1000), leaf("b", 2000)];
        fixture
            .store
            .record_outbound_leaves(&swap.id, &reserved)
            .await
            .unwrap();

        let reserved_again = [leaf("a", 7000), leaf("b", 2000)];
        fixture
            .store
            .record_outbound_leaves(&swap.id, &reserved_again)
            .await
            .unwrap();

        let detail = fixture.detail(&swap).await;
        assert_eq!(pairs(&detail.outbound), [("a", 1000), ("b", 2000)]);
    }

    #[tokio::test]
    async fn outbound_leaves_are_not_stored_for_a_swap_the_store_does_not_have() {
        let fixture = Fixture::new().await;
        let not_stored = new_swap();

        let stored = fixture
            .store
            .record_outbound_leaves(&not_stored.id, &[leaf("a", 3000)])
            .await;

        assert!(stored.is_err());
        assert_eq!(fixture.stored_leaves().await, 0);
    }

    #[tokio::test]
    async fn a_leaf_is_stored_with_each_swap_it_is_part_of() {
        let fixture = Fixture::new().await;
        let (first, second, third) = (
            fixture.swap().await,
            fixture.swap().await,
            fixture.swap().await,
        );
        let store = &fixture.store;
        let same = [leaf("a", 3000)];

        // The SSP sends the leaf in one swap, claims it in the next and sends
        // it again in a third.
        store
            .record_outbound_leaves(&first.id, &same)
            .await
            .unwrap();
        store
            .record_inbound_leaves(&second.id, &same)
            .await
            .unwrap();
        store
            .record_outbound_leaves(&third.id, &same)
            .await
            .unwrap();

        let (first, second, third) = (
            fixture.detail(&first).await,
            fixture.detail(&second).await,
            fixture.detail(&third).await,
        );
        assert_eq!(pairs(&first.outbound), [("a", 3000)]);
        assert_eq!(pairs(&second.inbound), [("a", 3000)]);
        assert_eq!(pairs(&third.outbound), [("a", 3000)]);
        assert_eq!(fixture.stored_leaves().await, 3);
    }

    #[tokio::test]
    async fn each_listed_swap_has_its_own_outbound_and_inbound_leaves() {
        let fixture = Fixture::new().await;
        let (both, outbound_only, neither) = (
            fixture.swap().await,
            fixture.swap().await,
            fixture.swap().await,
        );
        let store = &fixture.store;
        let outbound = [leaf("a", 1000), leaf("b", 2000)];
        store
            .record_outbound_leaves(&both.id, &outbound)
            .await
            .unwrap();
        let inbound = [leaf("c", 3000)];
        store
            .record_inbound_leaves(&both.id, &inbound)
            .await
            .unwrap();
        let outbound = [leaf("d", 3000)];
        store
            .record_outbound_leaves(&outbound_only.id, &outbound)
            .await
            .unwrap();

        let listed = store.list_swaps(10).await.unwrap();

        let leaves_of = |swap: &SwapRecord| {
            let detail = listed.iter().find(|detail| detail.swap.id == swap.id);
            let detail = detail.unwrap();
            (pairs(&detail.outbound), pairs(&detail.inbound))
        };
        assert_eq!(
            leaves_of(&both),
            (vec![("a", 1000), ("b", 2000)], vec![("c", 3000)])
        );
        assert_eq!(leaves_of(&outbound_only), (vec![("d", 3000)], vec![]));
        assert_eq!(leaves_of(&neither), (vec![], vec![]));
    }

    #[tokio::test]
    async fn the_latest_swaps_are_listed_newest_first() {
        let fixture = Fixture::new().await;
        let middle = fixture.swap_created(20).await;
        let oldest = fixture.swap_created(30).await;
        let newest = fixture.swap_created(10).await;
        let newest_first = [newest.id, middle.id, oldest.id];

        assert!(fixture.listed(0).await.is_empty());
        assert_eq!(fixture.listed(2).await, &newest_first[..2]);
        assert_eq!(fixture.listed(3).await, newest_first);
        assert_eq!(fixture.listed(10).await, newest_first);
    }

    #[tokio::test]
    async fn the_leaves_of_a_listed_swap_are_in_the_order_of_their_ids() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;
        let outbound = [leaf("c", 1000), leaf("a", 2000), leaf("b", 3000)];
        fixture
            .store
            .record_outbound_leaves(&swap.id, &outbound)
            .await
            .unwrap();
        let inbound = [leaf("f", 4000), leaf("d", 5000), leaf("e", 6000)];
        fixture
            .store
            .record_inbound_leaves(&swap.id, &inbound)
            .await
            .unwrap();

        let detail = fixture.detail(&swap).await;

        assert_eq!(
            pairs(&detail.outbound),
            [("a", 2000), ("b", 3000), ("c", 1000)]
        );
        assert_eq!(
            pairs(&detail.inbound),
            [("d", 5000), ("e", 6000), ("f", 4000)]
        );
    }

    #[tokio::test]
    async fn the_counter_transfers_among_the_given_transfers_are_returned() {
        let fixture = Fixture::new().await;
        let swaps = [
            fixture.swap().await,
            fixture.swap().await,
            fixture.swap().await,
        ];
        let [first, second, third] = swaps.map(|swap| swap.counter_transfer_id.to_string());
        let not_stored = TransferId::generate().to_string();
        let among = async |transfer_ids: &[String]| {
            let found = fixture.store.counter_transfers_among(transfer_ids).await;
            found.unwrap()
        };

        assert!(among(&[]).await.is_empty());
        assert_eq!(
            among(std::slice::from_ref(&second)).await,
            HashSet::from([second])
        );
        assert_eq!(
            among(&[third.clone(), not_stored, first.clone()]).await,
            HashSet::from([first.clone(), third])
        );
        assert_eq!(
            among(&[first.clone(), first.clone()]).await,
            HashSet::from([first])
        );
    }

    #[tokio::test]
    async fn the_transfer_of_a_user_is_not_returned_as_a_counter_transfer() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;

        let found = fixture
            .store
            .counter_transfers_among(&[swap.user_transfer_id.to_string()])
            .await
            .unwrap();

        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn a_swap_is_deleted_with_its_leaves_and_no_others() {
        let fixture = Fixture::new().await;
        let (deleted, kept) = (fixture.swap().await, fixture.swap().await);
        let outbound = [leaf("a", 1000), leaf("b", 2000)];
        for swap in [&deleted, &kept] {
            fixture
                .store
                .record_outbound_leaves(&swap.id, &outbound)
                .await
                .unwrap();
        }

        fixture.store.delete_swap(&deleted.id).await.unwrap();

        let by_transfer = fixture
            .store
            .get_by_user_transfer_id(&deleted.user_transfer_id.to_string())
            .await
            .unwrap();
        assert!(by_transfer.is_none());
        assert_eq!(fixture.unclaimed().await, std::slice::from_ref(&kept.id));
        assert_eq!(fixture.listed(10).await, std::slice::from_ref(&kept.id));
        assert_eq!(fixture.stored_leaves().await, 2);
        let kept = fixture.detail(&kept).await;
        assert_eq!(pairs(&kept.outbound), [("a", 1000), ("b", 2000)]);
    }

    #[tokio::test]
    async fn nothing_is_deleted_for_a_swap_the_store_does_not_have() {
        let fixture = Fixture::new().await;
        let swap = fixture.swap().await;
        fixture
            .store
            .record_outbound_leaves(&swap.id, &[leaf("a", 3000)])
            .await
            .unwrap();

        fixture.store.delete_swap(&new_swap().id).await.unwrap();

        assert_eq!(fixture.listed(10).await, [swap.id]);
        assert_eq!(fixture.stored_leaves().await, 1);
    }

    #[tokio::test]
    async fn the_user_transfer_of_a_deleted_swap_is_free_for_a_new_swap() {
        let fixture = Fixture::new().await;
        let deleted = fixture.swap().await;
        fixture.store.delete_swap(&deleted.id).await.unwrap();
        let next = SwapRecord {
            user_transfer_id: deleted.user_transfer_id,
            ..new_swap()
        };

        fixture.store.insert_swap(&next).await.unwrap();

        let found = fixture
            .store
            .get_by_user_transfer_id(&next.user_transfer_id.to_string())
            .await
            .unwrap();
        assert_eq!(found.map(|found| found.id), Some(next.id));
    }
}
