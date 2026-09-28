#![allow(clippy::needless_raw_string_hashes)]

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use bitcoin::{OutPoint, Txid};
use spark::tree::{TreeNode, TreeNodeId};
use sqlx::PgPool;
use sqlx::types::Json;

use crate::leaves::LeafSigningKeys;
use crate::pool::repository::{DepositTree, DepositTx, FundingBump, TreeNodes};
use crate::wallet::onchain::record_spends;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(sqlx::FromRow)]
struct DepositTreeRow {
    txid: String,
    raw_tx: Vec<u8>,
    fee_sats: i64,
    stored_height: i64,
    bump_tx: Option<Vec<u8>>,
    bump_fee_sats: Option<i64>,
    bump_height: Option<i64>,
    vout: Option<i32>,
    deposit_address: Option<String>,
    denomination: Option<i64>,
    leaf_count: Option<i32>,
}

pub struct PoolRepository {
    pool: Arc<PgPool>,
}

impl PoolRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Stores a funding transaction, its trees, the wallet coins it spends and the id
    /// each leaf's signing key derives from, in one database transaction.
    pub async fn insert_deposit(
        &self,
        deposit: &DepositTx,
        nodes: &[TreeNodes],
        signing_leaf_ids: &[(TreeNodeId, TreeNodeId)],
    ) -> Result<(), sqlx::Error> {
        let txid = deposit.tx.compute_txid();
        let mut db = self.pool.begin().await?;
        sqlx::query(
            r#"INSERT INTO brz_ssp_deposit_txs (txid, raw_tx, fee_sats, stored_height)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(txid.to_string())
        .bind(bitcoin::consensus::serialize(&deposit.tx))
        .bind(deposit.fee_sats.cast_signed())
        .bind(deposit.stored_height.cast_signed())
        .execute(&mut *db)
        .await?;
        for (tree, nodes) in deposit.trees.iter().zip(nodes) {
            sqlx::query(
                r#"INSERT INTO brz_ssp_deposit_trees
                   (txid, vout, deposit_address, denomination, leaf_count, leaves, branches)
                   VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            )
            .bind(txid.to_string())
            .bind(tree.outpoint.vout.cast_signed())
            .bind(&tree.deposit_address)
            .bind(tree.denomination.cast_signed())
            .bind(tree.leaf_count.cast_signed())
            .bind(Json(&nodes.leaves))
            .bind(Json(&nodes.branches))
            .execute(&mut *db)
            .await?;
        }
        let (node_ids, deposit_leaf_ids): (Vec<String>, Vec<String>) = signing_leaf_ids
            .iter()
            .map(|(node_id, leaf_id)| (node_id.to_string(), leaf_id.to_string()))
            .unzip();
        sqlx::query(
            r#"INSERT INTO brz_ssp_leaf_id_map (node_id, deposit_leaf_id)
               SELECT * FROM UNNEST($1::text[], $2::text[])"#,
        )
        .bind(&node_ids)
        .bind(&deposit_leaf_ids)
        .execute(&mut *db)
        .await?;
        let inputs: Vec<OutPoint> = deposit.tx.input.iter().map(|i| i.previous_output).collect();
        record_spends(&mut db, &txid, &inputs).await?;
        db.commit().await
    }

    /// Every funding transaction with a tree not yet in the pool or fee bump children
    /// not yet settled, carrying only those trees and its latest child.
    pub async fn open_deposits(&self) -> Result<Vec<DepositTx>, BoxError> {
        let rows: Vec<DepositTreeRow> = sqlx::query_as(
            r#"SELECT t.txid, t.raw_tx, t.fee_sats, t.stored_height,
                      b.raw_tx AS bump_tx, b.fee_sats AS bump_fee_sats, b.height AS bump_height,
                      d.vout, d.deposit_address, d.denomination, d.leaf_count
                   FROM brz_ssp_deposit_txs t
                   LEFT JOIN brz_ssp_deposit_trees d ON d.txid = t.txid AND NOT d.pooled
                   LEFT JOIN LATERAL (
                       SELECT raw_tx, fee_sats, height
                       FROM brz_ssp_funding_bumps
                       WHERE funding_txid = t.txid
                       ORDER BY created_at DESC
                       LIMIT 1
                   ) b ON TRUE
                   WHERE d.txid IS NOT NULL OR t.bump_pending
                   ORDER BY t.created_at, d.vout"#,
        )
        .fetch_all(self.pool.as_ref())
        .await?;

        let mut deposits: Vec<DepositTx> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for row in rows {
            let position = if let Some(position) = index.get(&row.txid) {
                *position
            } else {
                let position = deposits.len();
                let latest_bump = match (row.bump_tx, row.bump_fee_sats, row.bump_height) {
                    (Some(tx), Some(fee_sats), Some(height)) => Some(FundingBump {
                        tx: bitcoin::consensus::deserialize(&tx)?,
                        fee_sats: fee_sats.cast_unsigned(),
                        height: height.cast_unsigned(),
                    }),
                    _ => None,
                };
                index.insert(row.txid.clone(), position);
                deposits.push(DepositTx {
                    tx: bitcoin::consensus::deserialize(&row.raw_tx)?,
                    fee_sats: row.fee_sats.cast_unsigned(),
                    stored_height: row.stored_height.cast_unsigned(),
                    latest_bump,
                    trees: Vec::new(),
                });
                position
            };
            if let (Some(vout), Some(deposit_address), Some(denomination), Some(leaf_count)) = (
                row.vout,
                row.deposit_address,
                row.denomination,
                row.leaf_count,
            ) && let Some(deposit) = deposits.get_mut(position)
            {
                deposit.trees.push(DepositTree {
                    outpoint: OutPoint {
                        txid: Txid::from_str(&row.txid)?,
                        vout: vout.cast_unsigned(),
                    },
                    deposit_address,
                    denomination: denomination.cast_unsigned(),
                    leaf_count: leaf_count.cast_unsigned(),
                });
            }
        }
        Ok(deposits)
    }

    /// Stores a fee bump child of funding transaction `funding_txid` with the wallet
    /// coins it spends. Earlier children keep theirs: any of them can still confirm.
    pub async fn add_bump(
        &self,
        funding_txid: &Txid,
        bump: &FundingBump,
    ) -> Result<(), sqlx::Error> {
        let txid = bump.tx.compute_txid();
        let mut db = self.pool.begin().await?;
        sqlx::query(
            r#"INSERT INTO brz_ssp_funding_bumps (txid, funding_txid, raw_tx, fee_sats, height)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(txid.to_string())
        .bind(funding_txid.to_string())
        .bind(bitcoin::consensus::serialize(&bump.tx))
        .bind(bump.fee_sats.cast_signed())
        .bind(bump.height.cast_signed())
        .execute(&mut *db)
        .await?;
        sqlx::query(r#"UPDATE brz_ssp_deposit_txs SET bump_pending = TRUE WHERE txid = $1"#)
            .bind(funding_txid.to_string())
            .execute(&mut *db)
            .await?;
        let inputs: Vec<OutPoint> = bump.tx.input.iter().map(|i| i.previous_output).collect();
        record_spends(&mut db, &txid, &inputs).await?;
        db.commit().await
    }

    /// Drops every fee bump child of `funding_txid` but `confirmed`, and frees the
    /// wallet coins only they spend.
    pub async fn settle_bumps(
        &self,
        funding_txid: &Txid,
        confirmed: &Txid,
    ) -> Result<(), sqlx::Error> {
        let mut db = self.pool.begin().await?;
        let lost: Vec<String> = sqlx::query_scalar(
            r#"DELETE FROM brz_ssp_funding_bumps
               WHERE funding_txid = $1 AND txid <> $2
               RETURNING txid"#,
        )
        .bind(funding_txid.to_string())
        .bind(confirmed.to_string())
        .fetch_all(&mut *db)
        .await?;
        sqlx::query(r#"DELETE FROM brz_ssp_wallet_spends WHERE spending_txid = ANY($1)"#)
            .bind(&lost)
            .execute(&mut *db)
            .await?;
        sqlx::query(r#"UPDATE brz_ssp_deposit_txs SET bump_pending = FALSE WHERE txid = $1"#)
            .bind(funding_txid.to_string())
            .execute(&mut *db)
            .await?;
        db.commit().await
    }

    /// Deletes a funding transaction, its trees and its fee bump children, and frees
    /// the wallet coins they spend.
    pub async fn retire_deposit(&self, deposit: &DepositTx) -> Result<(), sqlx::Error> {
        let txid = deposit.tx.compute_txid().to_string();
        let mut db = self.pool.begin().await?;
        sqlx::query(
            r#"DELETE FROM brz_ssp_wallet_spends
               WHERE spending_txid = $1
               OR spending_txid IN (SELECT txid FROM brz_ssp_funding_bumps WHERE funding_txid = $1)"#,
        )
        .bind(&txid)
        .execute(&mut *db)
        .await?;
        sqlx::query(r#"DELETE FROM brz_ssp_deposit_trees WHERE txid = $1"#)
            .bind(&txid)
            .execute(&mut *db)
            .await?;
        sqlx::query(r#"DELETE FROM brz_ssp_deposit_txs WHERE txid = $1"#)
            .bind(&txid)
            .execute(&mut *db)
            .await?;
        db.commit().await
    }

    pub async fn tree_nodes(&self, outpoint: &OutPoint) -> Result<TreeNodes, sqlx::Error> {
        let (leaves, branches): (Json<Vec<TreeNode>>, Json<Vec<TreeNode>>) = sqlx::query_as(
            r#"SELECT leaves, branches FROM brz_ssp_deposit_trees WHERE txid = $1 AND vout = $2"#,
        )
        .bind(outpoint.txid.to_string())
        .bind(outpoint.vout.cast_signed())
        .fetch_one(self.pool.as_ref())
        .await?;
        Ok(TreeNodes {
            leaves: leaves.0,
            branches: branches.0,
        })
    }

    pub async fn mark_pooled(&self, outpoint: &OutPoint) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE brz_ssp_deposit_trees SET pooled = TRUE WHERE txid = $1 AND vout = $2"#,
        )
        .bind(outpoint.txid.to_string())
        .bind(outpoint.vout.cast_signed())
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl LeafSigningKeys for PoolRepository {
    async fn get_signing_leaf_id(&self, node_id: &str) -> Result<Option<String>, String> {
        let row: Option<(String,)> =
            sqlx::query_as(r#"SELECT deposit_leaf_id FROM brz_ssp_leaf_id_map WHERE node_id = $1"#)
                .bind(node_id)
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e| e.to_string())?;
        Ok(row.map(|(id,)| id))
    }

    async fn mark_signing_under_node_id(&self, node_ids: &[String]) -> Result<(), String> {
        sqlx::query(r#"DELETE FROM brz_ssp_leaf_id_map WHERE node_id = ANY($1)"#)
            .bind(node_ids)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
