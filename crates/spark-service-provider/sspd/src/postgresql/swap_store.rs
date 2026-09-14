#![allow(clippy::needless_raw_string_hashes)]

use std::sync::Arc;

use sqlx::PgPool;

use crate::swap::SwapStore;
use crate::swap::repository::{SwapDetail, SwapLeaf, SwapRecord};

const DIRECTION_OUTBOUND: &str = "outbound";
const DIRECTION_INBOUND: &str = "inbound";

pub struct SwapRepository {
    pool: Arc<PgPool>,
}

impl SwapRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    async fn insert_leaf(
        db: &mut sqlx::PgConnection,
        swap_id: &str,
        direction: &str,
        leaf: &SwapLeaf,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO brz_ssp_swap_leaves (swap_id, direction, leaf_id, value_sats)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (swap_id, direction, leaf_id) DO NOTHING"#,
        )
        .bind(swap_id)
        .bind(direction)
        .bind(&leaf.leaf_id)
        .bind(leaf.value_sats)
        .execute(&mut *db)
        .await?;
        Ok(())
    }
}

type SwapRow = (String, Vec<u8>, String, String, String, i64, i64, i64);

const SWAP_COLUMNS: &str = "id, user_identity_public_key, user_transfer_id, counter_transfer_id, \
     reservation_id, total_amount_sats, target_amount_sats, fee_sats";

fn swap_record(row: SwapRow) -> SwapRecord {
    let (
        id,
        user_identity_public_key,
        user_transfer_id,
        counter_transfer_id,
        reservation_id,
        total_amount_sats,
        target_amount_sats,
        fee_sats,
    ) = row;
    SwapRecord {
        id,
        user_identity_public_key,
        user_transfer_id,
        counter_transfer_id,
        reservation_id,
        total_amount_sats,
        target_amount_sats,
        fee_sats,
    }
}

impl SwapRepository {
    async fn with_leaves(&self, swaps: Vec<SwapRecord>) -> Result<Vec<SwapDetail>, String> {
        let mut details = Vec::with_capacity(swaps.len());
        for swap in swaps {
            let leaves: Vec<(String, String, i64)> = sqlx::query_as(
                r#"SELECT direction, leaf_id, value_sats FROM brz_ssp_swap_leaves
                   WHERE swap_id = $1
                   ORDER BY direction, leaf_id"#,
            )
            .bind(&swap.id)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;

            let (mut outbound, mut inbound) = (Vec::new(), Vec::new());
            for (direction, leaf_id, value_sats) in leaves {
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
            details.push(SwapDetail {
                swap,
                outbound,
                inbound,
            });
        }
        Ok(details)
    }
}

#[async_trait::async_trait]
impl SwapStore for SwapRepository {
    async fn insert_swap(
        &self,
        swap: &SwapRecord,
        outbound_leaves: &[SwapLeaf],
    ) -> Result<(), String> {
        let mut db = self.pool.begin().await.map_err(|e| e.to_string())?;
        sqlx::query(
            r#"INSERT INTO brz_ssp_swaps
               (id, user_identity_public_key, user_transfer_id, counter_transfer_id,
                reservation_id, total_amount_sats, target_amount_sats, fee_sats)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(&swap.id)
        .bind(&swap.user_identity_public_key)
        .bind(&swap.user_transfer_id)
        .bind(&swap.counter_transfer_id)
        .bind(&swap.reservation_id)
        .bind(swap.total_amount_sats)
        .bind(swap.target_amount_sats)
        .bind(swap.fee_sats)
        .execute(&mut *db)
        .await
        .map_err(|e| e.to_string())?;
        for leaf in outbound_leaves {
            Self::insert_leaf(&mut db, &swap.id, DIRECTION_OUTBOUND, leaf)
                .await
                .map_err(|e| e.to_string())?;
        }
        db.commit().await.map_err(|e| e.to_string())
    }

    async fn get_by_user_transfer_id(
        &self,
        user_transfer_id: &str,
    ) -> Result<Option<SwapRecord>, String> {
        let row: Option<SwapRow> = sqlx::query_as(&format!(
            "SELECT {SWAP_COLUMNS} FROM brz_ssp_swaps WHERE user_transfer_id = $1"
        ))
        .bind(user_transfer_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(row.map(swap_record))
    }

    async fn delete_swap(&self, swap_id: &str) -> Result<(), String> {
        sqlx::query(r#"DELETE FROM brz_ssp_swaps WHERE id = $1"#)
            .bind(swap_id)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get_unclaimed_swaps(&self) -> Result<Vec<SwapDetail>, String> {
        let rows: Vec<SwapRow> = sqlx::query_as(&format!(
            "SELECT {SWAP_COLUMNS} FROM brz_ssp_swaps WHERE NOT claimed ORDER BY created_at"
        ))
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        self.with_leaves(rows.into_iter().map(swap_record).collect())
            .await
    }

    async fn list_swaps(&self, limit: u32) -> Result<Vec<SwapDetail>, String> {
        let rows: Vec<SwapRow> = sqlx::query_as(&format!(
            "SELECT {SWAP_COLUMNS} FROM brz_ssp_swaps ORDER BY created_at DESC LIMIT $1"
        ))
        .bind(i64::from(limit))
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        self.with_leaves(rows.into_iter().map(swap_record).collect())
            .await
    }

    async fn record_inbound_leaves(
        &self,
        swap_id: &str,
        inbound_leaves: &[SwapLeaf],
    ) -> Result<(), String> {
        let mut db = self.pool.begin().await.map_err(|e| e.to_string())?;
        for leaf in inbound_leaves {
            Self::insert_leaf(&mut db, swap_id, DIRECTION_INBOUND, leaf)
                .await
                .map_err(|e| e.to_string())?;
        }
        sqlx::query(r#"UPDATE brz_ssp_swaps SET claimed = TRUE WHERE id = $1"#)
            .bind(swap_id)
            .execute(&mut *db)
            .await
            .map_err(|e| e.to_string())?;
        db.commit().await.map_err(|e| e.to_string())
    }
}
