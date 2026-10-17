mod rows;
mod unilateral_exit;

#[cfg(test)]
pub(crate) mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use bitcoin::{OutPoint, Txid};
use chrono::{DateTime, Utc};
use spark::services::{TransferId, csv_timelock};
use spark::tree::{LeafPedigree, TreeNode, TreeNodeId};
use sqlx::{PgConnection, PgPool};
use tracing::info;
use uuid::Uuid;

use super::ChainRepository;
use crate::chain::{ChainRepositoryError, OutputStates, Spender};

use rows::{Compare, Compared, Rows, Selected, Wanted, Which, node_uuid, transfer_uuid};
pub use rows::{Dispute, ExitChainStep, Reported};
pub use unilateral_exit::MatureTransaction;
pub use unilateral_exit::RefundOutput;

#[derive(Debug, thiserror::Error)]
pub enum LeafStoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("chain record error: {0}")]
    Chain(#[from] ChainRepositoryError),

    #[error("the SSP has already transferred leaf {0}")]
    Transferred(TreeNodeId),

    #[error("the store has a different record of leaf {0}")]
    DifferentRecord(TreeNodeId),

    #[error("the SSP changed its record of leaf {0} before it could record the renewal")]
    ChangedDuringRenewal(TreeNodeId),

    #[error("other callers reserved the selected leaves first in every attempt")]
    Contended,

    #[error("invalid leaf data: {0}")]
    Invalid(String),
}

const UNRESERVED: &str = r"
    NOT EXISTS (SELECT 1 FROM brz_ssp_spark_reservations r
                WHERE r.refund_tx_id = l.refund_tx_id)";

/// Postgres does not turn a subquery with `OFFSET 0` into a join. If it did,
/// its planner would treat the matches on node id and on refund as independent,
/// estimate that almost no leaf meets both, and plan every later join for a
/// single row.
const OPERATORS_AGREE: &str = r"
    EXISTS (SELECT 1 FROM brz_ssp_spark_operator_nodes o
            WHERE o.node_id = l.node_id
              AND o.status = 'Available'
              AND o.owner_identity_public_key = $1
              AND o.refund_tx_id = l.refund_tx_id
            OFFSET 0)";

/// A leaf whose refund timelock is at or below this, in blocks, needs a renewal
/// before the SSP transfers it again. When the SSP transfers a leaf, the refund
/// it signs for the receiver has a timelock 100 blocks lower than the SSP's
/// refund.
const RENEW_TIMELOCK_THRESHOLD: i32 = 100;

/// A query for the refunds of the leaves that are exiting once the outputs in
/// the relation `outputs` are spent. The query has the same rule as
/// [`exiting_steps`], but Postgres applies it from the outputs down instead of
/// from each leaf up, which suits a count over every leaf.
fn refunds_spending(outputs: &str) -> String {
    // The operators' watchtower does not broadcast a direct transaction without
    // a timelock.
    format!(
        r"WITH RECURSIVE below_direct AS (
               SELECT child.tx_id
               FROM {outputs} listed
               JOIN brz_ssp_spark_txs parent
                 ON parent.previous_tx_id = listed.tx_id
                AND parent.previous_output_index = listed.output_index
               JOIN brz_ssp_spark_txs child
                 ON child.previous_tx_id = parent.tx_id
               WHERE EXISTS (
                   SELECT 1 FROM brz_ssp_spark_direct_txs direct
                   WHERE direct.previous_tx_id = child.previous_tx_id
                     AND direct.previous_output_index = child.previous_output_index
                     AND direct.timelock > 0)
             UNION
               SELECT child.tx_id
               FROM below_direct
               JOIN brz_ssp_spark_txs child
                 ON child.previous_tx_id = below_direct.tx_id
           )
           SELECT refund.tx_id AS refund_tx_id
           FROM {outputs} listed
           JOIN brz_ssp_spark_txs refund
             ON refund.previous_tx_id = listed.tx_id
            AND refund.previous_output_index = listed.output_index
         UNION
           SELECT refund.tx_id
           FROM {outputs} listed
           JOIN brz_ssp_spark_txs node
             ON node.previous_tx_id = listed.tx_id
            AND node.previous_output_index = listed.output_index
           JOIN brz_ssp_spark_txs refund
             ON refund.previous_tx_id = node.tx_id
         UNION
           SELECT refund.tx_id
           FROM {outputs} listed
           JOIN brz_ssp_spark_txs parent
             ON parent.previous_tx_id = listed.tx_id
            AND parent.previous_output_index = listed.output_index
           JOIN brz_ssp_spark_txs node
             ON node.previous_tx_id = parent.tx_id
           JOIN brz_ssp_spark_txs refund
             ON refund.previous_tx_id = node.tx_id
         UNION
           SELECT tx_id FROM below_direct"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainStatus {
    Exiting(Spender),
    Unchecked,
}

/// The steps of `chain` whose input, once spent on chain, means that the leaf
/// is exiting.
///
/// A leaf is exiting once the input of its refund, of its node transaction or
/// of its parent's node transaction is spent. It is also exiting once the input
/// of a transaction further up is spent, if the next transaction down has a
/// direct transaction with a timelock. The operators' watchtower broadcasts
/// that direct transaction once it is mature, and after it has confirmed, the
/// leaf's node transaction and refund can never confirm.
///
/// The operators do not transfer an exiting leaf, and stop reporting it as
/// AVAILABLE. They keep reporting a leaf as AVAILABLE when any other
/// transaction above it confirms.
pub fn exiting_steps(chain: &[ExitChainStep]) -> impl Iterator<Item = &ExitChainStep> {
    chain.iter().enumerate().filter_map(|(level, step)| {
        let below_has_direct = level
            .checked_sub(1)
            .and_then(|below| chain.get(below))
            .is_some_and(|below| below.direct);
        (level <= 2 || below_has_direct).then_some(step)
    })
}

fn chain_status(chain: &[ExitChainStep], states: &OutputStates) -> Option<ChainStatus> {
    let spent = exiting_steps(chain).find_map(|step| states.spent.get(&step.spends));
    if let Some(spender) = spent {
        return Some(ChainStatus::Exiting(*spender));
    }
    exiting_steps(chain)
        .any(|step| states.unchecked.contains(&step.spends))
        .then_some(ChainStatus::Unchecked)
}

fn has_timelock(transaction: Option<&bitcoin::Transaction>) -> bool {
    transaction.is_some_and(|transaction| csv_timelock(transaction).is_some())
}

pub fn refund_step(leaf: &TreeNode) -> ExitChainStep {
    let spends = leaf
        .refund_tx
        .as_ref()
        .and_then(|refund| refund.input.first())
        .map_or_else(
            || OutPoint::new(leaf.node_tx.compute_txid(), 0),
            |input| input.previous_output,
        );
    ExitChainStep {
        spends,
        direct: has_timelock(leaf.direct_refund_tx.as_ref())
            || has_timelock(leaf.direct_from_cpfp_refund_tx.as_ref()),
    }
}

pub fn exit_chain(pedigree: &LeafPedigree) -> Vec<ExitChainStep> {
    let leaf = &pedigree.leaf;
    let mut steps = vec![refund_step(leaf)];
    for node in std::iter::once(leaf).chain(&pedigree.ancestors) {
        if let Some(input) = node.node_tx.input.first() {
            steps.push(ExitChainStep {
                spends: input.previous_output,
                direct: has_timelock(node.direct_tx.as_ref()),
            });
        }
    }
    steps
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainTransaction {
    pub txid: Txid,
    pub step: ExitChainStep,
}

#[derive(Debug, Default)]
pub struct StoredChains {
    /// For each node transaction whose exit chain the store has up to its
    /// tree's root: that transaction and the transactions above it, the root
    /// last.
    pub chains: HashMap<Txid, Vec<ChainTransaction>>,
    /// What the chain record has about the inputs of those transactions.
    pub states: OutputStates,
    /// The height of each funding transaction that is in a block of the chain
    /// record.
    pub funding_heights: HashMap<Txid, u64>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PoolCounts {
    pub available: HashMap<u64, u32>,
    /// How many leaves of each value are unavailable only because the SSP has
    /// yet to check outputs on their exit chains with bitcoind.
    pub unchecked: HashMap<u64, u32>,
}

#[derive(Debug, Clone)]
pub struct DisputeDetails {
    pub dispute: Dispute,
    pub refund_txid: Txid,
    pub value: u64,
    pub refund_timelock: u32,
    pub exiting: Option<Spender>,
    pub operator_refund_txid: Option<String>,
    pub observed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reservation {
    pub transfer_id: TransferId,
    pub leaves: u32,
    pub value: u64,
    pub reserved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
pub struct RenewalCursor(i32, Uuid);

impl Default for RenewalCursor {
    fn default() -> Self {
        Self(i32::MIN, Uuid::nil())
    }
}

const SELECTION_GROWTH: i64 = 4;

const MAX_SELECTION: i64 = 256;

const RESERVE_ATTEMPTS: usize = 5;

pub struct PostgresLeafStore {
    pool: Arc<PgPool>,
    identity: PublicKey,
    chain: Arc<ChainRepository>,
}

impl PostgresLeafStore {
    pub fn new(pool: Arc<PgPool>, identity: PublicKey, chain: Arc<ChainRepository>) -> Self {
        Self {
            pool,
            identity,
            chain,
        }
    }

    pub async fn connect(
        url: &str,
        identity: PublicKey,
        network: bitcoin::Network,
    ) -> Result<Self, LeafStoreError> {
        let pool = Arc::new(PgPool::connect(url).await?);
        let chain = Arc::new(ChainRepository::new(Arc::clone(&pool), network));
        Ok(Self::new(pool, identity, chain))
    }

    /// Call this after committing a write that registered outputs: the spend
    /// check does not see them before the commit. A method that takes a
    /// connection does not commit, so it leaves this call to its caller.
    pub(in crate::postgresql) fn wake_spend_check(&self) {
        self.chain.watched().wake();
    }

    pub async fn insert_leaves(
        &self,
        db: &mut PgConnection,
        pedigrees: &[LeafPedigree],
    ) -> Result<(), LeafStoreError> {
        let mut rows = Rows::default();
        for pedigree in pedigrees {
            for ancestor in &pedigree.ancestors {
                rows.node(ancestor)?;
            }
            rows.leaf(&pedigree.leaf)?;
        }
        rows.write(db, &self.chain).await
    }

    pub async fn observe(
        &self,
        db: &mut PgConnection,
        nodes: &[TreeNode],
        observed_at: DateTime<Utc>,
    ) -> Result<(), LeafStoreError> {
        rows::observe(db, nodes, &[], observed_at, true).await?;
        Ok(())
    }

    /// The caller must have verified each transaction's signature.
    pub async fn store_nodes(&self, pedigrees: &[LeafPedigree]) -> Result<(), LeafStoreError> {
        let mut rows = Rows::default();
        for pedigree in pedigrees {
            for node in pedigree.ancestors.iter().chain([&pedigree.leaf]) {
                rows.node(node)?;
            }
        }
        let mut db = self.pool.begin().await?;
        rows.write(&mut db, &self.chain).await?;
        db.commit().await?;
        self.wake_spend_check();
        Ok(())
    }

    pub async fn stored_chains(&self, node_txids: &[Txid]) -> Result<StoredChains, LeafStoreError> {
        let mut stored = StoredChains::default();
        if node_txids.is_empty() {
            return Ok(stored);
        }
        let mut db = self.pool.acquire().await?;
        stored.chains = rows::stored_chains(&mut db, node_txids).await?;
        let inputs: HashSet<OutPoint> = stored
            .chains
            .values()
            .flatten()
            .map(|transaction| transaction.step.spends)
            .collect();
        let inputs: Vec<OutPoint> = inputs.into_iter().collect();
        let funding: HashSet<Txid> = stored
            .chains
            .values()
            .filter_map(|chain| chain.last())
            .map(|root| root.step.spends.txid)
            .collect();
        let funding: Vec<Txid> = funding.into_iter().collect();
        (stored.states, stored.funding_heights) = self
            .chain
            .output_states_and_tx_heights(&mut db, &inputs, &funding)
            .await?;
        Ok(stored)
    }

    pub async fn stored_transactions(
        &self,
        txids: &[Txid],
    ) -> Result<HashSet<Txid>, LeafStoreError> {
        let ids: Vec<String> = txids.iter().map(ToString::to_string).collect();
        let stored: Vec<(String,)> =
            sqlx::query_as("SELECT tx_id FROM brz_ssp_spark_txs WHERE tx_id = ANY($1)")
                .bind(&ids)
                .fetch_all(&*self.pool)
                .await?;
        stored.iter().map(|(id,)| rows::txid(id)).collect()
    }

    pub async fn claimed(
        &self,
        transfer_id: &TransferId,
        leaves: &[LeafPedigree],
        queried_at: DateTime<Utc>,
    ) -> Result<bool, LeafStoreError> {
        let mut db = self.pool.begin().await?;
        let first = sqlx::query(
            r"INSERT INTO brz_ssp_spark_claimed_transfers (transfer_id) VALUES ($1)
               ON CONFLICT (transfer_id) DO NOTHING",
        )
        .bind(transfer_uuid(transfer_id)?)
        .execute(&mut *db)
        .await?
        .rows_affected();
        if first == 0 {
            return Ok(false);
        }
        self.insert_leaves(&mut db, leaves).await?;
        let reported: Vec<TreeNode> = leaves.iter().map(|p| p.leaf.clone()).collect();
        rows::observe(&mut db, &reported, &[], queried_at, true).await?;
        db.commit().await?;
        self.wake_spend_check();
        for leaf in &reported {
            info!(leaf_id = %leaf.id, %transfer_id, refund_txid = %refund_txid(leaf), "added a claimed leaf to the pool");
        }
        Ok(true)
    }

    /// The store reads up to `limit` leaves and skips those that are exiting or
    /// unchecked, so it can return fewer leaves, or none, while more remain.
    pub async fn leaves_needing_renewal(
        &self,
        cursor: RenewalCursor,
        limit: usize,
    ) -> Result<(Vec<LeafPedigree>, Option<RenewalCursor>), LeafStoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut db = self.pool.acquire().await?;
        let selected = rows::select_at_or_below(
            &mut db,
            RENEW_TIMELOCK_THRESHOLD,
            (cursor.0, cursor.1),
            limit,
        )
        .await?;
        let next = selected
            .last()
            .filter(|_| i64::try_from(selected.len()).is_ok_and(|looked_at| looked_at == limit))
            .map(|last| RenewalCursor(last.refund_timelock, last.node_id));
        let states = self.states(&mut db, &selected).await?;
        let ids: Vec<Uuid> = selected
            .iter()
            .filter(|leaf| chain_status(&leaf.chain, &states).is_none())
            .map(|leaf| leaf.node_id)
            .collect();
        Ok((self.pedigrees_of(&mut db, &ids).await?, next))
    }

    pub async fn renewed(
        &self,
        before: &[TreeNode],
        after: &[LeafPedigree],
        queried_at: DateTime<Utc>,
    ) -> Result<(), LeafStoreError> {
        let mut db = self.pool.begin().await?;
        rows::remove_leaves(&mut db, before).await?;
        self.insert_leaves(&mut db, after).await?;
        let leaves: Vec<TreeNode> = after.iter().map(|pedigree| pedigree.leaf.clone()).collect();
        rows::observe(&mut db, &leaves, &[], queried_at, true).await?;
        db.commit().await?;
        self.wake_spend_check();
        for (old, new) in before.iter().zip(&leaves) {
            info!(leaf_id = %new.id, replaced_leaf_id = %old.id, replaced_refund_txid = %refund_txid(old), refund_txid = %refund_txid(new), "replaced a pool leaf by the renewed leaf");
        }
        Ok(())
    }

    /// The store records what the operators reported in their view only. It
    /// does not change what the SSP has for any leaf.
    pub async fn operators_reported(
        &self,
        queried: &[TreeNodeId],
        reported: &[TreeNode],
        queried_at: DateTime<Utc>,
    ) -> Result<(), LeafStoreError> {
        let reported_ids: HashSet<&TreeNodeId> = reported.iter().map(|node| &node.id).collect();
        let unreported = queried
            .iter()
            .filter(|id| !reported_ids.contains(id))
            .map(node_uuid)
            .collect::<Result<Vec<Uuid>, _>>()?;
        // Postgres can fail the write with a foreign key violation when another
        // caller deletes one of these leaves while the store writes. The store
        // then writes again, and skips that leaf, which is no longer the SSP's.
        let mut written = self.write_reported(reported, &unreported, queried_at).await;
        if matches!(&written, Err(LeafStoreError::Database(e)) if is_foreign_key_violation(e)) {
            written = self.write_reported(reported, &unreported, queried_at).await;
        }
        written
    }

    async fn write_reported(
        &self,
        reported: &[TreeNode],
        unreported: &[Uuid],
        queried_at: DateTime<Utc>,
    ) -> Result<(), LeafStoreError> {
        let mut db = self.pool.acquire().await?;
        rows::observe(&mut db, reported, unreported, queried_at, false).await?;
        Ok(())
    }

    pub async fn disputes_among(
        &self,
        node_ids: &[TreeNodeId],
    ) -> Result<Vec<DisputeDetails>, LeafStoreError> {
        let ids = node_ids
            .iter()
            .map(node_uuid)
            .collect::<Result<Vec<Uuid>, _>>()?;
        let mut db = self.pool.acquire().await?;
        let compared = rows::compare(&mut db, &self.identity, Compare::Nodes(&ids)).await?;
        self.disputes(&mut db, compared).await
    }

    pub async fn available_among(
        &self,
        node_ids: &[TreeNodeId],
    ) -> Result<Vec<TreeNodeId>, LeafStoreError> {
        let ids = node_ids
            .iter()
            .map(node_uuid)
            .collect::<Result<Vec<Uuid>, _>>()?;
        let mut db = self.pool.acquire().await?;
        let compared = rows::compare(&mut db, &self.identity, Compare::Nodes(&ids)).await?;
        let states = self
            .states(&mut db, compared.iter().map(|(leaf, _)| leaf))
            .await?;
        compared
            .into_iter()
            .filter(|(leaf, operators)| {
                !operators.reserved
                    && operators.agreed
                    && leaf.refund_timelock > RENEW_TIMELOCK_THRESHOLD
                    && chain_status(&leaf.chain, &states).is_none()
            })
            .map(|(leaf, _)| rows::node_id(leaf.node_id))
            .collect()
    }

    pub async fn all_disputes(&self) -> Result<Vec<DisputeDetails>, LeafStoreError> {
        const PAGE: i64 = 1000;
        let mut db = self.pool.acquire().await?;
        let mut disputes = Vec::new();
        let mut after = Uuid::nil();
        loop {
            let compared =
                rows::compare(&mut db, &self.identity, Compare::After(after, PAGE)).await?;
            let Some((last, _)) = compared.last() else {
                return Ok(disputes);
            };
            after = last.node_id;
            disputes.extend(self.disputes(&mut db, compared).await?);
        }
    }

    async fn disputes(
        &self,
        db: &mut PgConnection,
        compared: Vec<(Selected, Compared)>,
    ) -> Result<Vec<DisputeDetails>, LeafStoreError> {
        let states = self
            .states(db, compared.iter().map(|(leaf, _)| leaf))
            .await?;
        let mut disputes = Vec::new();
        for (leaf, operators) in compared {
            let (reported, exiting) = match chain_status(&leaf.chain, &states) {
                _ if operators.reserved => continue,
                Some(ChainStatus::Exiting(spend)) if operators.agreed => {
                    (Reported::Available, Some(spend))
                }
                None if !operators.agreed => (operators.reported(), None),
                _ => continue,
            };
            disputes.push(DisputeDetails {
                dispute: Dispute {
                    node_id: rows::node_id(leaf.node_id)?,
                    reported,
                },
                refund_txid: rows::txid(&leaf.refund_tx_id)?,
                value: leaf.value,
                refund_timelock: leaf.refund_timelock.cast_unsigned(),
                exiting,
                operator_refund_txid: operators.operator_refund_tx_id,
                observed_at: operators.observed_at,
            });
        }
        Ok(disputes)
    }

    async fn states<'a>(
        &self,
        db: &mut PgConnection,
        leaves: impl IntoIterator<Item = &'a Selected>,
    ) -> Result<OutputStates, LeafStoreError> {
        let outpoints: HashSet<OutPoint> = leaves
            .into_iter()
            .flat_map(|leaf| leaf.chain.iter().map(|step| step.spends))
            .collect();
        let outpoints: Vec<OutPoint> = outpoints.into_iter().collect();
        Ok(self.chain.output_states(db, &outpoints).await?)
    }

    /// Returns `None`, and reserves nothing, when the pool is short of any of
    /// `denominations`. For a transfer that has leaves reserved already, the
    /// store returns those leaves and reserves no more, whatever
    /// `denominations` lists.
    pub async fn reserve(
        &self,
        denominations: &[u64],
        transfer_id: &TransferId,
    ) -> Result<Option<Vec<TreeNode>>, LeafStoreError> {
        let transfer = transfer_uuid(transfer_id)?;
        let mut wanted: BTreeMap<u64, i64> = BTreeMap::new();
        for denomination in denominations {
            let count = wanted.entry(*denomination).or_default();
            *count = count.saturating_add(1);
        }
        for _ in 0..RESERVE_ATTEMPTS {
            let mut db = self.pool.begin().await?;
            // Postgres makes a second caller for the same transfer wait here
            // until the first caller's database transaction ends. The second
            // caller then finds the leaves the first caller reserved.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
                .bind(transfer)
                .execute(&mut *db)
                .await?;
            let already_reserved =
                rows::leaves(&mut db, &self.identity, Which::Transfer(transfer)).await?;
            if !already_reserved.is_empty() {
                return Ok(Some(already_reserved));
            }
            let Some(selected) = self.select(&mut db, &wanted).await? else {
                return Ok(None);
            };
            // Postgres returns fewer leaves than the store selected when
            // another caller committed a reservation of one of them after this
            // search began. The store then rolls back and tries again, and in
            // its next search it sees that reservation.
            let reserving = Which::Reserving {
                refunds: &selected,
                transfer,
            };
            let leaves = rows::leaves(&mut db, &self.identity, reserving).await?;
            if leaves.len() == selected.len() {
                db.commit().await?;
                for leaf in &leaves {
                    info!(leaf_id = %leaf.id, %transfer_id, refund_txid = %refund_txid(leaf), "reserved a pool leaf for a transfer");
                }
                return Ok(Some(leaves));
            }
        }
        Err(LeafStoreError::Contended)
    }

    /// Postgres keeps every leaf this function looks at locked until `db`'s
    /// transaction ends.
    async fn select(
        &self,
        db: &mut PgConnection,
        wanted: &BTreeMap<u64, i64>,
    ) -> Result<Option<Vec<String>>, LeafStoreError> {
        let mut still: BTreeMap<u64, Wanted> = wanted
            .iter()
            .map(|(value, count)| {
                let after = (RENEW_TIMELOCK_THRESHOLD, Uuid::max());
                (
                    *value,
                    Wanted {
                        value: *value,
                        count: *count,
                        after,
                    },
                )
            })
            .collect();
        let mut refunds = Vec::new();
        let mut growth = 1;
        // The store works out which leaves are exiting or unchecked only after
        // it has selected them. When some are, it searches again for the
        // values still short. Nearly always it finds every leaf wanted in its
        // first search.
        while !still.is_empty() {
            let requested: Vec<Wanted> = still
                .values()
                .map(|wanted| Wanted {
                    count: wanted
                        .count
                        .saturating_mul(growth)
                        .min(MAX_SELECTION)
                        .max(wanted.count),
                    ..*wanted
                })
                .collect();
            let selected = rows::select(db, &self.identity, &requested).await?;
            let states = self.states(db, &selected).await?;
            let mut looked_at: HashMap<u64, i64> = HashMap::new();
            for leaf in &selected {
                let count = looked_at.entry(leaf.value).or_default();
                *count = count.saturating_add(1);
                let Some(wanted) = still.get_mut(&leaf.value) else {
                    continue;
                };
                wanted.after = (leaf.refund_timelock, leaf.node_id);
                if wanted.count > 0 && chain_status(&leaf.chain, &states).is_none() {
                    refunds.push(leaf.refund_tx_id.clone());
                    wanted.count = wanted.count.saturating_sub(1);
                }
            }
            // When the store finds fewer leaves of a value than it requested,
            // it cannot reserve any other leaf of that value at this moment.
            let short = requested.iter().any(|requested| {
                let found = looked_at.get(&requested.value).copied().unwrap_or(0);
                found < requested.count && still.get(&requested.value).is_some_and(|w| w.count > 0)
            });
            if short {
                return Ok(None);
            }
            still.retain(|_, wanted| wanted.count > 0);
            growth = growth.saturating_mul(SELECTION_GROWTH);
        }
        Ok(Some(refunds))
    }

    pub async fn reserved(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Vec<TreeNode>, LeafStoreError> {
        let transfer = transfer_uuid(transfer_id)?;
        let mut db = self.pool.acquire().await?;
        rows::leaves(&mut db, &self.identity, Which::Transfer(transfer)).await
    }

    pub async fn reservations(&self) -> Result<Vec<Reservation>, LeafStoreError> {
        let rows: Vec<(Uuid, i64, i64, DateTime<Utc>)> = sqlx::query_as(
            r"SELECT r.transfer_id, COUNT(*), SUM(l.value)::bigint, MIN(r.reserved_at)
               FROM brz_ssp_spark_reservations r
               JOIN brz_ssp_spark_leaves l ON l.refund_tx_id = r.refund_tx_id
               GROUP BY r.transfer_id
               ORDER BY MIN(r.reserved_at), r.transfer_id",
        )
        .fetch_all(&*self.pool)
        .await?;
        rows.into_iter()
            .map(|(transfer, leaves, value, reserved_at)| {
                Ok(Reservation {
                    transfer_id: transfer
                        .to_string()
                        .parse()
                        .map_err(|e| rows::invalid(format!("transfer id {transfer}: {e}")))?,
                    leaves: u32::try_from(leaves).unwrap_or(u32::MAX),
                    value: value.cast_unsigned(),
                    reserved_at,
                })
            })
            .collect()
    }

    pub async fn release(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Vec<TreeNodeId>, LeafStoreError> {
        let released: Vec<(Uuid, String)> = sqlx::query_as(
            r"DELETE FROM brz_ssp_spark_reservations r
               USING brz_ssp_spark_leaves l
               WHERE l.refund_tx_id = r.refund_tx_id AND r.transfer_id = $1
               RETURNING l.node_id, l.refund_tx_id",
        )
        .bind(transfer_uuid(transfer_id)?)
        .fetch_all(&*self.pool)
        .await?;
        released
            .into_iter()
            .map(|(leaf_id, refund_txid)| {
                info!(%leaf_id, %transfer_id, %refund_txid, "released a pool leaf a transfer had reserved");
                rows::node_id(leaf_id)
            })
            .collect()
    }

    /// The leaves reserved for `transfer_id` are no longer the SSP's once the
    /// operators have applied the transfer's key tweak.
    pub async fn transferred(&self, transfer_id: &TransferId) -> Result<(), LeafStoreError> {
        let transferred: Vec<(Uuid, String)> = sqlx::query_as(
            r"WITH transferred AS (
                   DELETE FROM brz_ssp_spark_leaves l
                   USING brz_ssp_spark_reservations reservation
                   WHERE reservation.refund_tx_id = l.refund_tx_id
                     AND reservation.transfer_id = $1
                   RETURNING l.refund_tx_id, l.node_id
               )
               INSERT INTO brz_ssp_spark_transferred_leaves (refund_tx_id, node_id, transfer_id)
               SELECT refund_tx_id, node_id, $1 FROM transferred
               RETURNING node_id, refund_tx_id",
        )
        .bind(transfer_uuid(transfer_id)?)
        .fetch_all(&*self.pool)
        .await?;
        for (leaf_id, refund_txid) in transferred {
            info!(%leaf_id, %transfer_id, %refund_txid, "transferred a pool leaf");
        }
        Ok(())
    }

    pub async fn counts(&self) -> Result<PoolCounts, LeafStoreError> {
        let (spent, unchecked) = self.chain.spent_and_unchecked_outputs().await?;
        let (spent_ids, spent_indices) = ids(&spent);
        let (unchecked_ids, unchecked_indices) = ids(&unchecked);
        // For a leaf that is both exiting and unchecked, MIN(how) is 'exiting',
        // the lower of the two strings.
        let rows: Vec<(i64, String, i64)> = sqlx::query_as(&format!(
            r"WITH spent_outputs (tx_id, output_index) AS (
                   SELECT * FROM UNNEST($2::text[], $3::int[])
               ), unchecked_outputs (tx_id, output_index) AS (
                   SELECT * FROM UNNEST($4::text[], $5::int[])
               )
               SELECT l.value, 'total', COUNT(*)
               FROM brz_ssp_spark_leaves l
               WHERE {counted}
               GROUP BY l.value
             UNION ALL
               SELECT l.value, unavailable.how, COUNT(*)
               FROM (SELECT refund_tx_id, MIN(how) AS how
                     FROM (SELECT refund_tx_id, 'exiting' AS how FROM ({exiting}) exiting
                         UNION ALL
                           SELECT refund_tx_id, 'unchecked' FROM ({unchecked}) unchecked) either
                     GROUP BY refund_tx_id) unavailable
               JOIN brz_ssp_spark_leaves l ON l.refund_tx_id = unavailable.refund_tx_id
               WHERE {counted}
               GROUP BY l.value, unavailable.how",
            counted = format_args!(
                "{UNRESERVED} AND {OPERATORS_AGREE} AND l.refund_timelock > {RENEW_TIMELOCK_THRESHOLD}"
            ),
            exiting = refunds_spending("spent_outputs"),
            unchecked = refunds_spending("unchecked_outputs"),
        ))
        .bind(self.identity.serialize().as_slice())
        .bind(&spent_ids)
        .bind(&spent_indices)
        .bind(&unchecked_ids)
        .bind(&unchecked_indices)
        .fetch_all(&*self.pool)
        .await?;
        let mut counts = PoolCounts::default();
        let mut unavailable: HashMap<u64, u32> = HashMap::new();
        for (value, how, count) in rows {
            let value = value.cast_unsigned();
            let count = u32::try_from(count).unwrap_or(u32::MAX);
            if how == "total" {
                counts.available.insert(value, count);
                continue;
            }
            if how == "unchecked" {
                counts.unchecked.insert(value, count);
            }
            let unavailable = unavailable.entry(value).or_default();
            *unavailable = unavailable.saturating_add(count);
        }
        for (value, unavailable) in unavailable {
            if let Some(available) = counts.available.get_mut(&value) {
                *available = available.saturating_sub(unavailable);
            }
        }
        counts.available.retain(|_, count| *count > 0);
        Ok(counts)
    }

    pub async fn leaves(&self) -> Result<Vec<TreeNode>, LeafStoreError> {
        let mut db = self.pool.acquire().await?;
        rows::leaves(&mut db, &self.identity, Which::All).await
    }

    pub async fn node_ids(
        &self,
        after: Option<&TreeNodeId>,
        limit: usize,
    ) -> Result<Vec<TreeNodeId>, LeafStoreError> {
        let after = after.map(node_uuid).transpose()?.unwrap_or(Uuid::nil());
        let ids: Vec<(Uuid,)> = sqlx::query_as(
            r"SELECT node_id FROM brz_ssp_spark_leaves
               WHERE node_id > $1 ORDER BY node_id LIMIT $2",
        )
        .bind(after)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&*self.pool)
        .await?;
        ids.into_iter().map(|(id,)| rows::node_id(id)).collect()
    }

    pub async fn pedigrees(
        &self,
        node_ids: &[TreeNodeId],
    ) -> Result<Vec<LeafPedigree>, LeafStoreError> {
        let ids = node_ids
            .iter()
            .map(node_uuid)
            .collect::<Result<Vec<Uuid>, _>>()?;
        let mut db = self.pool.acquire().await?;
        self.pedigrees_of(&mut db, &ids).await
    }

    async fn pedigrees_of(
        &self,
        db: &mut PgConnection,
        ids: &[Uuid],
    ) -> Result<Vec<LeafPedigree>, LeafStoreError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let leaves = rows::leaves(db, &self.identity, Which::Nodes(ids)).await?;
        let mut ancestors = rows::ancestors(db, ids).await?;
        Ok(leaves
            .into_iter()
            .map(|leaf| LeafPedigree {
                ancestors: ancestors.remove(&leaf.id).unwrap_or_default(),
                leaf,
            })
            .collect())
    }

    /// The SSP's leaves whose stored exit chain ends below the root, because
    /// the store does not have the parent of a node on it. The SSP stores each
    /// leaf with its whole exit chain, so there should be none.
    pub async fn leaves_missing_exit_chains(&self) -> Result<Vec<TreeNodeId>, LeafStoreError> {
        // To go through each node only once, however many leaves share it,
        // Postgres goes up from all the leaves together, and then back down
        // from each node whose parent the store does not have.
        let ids: Vec<(Uuid,)> = sqlx::query_as(
            r"WITH RECURSIVE above AS (
                   SELECT node.node_id, node.parent_node_id, node.node_tx_id
                   FROM brz_ssp_spark_leaves l
                   JOIN brz_ssp_spark_nodes node ON node.node_id = l.node_id
                 UNION
                   SELECT parent.node_id, parent.parent_node_id, parent.node_tx_id
                   FROM above
                   JOIN brz_ssp_spark_txs spending
                     ON spending.tx_id = above.node_tx_id
                   JOIN brz_ssp_spark_nodes parent
                     ON parent.node_tx_id = spending.previous_tx_id
               ), below AS (
                   SELECT above.node_id, above.node_tx_id
                   FROM above
                   JOIN brz_ssp_spark_txs spending
                     ON spending.tx_id = above.node_tx_id
                   WHERE above.parent_node_id IS NOT NULL
                     AND NOT EXISTS (
                         SELECT 1 FROM brz_ssp_spark_nodes parent
                         WHERE parent.node_tx_id = spending.previous_tx_id)
                 UNION
                   SELECT child.node_id, child.node_tx_id
                   FROM below
                   JOIN brz_ssp_spark_txs spending
                     ON spending.previous_tx_id = below.node_tx_id
                   JOIN brz_ssp_spark_nodes child
                     ON child.node_tx_id = spending.tx_id
               )
               SELECT l.node_id
               FROM brz_ssp_spark_leaves l
               JOIN below ON below.node_id = l.node_id",
        )
        .fetch_all(&*self.pool)
        .await?;
        ids.into_iter().map(|(id,)| rows::node_id(id)).collect()
    }
}

fn ids(outpoints: &[OutPoint]) -> (Vec<String>, Vec<i32>) {
    outpoints
        .iter()
        .map(|outpoint| (outpoint.txid.to_string(), outpoint.vout.cast_signed()))
        .unzip()
}

fn refund_txid(leaf: &TreeNode) -> String {
    leaf.refund_tx
        .as_ref()
        .map(|refund| refund.compute_txid().to_string())
        .unwrap_or_default()
}

fn is_foreign_key_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_foreign_key_violation)
}
