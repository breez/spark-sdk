use std::sync::Arc;

use sqlx::{PgPool, Row};

use crate::leaves::{IncomingLeaf, IncomingLeafStore};
use spark::tree::{TreeNode, TreeNodeId};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub struct PostgresIncomingLeafStore {
    pool: Arc<PgPool>,
}

impl PostgresIncomingLeafStore {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IncomingLeafStore for PostgresIncomingLeafStore {
    async fn hold(&self, leaves: &[TreeNode]) -> Result<(), BoxError> {
        let mut tx = self.pool.begin().await?;
        for leaf in leaves {
            // A retried claim must not reset the attempt count of a leaf already
            // held.
            sqlx::query(
                r"INSERT INTO brz_ssp_incoming_leaves (leaf_id, leaf)
                   VALUES ($1, $2)
                   ON CONFLICT (leaf_id) DO NOTHING",
            )
            .bind(leaf.id.to_string())
            .bind(serde_json::to_value(leaf)?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn held(&self, limit: i64) -> Result<Vec<IncomingLeaf>, BoxError> {
        let rows = sqlx::query(
            r"SELECT leaf, attempts
               FROM brz_ssp_incoming_leaves
               ORDER BY attempts, claimed_at
               LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&*self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(IncomingLeaf {
                    leaf: serde_json::from_value(row.try_get("leaf")?)?,
                    attempts: row.try_get("attempts")?,
                })
            })
            .collect()
    }

    async fn admitted(&self, leaf_ids: &[TreeNodeId]) -> Result<(), BoxError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = leaf_ids.iter().map(ToString::to_string).collect();
        sqlx::query("DELETE FROM brz_ssp_incoming_leaves WHERE leaf_id = ANY($1)")
            .bind(&ids)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    async fn failed(&self, leaf_ids: &[TreeNodeId]) -> Result<(), BoxError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = leaf_ids.iter().map(ToString::to_string).collect();
        sqlx::query(
            r"UPDATE brz_ssp_incoming_leaves
               SET attempts = attempts + 1
               WHERE leaf_id = ANY($1)",
        )
        .bind(&ids)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }
}
