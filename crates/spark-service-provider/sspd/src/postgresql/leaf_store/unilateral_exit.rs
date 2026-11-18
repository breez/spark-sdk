use std::collections::{HashMap, HashSet};

use bitcoin::Transaction;
use bitcoin::{OutPoint, TxOut};
use spark::tree::TreeNodeId;
use tokio_postgres::Row;
use uuid::Uuid;

use super::rows::{invalid, node_id, transaction};
use super::{LeafStoreError, PostgresLeafStore};
use crate::chain::ConfirmedOutput;
use crate::postgresql::query;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatureTransaction {
    pub transaction: Transaction,
    pub timelock: u32,
    pub input_confirmation_height: u64,
    /// The lowest timelock above zero among the direct transactions with the
    /// same input. The operators' watchtower broadcasts a direct transaction
    /// with a timelock once it is mature.
    pub direct_timelock: Option<u32>,
    pub leaves: Vec<TreeNodeId>,
    pub value: u64,
    /// The transactions for `leaves` that have yet to confirm, `transaction`
    /// included.
    pub remaining: Vec<Transaction>,
}

#[derive(Debug, Clone)]
pub struct RefundOutput {
    pub node_id: TreeNodeId,
    pub outpoint: OutPoint,
    pub output: TxOut,
    pub height: u64,
}

fn arrays(outputs: &[ConfirmedOutput]) -> (Vec<String>, Vec<i32>, Vec<i64>) {
    let mut tx_ids = Vec::with_capacity(outputs.len());
    let mut indices = Vec::with_capacity(outputs.len());
    let mut heights = Vec::with_capacity(outputs.len());
    for output in outputs {
        tx_ids.push(output.outpoint.txid.to_string());
        indices.push(output.outpoint.vout.cast_signed());
        heights.push(output.height.cast_signed());
    }
    (tx_ids, indices, heights)
}

struct Below {
    tx_id: String,
    parent_tx_id: Option<String>,
    transaction: Transaction,
    /// The leaf this transaction is the refund of.
    leaf: Option<(Uuid, u64)>,
}

/// For each unspent output, the candidate is the stored transaction with the
/// lowest timelock whose input is that output, if that transaction is mature.
/// Postgres returns each candidate with the stored transactions below it and
/// the leaves they are the refunds of. `$1` to `$3` are the unspent outputs and
/// their heights, and `$4` is the tip's height. The refund of a leaf the SSP
/// transferred is never a candidate: that leaf is no longer the SSP's.
///
/// Below a candidate, Postgres goes down one transaction, far enough to reach
/// the refund of a leaf whose node transaction the candidate is. It goes all
/// the way down when the candidate or a transaction above it has a direct
/// transaction with a timelock, because every leaf below the candidate is then
/// exiting. The operators' watchtower does not broadcast a direct transaction
/// without a timelock.
const MATURE_ON_UNSPENT: &str = r"WITH RECURSIVE unspent (tx_id, output_index, height) AS (
           SELECT * FROM UNNEST($1::text[], $2::int[], $3::bigint[])
       ), lowest AS (
           SELECT DISTINCT ON (unspent.tx_id, unspent.output_index)
                  t.tx_id, t.previous_tx_id, t.previous_output_index, t.timelock,
                  unspent.height
           FROM unspent
           JOIN brz_ssp_spark_txs t
             ON t.previous_tx_id = unspent.tx_id
            AND t.previous_output_index = unspent.output_index
           WHERE NOT EXISTS (SELECT 1 FROM brz_ssp_spark_transferred_leaves transferred
                             WHERE transferred.refund_tx_id = t.tx_id)
           ORDER BY unspent.tx_id, unspent.output_index, t.timelock, t.tx_id
       ), candidate AS (
           SELECT * FROM lowest WHERE height + timelock <= $4::bigint + 1
       ), above AS (
           SELECT candidate.tx_id AS candidate, candidate.previous_tx_id,
                  candidate.previous_output_index
           FROM candidate
         UNION ALL
           SELECT above.candidate, t.previous_tx_id, t.previous_output_index
           FROM above
           JOIN brz_ssp_spark_txs t ON t.tx_id = above.previous_tx_id
       ), with_direct AS (
           SELECT DISTINCT above.candidate
           FROM above
           WHERE EXISTS (SELECT 1 FROM brz_ssp_spark_direct_txs direct
                         WHERE direct.previous_tx_id = above.previous_tx_id
                           AND direct.previous_output_index = above.previous_output_index
                           AND direct.timelock > 0)
       ), below AS (
           SELECT candidate.tx_id AS candidate, candidate.tx_id,
                  NULL::text AS parent_tx_id, 0 AS depth
           FROM candidate
         UNION ALL
           SELECT below.candidate, child.tx_id, below.tx_id, below.depth + 1
           FROM below
           JOIN brz_ssp_spark_txs child ON child.previous_tx_id = below.tx_id
           WHERE below.depth < 1
              OR EXISTS (SELECT 1 FROM with_direct
                         WHERE with_direct.candidate = below.candidate)
       )
       SELECT below.candidate, below.tx_id, below.parent_tx_id, t.raw,
              candidate.timelock, candidate.height,
              (SELECT MIN(direct.timelock)
               FROM brz_ssp_spark_direct_txs direct
               WHERE direct.previous_tx_id = candidate.previous_tx_id
                 AND direct.previous_output_index = candidate.previous_output_index
                 AND direct.timelock > 0) AS direct_timelock,
              l.node_id, l.value
       FROM below
       JOIN candidate ON candidate.tx_id = below.candidate
       JOIN brz_ssp_spark_txs t ON t.tx_id = below.tx_id
       LEFT JOIN brz_ssp_spark_leaves l ON l.refund_tx_id = below.tx_id
       ORDER BY below.candidate, below.depth, below.tx_id";

fn candidates(rows: &[Row]) -> Result<Vec<MatureTransaction>, LeafStoreError> {
    let mut candidates = Vec::new();
    let mut start = 0;
    while let Some(first) = rows.get(start) {
        let candidate: String = first.try_get("candidate")?;
        let mut below = Vec::new();
        let mut end = start;
        while let Some(row) = rows.get(end) {
            if row.try_get::<_, String>("candidate")? != candidate {
                break;
            }
            let leaf: Option<Uuid> = row.try_get("node_id")?;
            let value: Option<i64> = row.try_get("value")?;
            below.push(Below {
                tx_id: row.try_get("tx_id")?,
                parent_tx_id: row.try_get("parent_tx_id")?,
                transaction: transaction(row, "raw")?,
                leaf: leaf.zip(value.map(i64::cast_unsigned)),
            });
            end = end.saturating_add(1);
        }
        let top = below
            .first()
            .ok_or_else(|| invalid("a candidate without a transaction"))?;
        let mut transaction = MatureTransaction {
            transaction: top.transaction.clone(),
            timelock: first.try_get::<_, i32>("timelock")?.cast_unsigned(),
            input_confirmation_height: first.try_get::<_, i64>("height")?.cast_unsigned(),
            direct_timelock: first
                .try_get::<_, Option<i32>>("direct_timelock")?
                .map(i32::cast_unsigned),
            leaves: Vec::new(),
            value: 0,
            remaining: Vec::new(),
        };
        fill_leaves(&mut transaction, &below)?;
        candidates.push(transaction);
        start = end;
    }
    Ok(candidates)
}

impl PostgresLeafStore {
    /// The SSP needs no rule for when to stop broadcasting a transaction: once
    /// its input is spent, the chain record no longer has that output as
    /// unspent.
    pub async fn mature_transactions(
        &self,
        unspent: &[ConfirmedOutput],
        tip_height: u64,
    ) -> Result<Vec<MatureTransaction>, LeafStoreError> {
        if unspent.is_empty() {
            return Ok(Vec::new());
        }
        let (tx_ids, indices, heights) = arrays(unspent);
        let rows = query(
            &self.pool.get().await?,
            MATURE_ON_UNSPENT,
            &[&tx_ids, &indices, &heights, &tip_height.cast_signed()],
        )
        .await?;
        let (mut mature, without_leaves): (Vec<_>, Vec<_>) = candidates(&rows)?
            .into_iter()
            .partition(|candidate| !candidate.leaves.is_empty());

        // A candidate without leaves can be below an old node transaction that
        // has confirmed. The store returns it with each leaf of the SSP's whose
        // exit chain conflicts with that old node transaction.
        let ids: Vec<String> = without_leaves
            .iter()
            .map(|t| t.transaction.compute_txid().to_string())
            .collect();
        let mut conflicted = self.leaves_below_conflict(&ids).await?;
        for mut transaction in without_leaves {
            let id = transaction.transaction.compute_txid().to_string();
            let Some(leaves) = conflicted.remove(&id) else {
                continue;
            };
            for (leaf, value) in leaves {
                transaction.leaves.push(node_id(leaf)?);
                transaction.value = transaction.value.saturating_add(value);
            }
            transaction.remaining = vec![transaction.transaction.clone()];
            mature.push(transaction);
        }
        Ok(mature)
    }

    /// For each of `tx_ids`, the SSP's leaves whose exit chain conflicts with
    /// the parent of the transaction with that id.
    async fn leaves_below_conflict(
        &self,
        tx_ids: &[String],
    ) -> Result<HashMap<String, Vec<(Uuid, u64)>>, LeafStoreError> {
        if tx_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = query(
            &self.pool.get().await?,
            r"WITH RECURSIVE conflicting AS (
                   SELECT t.tx_id AS conflicted_for, other.tx_id
                   FROM brz_ssp_spark_txs t
                   JOIN brz_ssp_spark_txs above ON above.tx_id = t.previous_tx_id
                   JOIN brz_ssp_spark_txs other
                     ON other.previous_tx_id = above.previous_tx_id
                    AND other.previous_output_index = above.previous_output_index
                    AND other.tx_id <> above.tx_id
                   WHERE t.tx_id = ANY($1)
                 UNION
                   SELECT conflicting.conflicted_for, child.tx_id
                   FROM conflicting
                   JOIN brz_ssp_spark_txs child ON child.previous_tx_id = conflicting.tx_id
               )
               SELECT conflicting.conflicted_for, l.node_id, l.value
               FROM conflicting
               JOIN brz_ssp_spark_leaves l ON l.refund_tx_id = conflicting.tx_id
               ORDER BY conflicting.conflicted_for, l.node_id",
            &[&tx_ids],
        )
        .await?;
        let mut conflicted: HashMap<String, Vec<(Uuid, u64)>> = HashMap::new();
        for row in &rows {
            let value: i64 = row.try_get(2)?;
            conflicted
                .entry(row.try_get(0)?)
                .or_default()
                .push((row.try_get(1)?, value.cast_unsigned()));
        }
        Ok(conflicted)
    }

    /// The outputs among `unspent` that are the SSP's for a leaf it owns: the
    /// first output of the leaf's refund, of one of the leaf's direct refunds,
    /// or of an old refund that confirmed below an old node transaction of the
    /// leaf.
    pub async fn refund_outputs(
        &self,
        unspent: &[ConfirmedOutput],
    ) -> Result<Vec<RefundOutput>, LeafStoreError> {
        if unspent.is_empty() {
            return Ok(Vec::new());
        }
        let (tx_ids, indices, heights) = arrays(unspent);
        // In the last branch, an old refund is a stored cpfp transaction that
        // is neither the refund of one of the SSP's leaves nor the refund of a
        // leaf it transferred, and that is the parent of no stored transaction.
        let rows = query(
            &self.pool.get().await?,
            r"WITH unspent (tx_id, height) AS (
                   SELECT tx_id, height
                   FROM UNNEST($1::text[], $2::int[], $3::bigint[])
                        AS unspent (tx_id, output_index, height)
                   WHERE output_index = 0
               )
               SELECT l.node_id, refund.raw, unspent.height, NULL AS old_refund_tx_id
               FROM unspent
               JOIN brz_ssp_spark_leaves l ON l.refund_tx_id = unspent.tx_id
               JOIN brz_ssp_spark_txs refund ON refund.tx_id = unspent.tx_id
             UNION ALL
               SELECT l.node_id, direct.raw, unspent.height, NULL
               FROM unspent
               JOIN brz_ssp_spark_direct_txs direct ON direct.tx_id = unspent.tx_id
               JOIN brz_ssp_spark_leaves l ON l.refund_tx_id = direct.cpfp_tx_id
             UNION ALL
               SELECT NULL, old_refund.raw, unspent.height, old_refund.tx_id
               FROM unspent
               JOIN brz_ssp_spark_txs old_refund ON old_refund.tx_id = unspent.tx_id
               WHERE NOT EXISTS (SELECT 1 FROM brz_ssp_spark_leaves l
                                 WHERE l.refund_tx_id = old_refund.tx_id)
                 AND NOT EXISTS (SELECT 1 FROM brz_ssp_spark_transferred_leaves transferred
                                 WHERE transferred.refund_tx_id = old_refund.tx_id)
                 AND NOT EXISTS (SELECT 1 FROM brz_ssp_spark_txs child
                                 WHERE child.previous_tx_id = old_refund.tx_id)",
            &[&tx_ids, &indices, &heights],
        )
        .await?;

        let old_refunds: Vec<String> = rows
            .iter()
            .filter_map(|row| {
                row.try_get::<_, Option<String>>("old_refund_tx_id")
                    .transpose()
            })
            .collect::<Result<_, _>>()?;
        let conflicted = self.leaves_below_conflict(&old_refunds).await?;
        let mut outputs = Vec::new();
        for row in &rows {
            let owned: Option<Uuid> = row.try_get("node_id")?;
            let old_refund: Option<String> = row.try_get("old_refund_tx_id")?;
            let leaf = match old_refund {
                None => owned,
                Some(id) => conflicted
                    .get(&id)
                    .and_then(|leaves| leaves.first())
                    .map(|(leaf, _)| *leaf),
            };
            let Some(leaf) = leaf else {
                continue;
            };
            let refund = transaction(row, "raw")?;
            let output = refund
                .output
                .first()
                .ok_or_else(|| invalid("a stored refund has no output"))?
                .clone();
            outputs.push(RefundOutput {
                node_id: node_id(leaf)?,
                outpoint: OutPoint::new(refund.compute_txid(), 0),
                output,
                height: row.try_get::<_, i64>("height")?.cast_unsigned(),
            });
        }
        outputs.sort_by_key(|output| (output.height, output.outpoint));
        Ok(outputs)
    }
}

fn fill_leaves(mature: &mut MatureTransaction, below: &[Below]) -> Result<(), LeafStoreError> {
    let parents: HashMap<&str, Option<&str>> = below
        .iter()
        .map(|t| (t.tx_id.as_str(), t.parent_tx_id.as_deref()))
        .collect();
    let mut on_exit_chains: HashSet<&str> = HashSet::new();
    for transaction in below {
        let Some((leaf, value)) = transaction.leaf else {
            continue;
        };
        mature.leaves.push(node_id(leaf)?);
        mature.value = mature.value.saturating_add(value);
        let mut step = Some(transaction.tx_id.as_str());
        while let Some(id) = step {
            if !on_exit_chains.insert(id) {
                break;
            }
            step = parents.get(id).copied().flatten();
        }
    }
    mature.remaining = below
        .iter()
        .filter(|t| on_exit_chains.contains(t.tx_id.as_str()))
        .map(|t| t.transaction.clone())
        .collect();
    Ok(())
}
