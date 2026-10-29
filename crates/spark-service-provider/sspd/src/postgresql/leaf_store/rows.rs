use std::collections::HashSet;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use bitcoin::secp256k1::PublicKey;
use bitcoin::{OutPoint, Transaction, Txid};
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use frost_secp256k1_tr::Identifier;
use spark::services::csv_timelock;
use spark::tree::{SigningKeyshare, TreeNode, TreeNodeId, TreeNodeStatus};
use tokio_postgres::Row;
use uuid::Uuid;

use super::{LeafStoreError, OPERATORS_AGREE, UNRESERVED};
use crate::chain::WatchedOutput;
use crate::postgresql::{ChainRepository, execute, query, query_one};

const IDENTIFIER_LEN: usize = 32;

pub(super) fn invalid(message: impl Into<String>) -> LeafStoreError {
    LeafStoreError::Invalid(message.into())
}

pub(super) fn txid(id: &str) -> Result<Txid, LeafStoreError> {
    id.parse()
        .map_err(|e| invalid(format!("transaction id {id}: {e}")))
}

pub(super) fn outpoint(tx_id: &str, output_index: i32) -> Result<OutPoint, LeafStoreError> {
    Ok(OutPoint::new(txid(tx_id)?, output_index.cast_unsigned()))
}

pub(super) fn node_uuid(id: &TreeNodeId) -> Result<Uuid, LeafStoreError> {
    Uuid::parse_str(&id.to_string()).map_err(|e| invalid(format!("node id {id}: {e}")))
}

pub(super) fn node_id(id: Uuid) -> Result<TreeNodeId, LeafStoreError> {
    id.to_string()
        .parse()
        .map_err(|e| invalid(format!("node id {id}: {e}")))
}

fn status_name(status: TreeNodeStatus) -> String {
    status.to_string()
}

fn status_from_name(name: &str) -> TreeNodeStatus {
    serde_json::from_value(serde_json::Value::String(name.to_string()))
        .unwrap_or(TreeNodeStatus::Unknown)
}

struct StoredTransaction {
    spends: OutPoint,
    timelock: i32,
    raw: Vec<u8>,
}

impl StoredTransaction {
    fn new(transaction: &Transaction) -> Result<(String, Self), LeafStoreError> {
        let txid = transaction.compute_txid();
        let input = transaction
            .input
            .first()
            .ok_or_else(|| invalid(format!("transaction {txid} has no input")))?;
        Ok((
            txid.to_string(),
            Self {
                spends: input.previous_output,
                timelock: csv_timelock(transaction).unwrap_or(0).cast_signed(),
                raw: bitcoin::consensus::serialize(transaction),
            },
        ))
    }

    fn signed(transaction: &Transaction) -> Result<(String, Self), LeafStoreError> {
        let (id, stored) = Self::new(transaction)?;
        if transaction
            .input
            .iter()
            .any(|input| input.witness.is_empty())
        {
            return Err(invalid(format!(
                "transaction {} is unsigned",
                transaction.compute_txid()
            )));
        }
        Ok((id, stored))
    }
}

#[derive(PartialEq)]
struct NodeRow {
    tree_id: Uuid,
    parent_node_id: Option<Uuid>,
    node_tx_id: String,
    verifying_key: Vec<u8>,
    keyshare_key: Vec<u8>,
    keyshare_threshold: i32,
    keyshare_owners: Vec<u8>,
}

struct LeafRow {
    node_id: TreeNodeId,
    refund_tx_id: String,
    value: i64,
    refund_timelock: i32,
}

#[derive(Default)]
pub(super) struct Rows {
    transactions: BTreeMap<String, StoredTransaction>,
    direct: BTreeMap<String, (String, StoredTransaction)>,
    nodes: BTreeMap<Uuid, NodeRow>,
    leaves: BTreeMap<Uuid, LeafRow>,
    refunds: BTreeSet<String>,
}

impl Rows {
    fn signed(&mut self, transaction: &Transaction) -> Result<String, LeafStoreError> {
        let (id, stored) = StoredTransaction::signed(transaction)?;
        self.transactions.insert(id.clone(), stored);
        Ok(id)
    }

    fn direct(
        &mut self,
        transaction: Option<&Transaction>,
        cpfp: &str,
    ) -> Result<Option<String>, LeafStoreError> {
        let Some(transaction) = transaction else {
            return Ok(None);
        };
        let (id, stored) = StoredTransaction::new(transaction)?;
        self.direct.insert(id.clone(), (cpfp.to_string(), stored));
        Ok(Some(id))
    }

    pub(super) fn node(&mut self, node: &TreeNode) -> Result<String, LeafStoreError> {
        let node_tx_id = self.signed(&node.node_tx)?;
        self.direct(node.direct_tx.as_ref(), &node_tx_id)?;
        let tree_id = Uuid::parse_str(&node.tree_id)
            .map_err(|e| invalid(format!("tree id {}: {e}", node.tree_id)))?;
        let keyshare_owners = node
            .signing_keyshare
            .owner_identifiers
            .iter()
            .flat_map(Identifier::serialize)
            .collect();
        self.nodes.insert(
            node_uuid(&node.id)?,
            NodeRow {
                tree_id,
                parent_node_id: node.parent_node_id.as_ref().map(node_uuid).transpose()?,
                node_tx_id: node_tx_id.clone(),
                verifying_key: node.verifying_public_key.serialize().to_vec(),
                keyshare_key: node.signing_keyshare.public_key.serialize().to_vec(),
                keyshare_threshold: node.signing_keyshare.threshold.cast_signed(),
                keyshare_owners,
            },
        );
        Ok(node_tx_id)
    }

    pub(super) fn leaf(&mut self, leaf: &TreeNode) -> Result<(), LeafStoreError> {
        let node_tx_id = self.node(leaf)?;
        let refund = leaf
            .refund_tx
            .as_ref()
            .ok_or_else(|| invalid(format!("leaf {} has no refund", leaf.id)))?;
        let (refund_tx_id, stored) = StoredTransaction::signed(refund)?;
        let value = (stored.spends.txid.to_string() == node_tx_id)
            .then(|| leaf.node_tx.output.get(stored.spends.vout as usize))
            .flatten()
            .ok_or_else(|| {
                invalid(format!(
                    "the input of the refund of leaf {} is not an output of its node transaction",
                    leaf.id
                ))
            })?
            .value
            .to_sat()
            .cast_signed();
        let refund_timelock = stored.timelock;
        self.transactions.insert(refund_tx_id.clone(), stored);
        self.refunds.insert(refund_tx_id.clone());
        for direct in [&leaf.direct_refund_tx, &leaf.direct_from_cpfp_refund_tx] {
            let direct_refund = self.direct(direct.as_ref(), &refund_tx_id)?;
            self.refunds.extend(direct_refund);
        }
        self.leaves.insert(
            node_uuid(&leaf.id)?,
            LeafRow {
                node_id: leaf.id.clone(),
                refund_tx_id,
                value,
                refund_timelock,
            },
        );
        Ok(())
    }

    pub(super) async fn write(
        self,
        db: &impl GenericClient,
        chain: &ChainRepository,
    ) -> Result<(), LeafStoreError> {
        insert(db, &self).await?;
        check_leaves(db, &self.leaves).await?;
        let watched = self.watched(db).await?;
        chain.watch_outputs(db, &watched).await?;
        Ok(())
    }

    async fn watched(&self, db: &impl GenericClient) -> Result<Vec<WatchedOutput>, LeafStoreError> {
        let ids: Vec<&str> = self
            .transactions
            .keys()
            .chain(self.direct.keys())
            .map(String::as_str)
            .collect();
        let sql = format!(
            r"SELECT t.previous_tx_id, t.previous_output_index,
                      above.previous_tx_id, above.previous_output_index
               FROM {STORED} t
               LEFT JOIN {STORED} above ON above.tx_id = t.previous_tx_id
               WHERE t.tx_id = ANY($1)"
        );
        let rows = query(db, &sql, &[&ids]).await?;
        let mut watched = Vec::with_capacity(rows.len().saturating_add(self.refunds.len()));
        for row in &rows {
            let input_tx_id: Option<&str> = row.try_get(2)?;
            let input_output_index: Option<i32> = row.try_get(3)?;
            watched.push(WatchedOutput {
                outpoint: outpoint(row.try_get(0)?, row.try_get(1)?)?,
                input: match (input_tx_id, input_output_index) {
                    (Some(tx_id), Some(output_index)) => Some(outpoint(tx_id, output_index)?),
                    _ => None,
                },
            });
        }
        // The first output of a refund is for the leaf's owner.
        let direct = self.direct.iter().map(|(id, (_, stored))| (id, stored));
        for (id, stored) in self.transactions.iter().chain(direct) {
            if self.refunds.contains(id) {
                watched.push(WatchedOutput {
                    outpoint: OutPoint::new(txid(id)?, 0),
                    input: Some(stored.spends),
                });
            }
        }
        Ok(watched)
    }
}

pub(super) const STORED: &str = r"(SELECT tx_id, previous_tx_id, previous_output_index
                                   FROM brz_ssp_spark_txs
                                 UNION ALL
                                   SELECT tx_id, previous_tx_id, previous_output_index
                                   FROM brz_ssp_spark_direct_txs)";

const INSERT_ROWS: &str = r"WITH transactions AS (
           INSERT INTO brz_ssp_spark_txs
               (tx_id, previous_tx_id, previous_output_index, timelock, raw)
           SELECT * FROM UNNEST($1::text[], $2::text[], $3::int[], $4::int[], $5::bytea[])
           ON CONFLICT (tx_id) DO NOTHING
       ), direct AS (
           INSERT INTO brz_ssp_spark_direct_txs
               (tx_id, cpfp_tx_id, previous_tx_id, previous_output_index, timelock, raw)
           SELECT * FROM UNNEST($6::text[], $7::text[], $8::text[], $9::int[], $10::int[],
                                $11::bytea[])
           ON CONFLICT (tx_id) DO NOTHING
       ), nodes AS (
           INSERT INTO brz_ssp_spark_nodes
               (node_id, tree_id, parent_node_id, node_tx_id, verifying_key, keyshare_key,
                keyshare_threshold, keyshare_owners)
           SELECT reported.*
           FROM UNNEST($12::uuid[], $13::uuid[], $14::uuid[], $15::text[], $16::bytea[],
                       $17::bytea[], $18::int[], $19::bytea[])
                AS reported (node_id, tree_id, parent_node_id, node_tx_id, verifying_key,
                             keyshare_key, keyshare_threshold, keyshare_owners)
           LEFT JOIN brz_ssp_spark_nodes stored ON stored.node_id = reported.node_id
           WHERE stored.node_id IS NULL
              OR (stored.tree_id, stored.parent_node_id, stored.node_tx_id,
                  stored.verifying_key, stored.keyshare_key, stored.keyshare_threshold,
                  stored.keyshare_owners)
                 IS DISTINCT FROM
                 (reported.tree_id, reported.parent_node_id, reported.node_tx_id,
                  reported.verifying_key, reported.keyshare_key, reported.keyshare_threshold,
                  reported.keyshare_owners)
           ORDER BY reported.node_id
           ON CONFLICT (node_id) DO UPDATE SET
               tree_id = EXCLUDED.tree_id,
               parent_node_id = EXCLUDED.parent_node_id,
               node_tx_id = EXCLUDED.node_tx_id,
               verifying_key = EXCLUDED.verifying_key,
               keyshare_key = EXCLUDED.keyshare_key,
               keyshare_threshold = EXCLUDED.keyshare_threshold,
               keyshare_owners = EXCLUDED.keyshare_owners
       )
       INSERT INTO brz_ssp_spark_leaves (refund_tx_id, node_id, value, refund_timelock)
       SELECT * FROM UNNEST($20::text[], $21::uuid[], $22::bigint[], $23::int[])
       ON CONFLICT DO NOTHING";

/// Postgres checks foreign keys at the end of a statement, so the store can
/// insert a leaf in the same statement as its node and its transactions.
///
/// It leaves out each node that the store already has as the operators report
/// it. For ON CONFLICT DO UPDATE, Postgres locks each conflicting row, even
/// when every column stays the same.
async fn insert(db: &impl GenericClient, write: &Rows) -> Result<(), LeafStoreError> {
    let transactions = &write.transactions;
    let ids: Vec<&str> = transactions.keys().map(String::as_str).collect();
    let previous_ids: Vec<String> = transactions
        .values()
        .map(|t| t.spends.txid.to_string())
        .collect();
    let previous_indices: Vec<i32> = transactions
        .values()
        .map(|t| t.spends.vout.cast_signed())
        .collect();
    let timelocks: Vec<i32> = transactions.values().map(|t| t.timelock).collect();
    let raws: Vec<&[u8]> = transactions.values().map(|t| t.raw.as_slice()).collect();

    let direct = &write.direct;
    let direct_ids: Vec<&str> = direct.keys().map(String::as_str).collect();
    let cpfp_ids: Vec<&str> = direct.values().map(|(c, _)| c.as_str()).collect();
    let direct_previous_ids: Vec<String> = direct
        .values()
        .map(|(_, t)| t.spends.txid.to_string())
        .collect();
    let direct_previous_indices: Vec<i32> = direct
        .values()
        .map(|(_, t)| t.spends.vout.cast_signed())
        .collect();
    let direct_timelocks: Vec<i32> = direct.values().map(|(_, t)| t.timelock).collect();
    let direct_raws: Vec<&[u8]> = direct.values().map(|(_, t)| t.raw.as_slice()).collect();

    let nodes = &write.nodes;
    let node_ids: Vec<Uuid> = nodes.keys().copied().collect();
    let tree_ids: Vec<Uuid> = nodes.values().map(|n| n.tree_id).collect();
    let parents: Vec<Option<Uuid>> = nodes.values().map(|n| n.parent_node_id).collect();
    let node_transactions: Vec<&str> = nodes.values().map(|n| n.node_tx_id.as_str()).collect();
    let verifying: Vec<&[u8]> = nodes.values().map(|n| n.verifying_key.as_slice()).collect();
    let keyshares: Vec<&[u8]> = nodes.values().map(|n| n.keyshare_key.as_slice()).collect();
    let thresholds: Vec<i32> = nodes.values().map(|n| n.keyshare_threshold).collect();
    let owners: Vec<&[u8]> = nodes
        .values()
        .map(|n| n.keyshare_owners.as_slice())
        .collect();

    let leaves = &write.leaves;
    let leaf_ids: Vec<Uuid> = leaves.keys().copied().collect();
    let refunds: Vec<&str> = leaves.values().map(|l| l.refund_tx_id.as_str()).collect();
    let values: Vec<i64> = leaves.values().map(|l| l.value).collect();
    let refund_timelocks: Vec<i32> = leaves.values().map(|l| l.refund_timelock).collect();

    execute(
        db,
        INSERT_ROWS,
        &[
            &ids,
            &previous_ids,
            &previous_indices,
            &timelocks,
            &raws,
            &direct_ids,
            &cpfp_ids,
            &direct_previous_ids,
            &direct_previous_indices,
            &direct_timelocks,
            &direct_raws,
            &node_ids,
            &tree_ids,
            &parents,
            &node_transactions,
            &verifying,
            &keyshares,
            &thresholds,
            &owners,
            &refunds,
            &leaf_ids,
            &values,
            &refund_timelocks,
        ],
    )
    .await?;
    Ok(())
}

/// The store calls this after it inserts the leaves. Postgres makes that insert
/// wait for any caller that is marking one of the leaves transferred, so that
/// caller has committed before this function reads the transferred leaves.
async fn check_leaves(
    db: &impl GenericClient,
    leaves: &BTreeMap<Uuid, LeafRow>,
) -> Result<(), LeafStoreError> {
    if leaves.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = leaves.keys().copied().collect();
    let refunds: Vec<&str> = leaves.values().map(|l| l.refund_tx_id.as_str()).collect();
    let stored = query(
        db,
        r"SELECT node_id, refund_tx_id, FALSE FROM brz_ssp_spark_leaves WHERE node_id = ANY($1)
         UNION ALL
           SELECT node_id, refund_tx_id, TRUE FROM brz_ssp_spark_transferred_leaves
           WHERE refund_tx_id = ANY($2)
           ORDER BY 3",
        &[&ids, &refunds],
    )
    .await?;
    for row in &stored {
        let id: Uuid = row.try_get(0)?;
        let refund: &str = row.try_get(1)?;
        if row.try_get(2)? {
            return Err(LeafStoreError::Transferred(node_id(id)?));
        }
        if let Some(wanted) = leaves.get(&id)
            && wanted.refund_tx_id != refund
        {
            return Err(LeafStoreError::DifferentRecord(wanted.node_id.clone()));
        }
    }
    Ok(())
}

pub(super) async fn remove_leaves(
    db: &impl GenericClient,
    leaves: &[TreeNode],
) -> Result<(), LeafStoreError> {
    let mut ids = Vec::with_capacity(leaves.len());
    let mut refunds = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        let refund = leaf
            .refund_tx
            .as_ref()
            .ok_or_else(|| invalid(format!("leaf {} has no refund", leaf.id)))?;
        ids.push(node_uuid(&leaf.id)?);
        refunds.push(refund.compute_txid().to_string());
    }
    let removed = query(
        db,
        r"DELETE FROM brz_ssp_spark_leaves l
           USING UNNEST($1::uuid[], $2::text[]) AS owned (node_id, refund_tx_id)
           WHERE l.node_id = owned.node_id
             AND l.refund_tx_id = owned.refund_tx_id
           RETURNING l.node_id",
        &[&ids, &refunds],
    )
    .await?
    .iter()
    .map(|row| row.try_get(0))
    .collect::<Result<Vec<Uuid>, _>>()?;
    match leaves
        .iter()
        .zip(&ids)
        .find(|(_, id)| !removed.contains(id))
    {
        Some((leaf, _)) => Err(LeafStoreError::ChangedDuringRenewal(leaf.id.clone())),
        None => Ok(()),
    }
}

/// `unreported` lists the nodes that the SSP queried the operators for at
/// `observed_at` and that they did not return.
pub(super) async fn observe(
    db: &impl GenericClient,
    nodes: &[TreeNode],
    unreported: &[Uuid],
    observed_at: DateTime<Utc>,
    refresh_observed_at: bool,
) -> Result<bool, LeafStoreError> {
    // Postgres fails an ON CONFLICT DO UPDATE that would update one row twice.
    let nodes: BTreeMap<Uuid, &TreeNode> = nodes
        .iter()
        .map(|node| Ok((node_uuid(&node.id)?, node)))
        .collect::<Result<_, LeafStoreError>>()?;
    if nodes.is_empty() && unreported.is_empty() {
        return Ok(false);
    }
    let ids: Vec<Uuid> = nodes.keys().copied().collect();
    let statuses: Vec<String> = nodes.values().map(|n| status_name(n.status)).collect();
    let owners: Vec<Option<Vec<u8>>> = nodes
        .values()
        .map(|n| {
            n.owner_identity_public_key
                .map(|key| key.serialize().to_vec())
        })
        .collect();
    let node_transactions: Vec<String> = nodes
        .values()
        .map(|n| n.node_tx.compute_txid().to_string())
        .collect();
    let refunds: Vec<Option<String>> = nodes
        .values()
        .map(|n| n.refund_tx.as_ref().map(|tx| tx.compute_txid().to_string()))
        .collect();
    let keyshares: Vec<Vec<u8>> = nodes
        .values()
        .map(|n| n.signing_keyshare.public_key.serialize().to_vec())
        .collect();
    let changed: i64 = query_one(
        db,
        r"WITH reported AS (
               SELECT reported.*
               FROM UNNEST($1::uuid[], $2::text[], $3::bytea[], $4::text[], $5::text[],
                           $6::bytea[])
                    AS reported (node_id, status, owner_identity_public_key, node_tx_id, refund_tx_id,
                                 keyshare_key)
               WHERE EXISTS (SELECT 1 FROM brz_ssp_spark_leaves l
                             WHERE l.node_id = reported.node_id)
           ), compared AS (
               SELECT reported.*,
                      observed.node_id IS NULL
                      OR (observed.status, observed.owner_identity_public_key, observed.node_tx_id,
                          observed.refund_tx_id, observed.keyshare_key)
                         IS DISTINCT FROM
                         (reported.status, reported.owner_identity_public_key, reported.node_tx_id,
                          reported.refund_tx_id, reported.keyshare_key) AS differs
               FROM reported
               LEFT JOIN brz_ssp_spark_operator_nodes observed
                 ON observed.node_id = reported.node_id
           ), written AS (
               INSERT INTO brz_ssp_spark_operator_nodes
                   (node_id, status, owner_identity_public_key, node_tx_id, refund_tx_id, keyshare_key,
                    observed_at)
               SELECT node_id, status, owner_identity_public_key, node_tx_id, refund_tx_id, keyshare_key, $7
               FROM compared
               WHERE $8 OR differs
               ON CONFLICT (node_id) DO UPDATE SET
                   status = EXCLUDED.status,
                   owner_identity_public_key = EXCLUDED.owner_identity_public_key,
                   node_tx_id = EXCLUDED.node_tx_id,
                   refund_tx_id = EXCLUDED.refund_tx_id,
                   keyshare_key = EXCLUDED.keyshare_key,
                   observed_at = EXCLUDED.observed_at
               WHERE EXCLUDED.observed_at >= brz_ssp_spark_operator_nodes.observed_at
               RETURNING node_id
           ), deleted AS (
               DELETE FROM brz_ssp_spark_operator_nodes
               WHERE node_id = ANY($9) AND observed_at < $7
               RETURNING node_id
           )
           SELECT (SELECT COUNT(*) FROM written) + (SELECT COUNT(*) FROM deleted)",
        &[
            &ids,
            &statuses,
            &owners,
            &node_transactions,
            &refunds,
            &keyshares,
            &observed_at,
            &refresh_observed_at,
            &unreported,
        ],
    )
    .await?
    .try_get(0)?;
    Ok(changed > 0)
}

/// A transaction on a leaf's exit chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitChainStep {
    pub spends: OutPoint,
    /// Whether a direct transaction with a timelock has the same input. The
    /// operators' watchtower broadcasts such a transaction once it is mature.
    pub direct: bool,
}

pub(super) async fn stored_chains(
    db: &impl GenericClient,
    node_txids: &[Txid],
) -> Result<HashMap<Txid, Vec<super::ChainTransaction>>, LeafStoreError> {
    let ids: Vec<String> = node_txids.iter().map(ToString::to_string).collect();
    let rows = query(
        db,
        r"WITH RECURSIVE exit_chain AS (
               SELECT t.tx_id AS node_tx_id, 0 AS level, t.tx_id, t.previous_tx_id,
                      t.previous_output_index
               FROM brz_ssp_spark_txs t
               WHERE t.tx_id = ANY($1)
             UNION ALL
               SELECT exit_chain.node_tx_id, exit_chain.level + 1, above.tx_id,
                      above.previous_tx_id, above.previous_output_index
               FROM exit_chain
               JOIN brz_ssp_spark_txs above ON above.tx_id = exit_chain.previous_tx_id
           )
           SELECT exit_chain.node_tx_id, exit_chain.tx_id, exit_chain.previous_tx_id,
                  exit_chain.previous_output_index,
                  EXISTS (SELECT 1 FROM brz_ssp_spark_direct_txs direct
                          WHERE direct.previous_tx_id = exit_chain.previous_tx_id
                            AND direct.previous_output_index = exit_chain.previous_output_index
                            AND direct.timelock > 0) AS direct,
                  EXISTS (SELECT 1 FROM brz_ssp_spark_nodes node
                          WHERE node.node_tx_id = exit_chain.tx_id
                            AND node.parent_node_id IS NULL) AS root
           FROM exit_chain
           ORDER BY exit_chain.node_tx_id, exit_chain.level",
        &[&ids],
    )
    .await?;
    let mut chains: HashMap<Txid, Vec<super::ChainTransaction>> = HashMap::new();
    let mut reaches_root: HashSet<Txid> = HashSet::new();
    for row in &rows {
        let node_txid = txid(row.try_get(0)?)?;
        chains
            .entry(node_txid)
            .or_default()
            .push(super::ChainTransaction {
                txid: txid(row.try_get(1)?)?,
                step: ExitChainStep {
                    spends: outpoint(row.try_get(2)?, row.try_get(3)?)?,
                    direct: row.try_get(4)?,
                },
            });
        if row.try_get(5)? {
            reaches_root.insert(node_txid);
        }
    }
    chains.retain(|node_txid, _| reaches_root.contains(node_txid));
    Ok(chains)
}

#[derive(Debug, Clone)]
pub(super) struct Selected {
    pub refund_tx_id: String,
    pub node_id: Uuid,
    pub value: u64,
    pub refund_timelock: i32,
    pub chain: Vec<ExitChainStep>,
}

const EXIT_CHAIN: &str = r"exit_chain AS (
        SELECT t.tx_id AS refund_tx_id, 0 AS level, t.previous_tx_id, t.previous_output_index
        FROM selected
        JOIN brz_ssp_spark_txs t ON t.tx_id = selected.refund_tx_id
      UNION ALL
        SELECT exit_chain.refund_tx_id, exit_chain.level + 1, above.previous_tx_id,
               above.previous_output_index
        FROM exit_chain
        JOIN brz_ssp_spark_txs above ON above.tx_id = exit_chain.previous_tx_id
    )";

const SELECTED_COLUMNS: &str = r"
    selected.refund_tx_id, selected.node_id, selected.value, selected.refund_timelock,
    exit_chain.previous_tx_id, exit_chain.previous_output_index,
    EXISTS (SELECT 1 FROM brz_ssp_spark_direct_txs direct
            WHERE direct.previous_tx_id = exit_chain.previous_tx_id
              AND direct.previous_output_index = exit_chain.previous_output_index
              AND direct.timelock > 0) AS direct";

/// Expects the rows of each leaf next to each other, in the order of their exit
/// chain level.
fn selected<T>(
    rows: &[Row],
    extra: impl Fn(&Row) -> Result<T, LeafStoreError>,
) -> Result<Vec<(Selected, T)>, LeafStoreError> {
    let mut leaves: Vec<(Selected, T)> = Vec::new();
    for row in rows {
        let refund_tx_id: String = row.try_get("refund_tx_id")?;
        let step = ExitChainStep {
            spends: outpoint(
                row.try_get("previous_tx_id")?,
                row.try_get("previous_output_index")?,
            )?,
            direct: row.try_get("direct")?,
        };
        match leaves.last_mut() {
            Some((leaf, _)) if leaf.refund_tx_id == refund_tx_id => leaf.chain.push(step),
            _ => leaves.push((
                Selected {
                    refund_tx_id,
                    node_id: row.try_get("node_id")?,
                    value: row.try_get::<_, i64>("value")?.cast_unsigned(),
                    refund_timelock: row.try_get("refund_timelock")?,
                    chain: vec![step],
                },
                extra(row)?,
            )),
        }
    }
    Ok(leaves)
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Wanted {
    pub value: u64,
    pub count: i64,
    pub after: (i32, Uuid),
}

pub(super) async fn select(
    db: &impl GenericClient,
    identity: &PublicKey,
    wanted: &[Wanted],
) -> Result<Vec<Selected>, LeafStoreError> {
    let values: Vec<i64> = wanted.iter().map(|w| w.value.cast_signed()).collect();
    let counts: Vec<i64> = wanted.iter().map(|w| w.count).collect();
    let timelocks: Vec<i32> = wanted.iter().map(|w| w.after.0).collect();
    let nodes: Vec<Uuid> = wanted.iter().map(|w| w.after.1).collect();
    let sql = format!(
        r"WITH RECURSIVE selected AS (
               SELECT leaf.*
               FROM UNNEST($2::bigint[], $3::bigint[], $4::int[], $5::uuid[])
                    AS wanted (value, count, after_timelock, after_node_id)
               CROSS JOIN LATERAL (
                   SELECT l.refund_tx_id, l.node_id, l.value, l.refund_timelock
                   FROM brz_ssp_spark_leaves l
                   WHERE l.value = wanted.value
                     AND (l.refund_timelock, l.node_id)
                         > (wanted.after_timelock, wanted.after_node_id)
                     AND {UNRESERVED}
                     AND {OPERATORS_AGREE}
                   ORDER BY l.refund_timelock, l.node_id
                   LIMIT wanted.count
                   FOR NO KEY UPDATE OF l SKIP LOCKED
               ) leaf
           ), {EXIT_CHAIN}
           SELECT {SELECTED_COLUMNS}
           FROM selected
           JOIN exit_chain ON exit_chain.refund_tx_id = selected.refund_tx_id
           ORDER BY selected.value, selected.refund_timelock, selected.node_id,
                    exit_chain.level"
    );
    let rows = query(
        db,
        &sql,
        &[
            &identity.serialize().as_slice(),
            &values,
            &counts,
            &timelocks,
            &nodes,
        ],
    )
    .await?;
    Ok(selected(&rows, |_| Ok(()))?
        .into_iter()
        .map(|(leaf, ())| leaf)
        .collect())
}

pub(super) async fn select_at_or_below(
    db: &impl GenericClient,
    timelock: i32,
    after: (i32, Uuid),
    limit: i64,
) -> Result<Vec<Selected>, LeafStoreError> {
    let sql = format!(
        r"WITH RECURSIVE selected AS (
               SELECT l.refund_tx_id, l.node_id, l.value, l.refund_timelock
               FROM brz_ssp_spark_leaves l
               WHERE l.refund_timelock <= $1
                 AND (l.refund_timelock, l.node_id) > ($2, $3)
                 AND {UNRESERVED}
               ORDER BY l.refund_timelock, l.node_id
               LIMIT $4
           ), {EXIT_CHAIN}
           SELECT {SELECTED_COLUMNS}
           FROM selected
           JOIN exit_chain ON exit_chain.refund_tx_id = selected.refund_tx_id
           ORDER BY selected.refund_timelock, selected.node_id, exit_chain.level"
    );
    let rows = query(db, &sql, &[&timelock, &after.0, &after.1, &limit]).await?;
    Ok(selected(&rows, |_| Ok(()))?
        .into_iter()
        .map(|(leaf, ())| leaf)
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Reported {
    Unreported,
    Status(String),
    AnotherOwner,
    AnotherRefund,
    /// The operators report the leaf as AVAILABLE, as the SSP's and at the
    /// SSP's refund, but the SSP finds it exiting.
    Available,
}

/// An unreserved leaf that only one of the SSP and the operators would use.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Dispute {
    pub node_id: TreeNodeId,
    pub reported: Reported,
}

impl std::fmt::Display for Dispute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id = &self.node_id;
        match &self.reported {
            Reported::Unreported => {
                write!(
                    f,
                    "the SSP would use leaf {id}, but the operators do not report it"
                )
            }
            Reported::Status(status) => write!(
                f,
                "the SSP would use leaf {id}, but the operators report it as {status}"
            ),
            Reported::AnotherOwner => write!(
                f,
                "the SSP would use leaf {id}, but the operators report another owner for it"
            ),
            Reported::AnotherRefund => write!(
                f,
                "the SSP would use leaf {id}, but the operators report another refund for it"
            ),
            Reported::Available => write!(
                f,
                "leaf {id} is exiting, but the operators report it as AVAILABLE"
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Compared {
    pub reserved: bool,
    pub agreed: bool,
    pub status: Option<String>,
    pub same_owner: bool,
    pub operator_refund_tx_id: Option<String>,
    pub observed_at: Option<DateTime<Utc>>,
}

impl Compared {
    /// Expects a leaf the operators do not agree on.
    pub(super) fn reported(&self) -> Reported {
        let available = status_name(TreeNodeStatus::Available);
        match &self.status {
            None => Reported::Unreported,
            Some(status) if *status != available => Reported::Status(status.clone()),
            Some(_) if self.same_owner => Reported::AnotherRefund,
            Some(_) => Reported::AnotherOwner,
        }
    }
}

pub(super) enum Compare<'a> {
    Nodes(&'a [Uuid]),
    After(Uuid, i64),
}

pub(super) async fn compare(
    db: &impl GenericClient,
    identity: &PublicKey,
    which: Compare<'_>,
) -> Result<Vec<(Selected, Compared)>, LeafStoreError> {
    let filter = match which {
        Compare::Nodes(_) => "WHERE l.node_id = ANY($2)",
        Compare::After(..) => "WHERE l.node_id > $2 ORDER BY l.node_id LIMIT $3",
    };
    let sql = format!(
        r"WITH RECURSIVE selected AS (
               SELECT l.refund_tx_id, l.node_id, l.value, l.refund_timelock,
                      NOT ({UNRESERVED}) AS reserved,
                      {OPERATORS_AGREE} AS agreed,
                      observed.status,
                      observed.owner_identity_public_key IS NOT DISTINCT FROM $1 AS same_owner,
                      observed.refund_tx_id AS operator_refund_tx_id,
                      observed.observed_at
               FROM brz_ssp_spark_leaves l
               LEFT JOIN brz_ssp_spark_operator_nodes observed ON observed.node_id = l.node_id
               {filter}
           ), {EXIT_CHAIN}
           SELECT {SELECTED_COLUMNS}, selected.reserved, selected.agreed, selected.status,
                  selected.same_owner, selected.operator_refund_tx_id, selected.observed_at
           FROM selected
           JOIN exit_chain ON exit_chain.refund_tx_id = selected.refund_tx_id
           ORDER BY selected.node_id, exit_chain.level"
    );
    let identity = identity.serialize().to_vec();
    let rows = match which {
        Compare::Nodes(ids) => query(db, &sql, &[&identity, &ids]).await?,
        Compare::After(after, limit) => query(db, &sql, &[&identity, &after, &limit]).await?,
    };
    selected(&rows, |row| {
        Ok(Compared {
            reserved: row.try_get("reserved")?,
            agreed: row.try_get("agreed")?,
            status: row.try_get("status")?,
            same_owner: row.try_get("same_owner")?,
            operator_refund_tx_id: row.try_get("operator_refund_tx_id")?,
            observed_at: row.try_get("observed_at")?,
        })
    })
}

pub(super) fn transaction(row: &Row, column: &str) -> Result<Transaction, LeafStoreError> {
    let raw: Vec<u8> = row.try_get(column)?;
    bitcoin::consensus::deserialize(&raw).map_err(|e| invalid(format!("{column}: {e}")))
}

fn optional_transaction(row: &Row, column: &str) -> Result<Option<Transaction>, LeafStoreError> {
    let raw: Option<Vec<u8>> = row.try_get(column)?;
    raw.map(|raw| {
        bitcoin::consensus::deserialize(&raw).map_err(|e| invalid(format!("{column}: {e}")))
    })
    .transpose()
}

fn key(row: &Row, column: &str) -> Result<PublicKey, LeafStoreError> {
    let bytes: Vec<u8> = row.try_get(column)?;
    PublicKey::from_slice(&bytes).map_err(|e| invalid(format!("{column}: {e}")))
}

const NODE_COLUMNS: &str = r"
    node.node_id, node.tree_id, node.parent_node_id, node.verifying_key, node.keyshare_key,
    node.keyshare_threshold, node.keyshare_owners,
    node_transaction.raw AS node_transaction,
    node_transaction.previous_output_index AS output_index,
    (SELECT direct.raw FROM brz_ssp_spark_direct_txs direct
     WHERE direct.cpfp_tx_id = node_transaction.tx_id
     ORDER BY direct.stored_at DESC LIMIT 1) AS direct_transaction";

fn node(row: &Row) -> Result<TreeNode, LeafStoreError> {
    let node_tx = transaction(row, "node_transaction")?;
    let owners: Vec<u8> = row.try_get("keyshare_owners")?;
    let owner_identifiers = owners
        .chunks(IDENTIFIER_LEN)
        .map(|bytes| {
            Identifier::deserialize(bytes).map_err(|e| invalid(format!("keyshare owner: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let parent: Option<Uuid> = row.try_get("parent_node_id")?;
    let tree_id: Uuid = row.try_get("tree_id")?;
    Ok(TreeNode {
        id: node_id(row.try_get("node_id")?)?,
        tree_id: tree_id.to_string(),
        value: node_tx.output.iter().map(|o| o.value.to_sat()).sum(),
        parent_node_id: parent.map(node_id).transpose()?,
        node_tx,
        refund_tx: None,
        direct_tx: optional_transaction(row, "direct_transaction")?,
        direct_refund_tx: None,
        direct_from_cpfp_refund_tx: None,
        vout: row.try_get::<_, i32>("output_index")?.cast_unsigned(),
        verifying_public_key: key(row, "verifying_key")?,
        owner_identity_public_key: None,
        signing_keyshare: SigningKeyshare {
            owner_identifiers,
            threshold: row.try_get::<_, i32>("keyshare_threshold")?.cast_unsigned(),
            public_key: key(row, "keyshare_key")?,
        },
        status: TreeNodeStatus::Splitted,
    })
}

pub(super) enum Which<'a> {
    All,
    Transfer(Uuid),
    Nodes(&'a [Uuid]),
    Reserving {
        refunds: &'a [String],
        transfer: Uuid,
    },
}

pub(super) async fn leaves(
    db: &impl GenericClient,
    identity: &PublicKey,
    which: Which<'_>,
) -> Result<Vec<TreeNode>, LeafStoreError> {
    let (reserving, filter) = match which {
        Which::All => ("", ""),
        Which::Transfer(_) => (
            "",
            r"JOIN brz_ssp_spark_reservations reservation
                ON reservation.refund_tx_id = l.refund_tx_id
               WHERE reservation.transfer_id = $1",
        ),
        Which::Nodes(_) => ("", "WHERE l.node_id = ANY($1)"),
        // The store has locked the leaves at these refunds, so another caller
        // can have reserved one of them only if it committed after this caller
        // started its search. Postgres returns no row for such a leaf.
        Which::Reserving { .. } => (
            r"WITH reserved AS (
                   INSERT INTO brz_ssp_spark_reservations (refund_tx_id, transfer_id)
                   SELECT refund_tx_id, $2
                   FROM UNNEST($1::text[]) AS selected (refund_tx_id)
                   ORDER BY refund_tx_id
                   ON CONFLICT (refund_tx_id) DO NOTHING
                   RETURNING refund_tx_id
               )",
            "JOIN reserved ON reserved.refund_tx_id = l.refund_tx_id",
        ),
    };
    // The input of a direct refund is an output of the direct node transaction.
    // The direct refund made from the cpfp node transaction has the same input
    // as the refund itself.
    let sql = format!(
        r"{reserving}
           SELECT {NODE_COLUMNS},
                 l.value,
                 refund.raw AS refund_transaction,
                 (SELECT direct.raw FROM brz_ssp_spark_direct_txs direct
                  WHERE direct.cpfp_tx_id = refund.tx_id
                    AND direct.previous_tx_id <> refund.previous_tx_id
                  ORDER BY direct.stored_at DESC LIMIT 1) AS direct_refund_transaction,
                 (SELECT direct.raw FROM brz_ssp_spark_direct_txs direct
                  WHERE direct.cpfp_tx_id = refund.tx_id
                    AND direct.previous_tx_id = refund.previous_tx_id
                  ORDER BY direct.stored_at DESC LIMIT 1)
                     AS direct_from_cpfp_refund_transaction,
                 observed.status AS operator_status
           FROM brz_ssp_spark_leaves l
           JOIN brz_ssp_spark_txs refund ON refund.tx_id = l.refund_tx_id
           JOIN brz_ssp_spark_txs node_transaction
             ON node_transaction.tx_id = refund.previous_tx_id
           JOIN brz_ssp_spark_nodes node ON node.node_id = l.node_id
           LEFT JOIN brz_ssp_spark_operator_nodes observed ON observed.node_id = l.node_id
           {filter}
           ORDER BY l.node_id"
    );
    let rows = match which {
        Which::All => query(db, &sql, &[]).await?,
        Which::Transfer(id) => query(db, &sql, &[&id]).await?,
        Which::Nodes(ids) => query(db, &sql, &[&ids]).await?,
        Which::Reserving { refunds, transfer } => query(db, &sql, &[&refunds, &transfer]).await?,
    };
    rows.iter()
        .map(|row| {
            let mut node = node(row)?;
            node.value = row.try_get::<_, i64>("value")?.cast_unsigned();
            node.refund_tx = Some(transaction(row, "refund_transaction")?);
            node.direct_refund_tx = optional_transaction(row, "direct_refund_transaction")?;
            node.direct_from_cpfp_refund_tx =
                optional_transaction(row, "direct_from_cpfp_refund_transaction")?;
            node.owner_identity_public_key = Some(*identity);
            let operator_status: Option<String> = row.try_get("operator_status")?;
            node.status = operator_status
                .as_deref()
                .map_or(TreeNodeStatus::Available, status_from_name);
            Ok(node)
        })
        .collect()
}

pub(super) async fn ancestors(
    db: &impl GenericClient,
    leaf_ids: &[Uuid],
) -> Result<HashMap<TreeNodeId, Vec<TreeNode>>, LeafStoreError> {
    let sql = format!(
        r"WITH RECURSIVE above AS (
               SELECT l.node_id AS leaf_id, 1 AS distance, parent.node_id
               FROM brz_ssp_spark_leaves l
               JOIN brz_ssp_spark_nodes leaf ON leaf.node_id = l.node_id
               JOIN brz_ssp_spark_txs leaf_transaction
                 ON leaf_transaction.tx_id = leaf.node_tx_id
               JOIN brz_ssp_spark_nodes parent
                 ON parent.node_tx_id = leaf_transaction.previous_tx_id
               WHERE l.node_id = ANY($1)
             UNION ALL
               SELECT above.leaf_id, above.distance + 1, parent.node_id
               FROM above
               JOIN brz_ssp_spark_nodes child ON child.node_id = above.node_id
               JOIN brz_ssp_spark_txs child_transaction
                 ON child_transaction.tx_id = child.node_tx_id
               JOIN brz_ssp_spark_nodes parent
                 ON parent.node_tx_id = child_transaction.previous_tx_id
           )
           SELECT above.leaf_id, {NODE_COLUMNS}
           FROM above
           JOIN brz_ssp_spark_nodes node ON node.node_id = above.node_id
           JOIN brz_ssp_spark_txs node_transaction
             ON node_transaction.tx_id = node.node_tx_id
           ORDER BY above.leaf_id, above.distance"
    );
    let rows = query(db, &sql, &[&leaf_ids]).await?;
    let mut ancestors: HashMap<TreeNodeId, Vec<TreeNode>> = HashMap::new();
    for row in &rows {
        ancestors
            .entry(node_id(row.try_get("leaf_id")?)?)
            .or_default()
            .push(node(row)?);
    }
    Ok(ancestors)
}
