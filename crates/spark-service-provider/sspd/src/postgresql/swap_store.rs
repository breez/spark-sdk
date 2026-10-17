#![allow(clippy::needless_raw_string_hashes)]

use std::collections::HashMap;
use std::sync::Arc;

use spark::services::TransferId;
use sqlx::PgPool;
use uuid::Uuid;

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
}

type SwapRow = (String, Vec<u8>, Uuid, Uuid, i64, i64, i64);

const SWAP_COLUMNS: &str = "id, user_identity_public_key, user_transfer_id, counter_transfer_id, \
     total_amount_sats, target_amount_sats, fee_sats";

fn swap_record(row: SwapRow) -> SwapRecord {
    let (
        id,
        user_identity_public_key,
        user_transfer_id,
        counter_transfer_id,
        total_amount_sats,
        target_amount_sats,
        fee_sats,
    ) = row;
    SwapRecord {
        id,
        user_identity_public_key,
        user_transfer_id: TransferId::from_bytes(user_transfer_id.into_bytes()),
        counter_transfer_id: TransferId::from_bytes(counter_transfer_id.into_bytes()),
        total_amount_sats,
        target_amount_sats,
        fee_sats,
    }
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
        let leaves: Vec<(String, String, String, i64)> = sqlx::query_as(
            r#"SELECT swap_id, direction, leaf_id, value_sats FROM brz_ssp_swap_leaves
               WHERE swap_id = ANY($1)
               ORDER BY swap_id, direction, leaf_id"#,
        )
        .bind(&ids)
        .fetch_all(self.pool.as_ref())
        .await
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
        sqlx::query(
            r#"INSERT INTO brz_ssp_swaps
               (id, user_identity_public_key, user_transfer_id, counter_transfer_id,
                total_amount_sats, target_amount_sats, fee_sats)
               VALUES ($1, $2, $3::uuid, $4::uuid, $5, $6, $7)"#,
        )
        .bind(&swap.id)
        .bind(&swap.user_identity_public_key)
        .bind(swap.user_transfer_id.to_string())
        .bind(swap.counter_transfer_id.to_string())
        .bind(swap.total_amount_sats)
        .bind(swap.target_amount_sats)
        .bind(swap.fee_sats)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get_by_user_transfer_id(
        &self,
        user_transfer_id: &str,
    ) -> Result<Option<SwapRecord>, String> {
        let row: Option<SwapRow> = sqlx::query_as(&format!(
            "SELECT {SWAP_COLUMNS} FROM brz_ssp_swaps WHERE user_transfer_id = $1::uuid"
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

    async fn record_outbound_leaves(
        &self,
        swap_id: &str,
        outbound_leaves: &[SwapLeaf],
    ) -> Result<(), String> {
        let (leaf_ids, values) = leaf_columns(outbound_leaves);
        sqlx::query(
            r#"INSERT INTO brz_ssp_swap_leaves (swap_id, direction, leaf_id, value_sats)
               SELECT $1, $2, leaf.leaf_id, leaf.value_sats
               FROM UNNEST($3::text[], $4::bigint[]) AS leaf (leaf_id, value_sats)
               ON CONFLICT (swap_id, direction, leaf_id) DO NOTHING"#,
        )
        .bind(swap_id)
        .bind(DIRECTION_OUTBOUND)
        .bind(&leaf_ids)
        .bind(&values)
        .execute(self.pool.as_ref())
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
        sqlx::query(
            r#"WITH stored AS (
                   INSERT INTO brz_ssp_swap_leaves (swap_id, direction, leaf_id, value_sats)
                   SELECT $1, $2, leaf.leaf_id, leaf.value_sats
                   FROM UNNEST($3::text[], $4::bigint[]) AS leaf (leaf_id, value_sats)
                   ON CONFLICT (swap_id, direction, leaf_id) DO NOTHING
               )
               UPDATE brz_ssp_swaps SET claimed = TRUE WHERE id = $1"#,
        )
        .bind(swap_id)
        .bind(DIRECTION_INBOUND)
        .bind(&leaf_ids)
        .bind(&values)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
