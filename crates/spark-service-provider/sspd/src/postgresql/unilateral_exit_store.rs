use std::sync::Arc;

use bitcoin::{OutPoint, Transaction, Txid};
use sqlx::PgPool;

use crate::wallet::onchain::{forget_spends, record_spends};

#[derive(Debug, thiserror::Error)]
pub enum UnilateralExitStoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("invalid unilateral exit data: {0}")]
    Invalid(String),
}

fn invalid(message: impl Into<String>) -> UnilateralExitStoreError {
    UnilateralExitStoreError::Invalid(message.into())
}

#[derive(Debug, Clone)]
pub struct StoredCpfpChild {
    pub parent: Txid,
    pub child: Transaction,
}

impl StoredCpfpChild {
    pub fn wallet_inputs(&self) -> Vec<OutPoint> {
        wallet_inputs(&self.parent, &self.child)
    }
}

fn wallet_inputs(parent: &Txid, child: &Transaction) -> Vec<OutPoint> {
    child
        .input
        .iter()
        .map(|input| input.previous_output)
        .filter(|outpoint| outpoint.txid != *parent)
        .collect()
}

pub struct PostgresUnilateralExitStore {
    pool: Arc<PgPool>,
}

impl PostgresUnilateralExitStore {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    pub async fn cpfp_children(&self) -> Result<Vec<StoredCpfpChild>, UnilateralExitStoreError> {
        let rows: Vec<(String, Vec<u8>)> =
            sqlx::query_as("SELECT parent_tx_id, raw FROM brz_ssp_unilateral_exit_cpfp_children")
                .fetch_all(&*self.pool)
                .await?;
        rows.into_iter()
            .map(|(parent, raw)| {
                Ok(StoredCpfpChild {
                    parent: parent
                        .parse()
                        .map_err(|e| invalid(format!("transaction id {parent}: {e}")))?,
                    child: bitcoin::consensus::deserialize(&raw)
                        .map_err(|e| invalid(format!("stored CPFP child: {e}")))?,
                })
            })
            .collect()
    }

    /// In the same database transaction, the store has the wallet keep the
    /// CPFP child's wallet inputs out of coin selection.
    pub async fn set_cpfp_child(
        &self,
        parent: &Txid,
        child: &Transaction,
    ) -> Result<(), UnilateralExitStoreError> {
        let mut db = self.pool.begin().await?;
        let stored = sqlx::query(
            r"INSERT INTO brz_ssp_unilateral_exit_cpfp_children (parent_tx_id, tx_id, raw)
               VALUES ($1, $2, $3)
               ON CONFLICT (parent_tx_id) DO NOTHING",
        )
        .bind(parent.to_string())
        .bind(child.compute_txid().to_string())
        .bind(bitcoin::consensus::serialize(child))
        .execute(&mut *db)
        .await?
        .rows_affected();
        if stored == 0 {
            return Err(invalid(format!(
                "transaction {parent} has a CPFP child already"
            )));
        }
        let inputs = wallet_inputs(parent, child);
        record_spends(&mut db, &child.compute_txid(), &inputs).await?;
        db.commit().await?;
        Ok(())
    }

    /// In the same database transaction, the store has the wallet keep the
    /// wallet inputs of `child` out of coin selection instead of those of
    /// `earlier`.
    pub async fn replace_cpfp_child(
        &self,
        parent: &Txid,
        earlier: &Txid,
        child: &Transaction,
    ) -> Result<(), UnilateralExitStoreError> {
        let mut db = self.pool.begin().await?;
        let replaced = sqlx::query(
            r"UPDATE brz_ssp_unilateral_exit_cpfp_children SET tx_id = $3, raw = $4
               WHERE parent_tx_id = $1 AND tx_id = $2",
        )
        .bind(parent.to_string())
        .bind(earlier.to_string())
        .bind(child.compute_txid().to_string())
        .bind(bitcoin::consensus::serialize(child))
        .execute(&mut *db)
        .await?
        .rows_affected();
        if replaced == 0 {
            return Err(invalid(format!(
                "transaction {earlier} is not the CPFP child stored for {parent}"
            )));
        }
        forget_spends(&mut db, &[*earlier]).await?;
        let inputs = wallet_inputs(parent, child);
        record_spends(&mut db, &child.compute_txid(), &inputs).await?;
        db.commit().await?;
        Ok(())
    }

    /// The store also has the wallet stop keeping the wallet inputs of
    /// `children` out of coin selection. The inputs of a CPFP child that will
    /// not confirm stay unspent, and the chain record has the spends of a CPFP
    /// child that has confirmed.
    pub async fn delete_cpfp_children(
        &self,
        children: &[Txid],
    ) -> Result<(), UnilateralExitStoreError> {
        if children.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = children.iter().map(ToString::to_string).collect();
        let mut db = self.pool.begin().await?;
        sqlx::query("DELETE FROM brz_ssp_unilateral_exit_cpfp_children WHERE tx_id = ANY($1)")
            .bind(&ids)
            .execute(&mut *db)
            .await?;
        forget_spends(&mut db, children).await?;
        db.commit().await?;
        Ok(())
    }
}
