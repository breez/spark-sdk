//! `MySQL`-backed implementation of the `TreeStore` trait.
//!
//! Direct port of `crates/spark-postgres/src/tree_store.rs`. SQL syntax
//! differences vs. `PostgreSQL`:
//!
//! - `JSONB` → `JSON`
//! - `TIMESTAMPTZ NOT NULL DEFAULT NOW()` → `DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)`
//! - `TEXT PRIMARY KEY` → `VARCHAR(255) PRIMARY KEY` (TEXT cannot be a primary key in `MySQL` without prefix length)
//! - `ON CONFLICT (id) DO UPDATE SET … = EXCLUDED.…` → `ON DUPLICATE KEY UPDATE … = VALUES(…)`
//! - `ON CONFLICT DO NOTHING` → `INSERT IGNORE`
//! - `pg_advisory_xact_lock(key)` → `GET_LOCK('tree_store_write_lock', timeout)` with explicit `RELEASE_LOCK`
//! - `$N` positional placeholders → `?` placeholders
//! - `UNNEST(...)` batch inserts → manually built `VALUES (?,?,…), (?,?,…), …`
//! - `ANY($1)` IN-array predicates → manually built `IN (?, ?, …)`
//! - `make_interval(secs => $1)` → `INTERVAL ? SECOND_MICROSECOND`
//! - Partial indexes (`WHERE …`) are dropped (`MySQL` does not support them); the
//!   selectivity is acceptable on full indexes for our workload.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, NaiveDateTime, Utc};
use macros::async_trait;
use mysql_async::prelude::*;
use mysql_async::{Conn, Params, Pool, TxOpts, Value};
use platform_utils::time::{Instant, SystemTime};
use spark_wallet::{
    LeafLike, Leaves, LeavesReservation, LeavesReservationId, ReservationPurpose, ReserveResult,
    TargetAmounts, TreeNode, TreeNodeStatus, TreeServiceError, TreeStore,
    select_leaves_by_minimum_amount, select_leaves_by_target_amounts,
};
use tokio::sync::watch;
use tracing::{debug, info, trace};
use uuid::Uuid;

use crate::config::MysqlStorageConfig;
use crate::error::MysqlError;
use crate::migrations::run_migrations;
use crate::pool::create_pool;

/// Name of the schema migrations table for `MysqlTreeStore`.
const TREE_MIGRATIONS_TABLE: &str = "tree_schema_migrations";

/// Named lock used to serialize tree store write operations across connections.
/// `MySQL` `GET_LOCK` is session-scoped, so we acquire it at the start of each
/// write and release it explicitly before returning.
const TREE_STORE_WRITE_LOCK_NAME: &str = "tree_store_write_lock";

/// Timeout (seconds) when waiting on the write lock. Long enough to outlast
/// brief contention, short enough to surface true deadlocks instead of hanging.
const WRITE_LOCK_TIMEOUT_SECS: i64 = 30;

/// Reservations older than this (seconds) are considered stale and dropped at
/// the start of `set_leaves` to release leaves locked by crashed clients.
const RESERVATION_TIMEOUT_SECS: i64 = 300; // 5 minutes

/// Spent markers older than this (milliseconds, relative to refresh timestamp)
/// are deleted during `set_leaves`. Kept long enough to support multi-instance
/// deployments where another instance may still be processing a refresh.
const SPENT_MARKER_CLEANUP_THRESHOLD_MS: i64 = 5 * 60 * 1000;

/// Lightweight `(id, value)` pair used by `try_reserve_leaves` to run the
/// selection algorithm without pulling each leaf's full `data` JSON.
#[derive(Clone)]
struct SlimLeaf {
    id: String,
    value: u64,
}

impl LeafLike for SlimLeaf {
    type Id = String;
    fn leaf_id(&self) -> &Self::Id {
        &self.id
    }
    fn leaf_value(&self) -> u64 {
        self.value
    }
}

/// `MySQL`-backed tree store implementation.
///
/// Uses an application-level named lock to serialize writes (`GET_LOCK`) and
/// row-level FK constraints to keep reservations and leaves in sync.
pub struct MysqlTreeStore {
    pool: Pool,
    balance_changed_tx: Arc<watch::Sender<()>>,
    balance_changed_rx: watch::Receiver<()>,
}

#[async_trait]
impl TreeStore for MysqlTreeStore {
    async fn add_leaves(&self, leaves: &[TreeNode]) -> Result<(), TreeServiceError> {
        if leaves.is_empty() {
            return Ok(());
        }

        for leaf in leaves {
            trace!(
                "leaf_lifecycle add_leaves: leaf={} value={}",
                leaf.id, leaf.value
            );
        }

        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        // No global write lock: `add_leaves` is scoped to inserting/updating
        // a known set of leaf rows; row-level locks + InnoDB MVCC are
        // sufficient. Mirrors the postgres impl's lock-removal change.
        Self::add_leaves_inner(&mut conn, leaves).await?;
        self.notify_balance_change();
        Ok(())
    }

    async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        // Server-side aggregation: counts the same set as `Leaves::balance()`
        // (available + missing-from-operators is excluded; swap-reserved is
        // included). Avoids fetching every leaf's `data` JSON when callers
        // only need the spendable total.
        let row: Option<i64> = conn
            .query_first(
                r"SELECT COALESCE(SUM(l.value), 0) AS balance
                  FROM tree_leaves l
                  LEFT JOIN tree_reservations r ON l.reservation_id = r.id
                  WHERE
                      (l.reservation_id IS NULL AND l.status = 'Available')
                      OR r.purpose = 'Swap'",
            )
            .await
            .map_err(map_err)?;
        Ok(u64::try_from(row.unwrap_or(0)).unwrap_or(0))
    }

    async fn get_leaves(&self) -> Result<Leaves, TreeServiceError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;

        let rows: Vec<(String, String, bool, String, Option<String>, Option<String>)> = conn
            .query(
                r"SELECT l.id, l.status, l.is_missing_from_operators, l.data,
                         l.reservation_id, r.purpose
                  FROM tree_leaves l
                  LEFT JOIN tree_reservations r ON l.reservation_id = r.id",
            )
            .await
            .map_err(map_err)?;

        let mut available = Vec::new();
        let mut not_available = Vec::new();
        let mut available_missing_from_operators = Vec::new();
        let mut reserved_for_payment = Vec::new();
        let mut reserved_for_swap = Vec::new();

        for (_id, _status, is_missing, data_str, _reservation_id, purpose) in rows {
            let node = Self::deserialize_node(&data_str)?;

            if let Some(purpose_str) = purpose {
                match purpose_str
                    .parse::<ReservationPurpose>()
                    .map_err(TreeServiceError::Generic)?
                {
                    ReservationPurpose::Payment => reserved_for_payment.push(node),
                    ReservationPurpose::Swap => reserved_for_swap.push(node),
                }
            } else if is_missing {
                if node.status == TreeNodeStatus::Available {
                    available_missing_from_operators.push(node);
                }
            } else if node.status == TreeNodeStatus::Available {
                available.push(node);
            } else {
                not_available.push(node);
            }
        }

        Ok(Leaves {
            available,
            not_available,
            available_missing_from_operators,
            reserved_for_payment,
            reserved_for_swap,
        })
    }

    async fn set_leaves(
        &self,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
        refresh_started_at: SystemTime,
    ) -> Result<(), TreeServiceError> {
        let refresh_timestamp: DateTime<Utc> = refresh_started_at.into();

        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        Self::acquire_write_lock(&mut conn).await?;
        let result = Self::set_leaves_inner(
            &mut conn,
            leaves,
            missing_operators_leaves,
            refresh_timestamp,
        )
        .await;
        Self::release_write_lock_quiet(&mut conn).await;
        result?;
        self.notify_balance_change();
        Ok(())
    }

    async fn cancel_reservation(
        &self,
        id: &LeavesReservationId,
        leaves_to_keep: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        // Scoped to a single `reservation_id`; row-level FK + MVCC suffice.
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        Self::cancel_reservation_inner(&mut conn, id, leaves_to_keep).await?;
        self.notify_balance_change();
        Ok(())
    }

    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        // Scoped to a single `reservation_id`; row-level FK + MVCC suffice.
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        Self::finalize_reservation_inner(&mut conn, id, new_leaves).await?;
        trace!("Finalized reservation: {id}");
        self.notify_balance_change();
        Ok(())
    }

    #[allow(clippy::arithmetic_side_effects, clippy::too_many_lines)]
    async fn try_reserve_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<ReserveResult, TreeServiceError> {
        let target_amount = target_amounts.map_or(0, TargetAmounts::total_sats);
        let reservation_id = Uuid::now_v7().to_string();

        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        Self::acquire_write_lock(&mut conn).await?;

        let result = Self::try_reserve_leaves_inner(
            &mut conn,
            &reservation_id,
            target_amounts,
            target_amount,
            exact_only,
            purpose,
        )
        .await;
        Self::release_write_lock_quiet(&mut conn).await;
        let reserve_result = result?;
        if matches!(reserve_result, ReserveResult::Success(_)) {
            self.notify_balance_change();
        }
        Ok(reserve_result)
    }

    async fn now(&self) -> Result<SystemTime, TreeServiceError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        let row: Option<NaiveDateTime> =
            conn.query_first("SELECT NOW(6)").await.map_err(map_err)?;
        let now = row.ok_or_else(|| TreeServiceError::Generic("NOW() returned no row".into()))?;
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(now, Utc);
        Ok(dt.into())
    }

    fn subscribe_balance_changes(&self) -> watch::Receiver<()> {
        self.balance_changed_rx.clone()
    }

    async fn update_reservation(
        &self,
        reservation_id: &LeavesReservationId,
        reserved_leaves: &[TreeNode],
        change_leaves: &[TreeNode],
    ) -> Result<LeavesReservation, TreeServiceError> {
        // Scoped to a single `reservation_id`; row-level FK + MVCC suffice.
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        let reservation = Self::update_reservation_inner(
            &mut conn,
            reservation_id,
            reserved_leaves,
            change_leaves,
        )
        .await?;
        trace!(
            "Updated reservation {}: reserved {} leaves, added {} change leaves",
            reservation_id,
            reserved_leaves.len(),
            change_leaves.len()
        );
        self.notify_balance_change();
        Ok(reservation)
    }
}

impl MysqlTreeStore {
    /// Creates a new `MysqlTreeStore` from a configuration.
    pub async fn from_config(config: MysqlStorageConfig) -> Result<Self, MysqlError> {
        let pool = create_pool(&config)?;
        Self::init(pool).await
    }

    /// Creates a new `MysqlTreeStore` from an existing connection pool.
    pub async fn from_pool(pool: Pool) -> Result<Self, MysqlError> {
        Self::init(pool).await
    }

    async fn init(pool: Pool) -> Result<Self, MysqlError> {
        let (balance_changed_tx, balance_changed_rx) = watch::channel(());

        let store = Self {
            pool,
            balance_changed_tx: Arc::new(balance_changed_tx),
            balance_changed_rx,
        };

        store.migrate().await?;
        store.notify_balance_change();

        Ok(store)
    }

    async fn migrate(&self) -> Result<(), MysqlError> {
        run_migrations(&self.pool, TREE_MIGRATIONS_TABLE, &Self::migrations()).await
    }

    fn migrations() -> Vec<&'static [&'static str]> {
        vec![
            // Migration 1: Initial tree tables.
            //
            // Reservations are referenced via FK so that ON DELETE SET NULL
            // releases the leaves automatically when a reservation is dropped.
            &[
                "CREATE TABLE IF NOT EXISTS tree_reservations (
                    id VARCHAR(255) NOT NULL PRIMARY KEY,
                    purpose VARCHAR(64) NOT NULL,
                    pending_change_amount BIGINT NOT NULL DEFAULT 0,
                    created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
                )",
                "CREATE TABLE IF NOT EXISTS tree_leaves (
                    id VARCHAR(255) NOT NULL PRIMARY KEY,
                    status VARCHAR(64) NOT NULL,
                    is_missing_from_operators TINYINT(1) NOT NULL DEFAULT 0,
                    reservation_id VARCHAR(255) NULL,
                    data JSON NOT NULL,
                    created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
                    added_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
                    CONSTRAINT fk_tree_leaves_reservation FOREIGN KEY (reservation_id)
                        REFERENCES tree_reservations(id) ON DELETE SET NULL
                )",
                "CREATE TABLE IF NOT EXISTS tree_spent_leaves (
                    leaf_id VARCHAR(255) NOT NULL PRIMARY KEY,
                    spent_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
                )",
                "CREATE INDEX idx_tree_leaves_available
                    ON tree_leaves(status, is_missing_from_operators)",
                "CREATE INDEX idx_tree_leaves_reservation ON tree_leaves(reservation_id)",
                "CREATE INDEX idx_tree_leaves_added_at ON tree_leaves(added_at)",
            ],
            // Migration 2: Swap status tracking.
            &[
                "CREATE TABLE IF NOT EXISTS tree_swap_status (
                    id INT NOT NULL PRIMARY KEY DEFAULT 1,
                    last_completed_at DATETIME(6) NULL,
                    CHECK (id = 1)
                )",
                "INSERT IGNORE INTO tree_swap_status (id) VALUES (1)",
            ],
            // Migration 3: Promote `value` out of the JSON `data` column into a
            // dedicated BIGINT. JSON_EXTRACT/JSON_UNQUOTE on every reservation
            // and balance query was the dominant cost vs. postgres's
            // `(data->>'value')::bigint` expression. Also adds a composite index
            // `(status, is_missing_from_operators, reservation_id, value)` so
            // the slim selection in `try_reserve_leaves` is index-only.
            &[
                "ALTER TABLE tree_leaves
                    ADD COLUMN value BIGINT NOT NULL DEFAULT 0",
                "UPDATE tree_leaves
                    SET value = CAST(JSON_UNQUOTE(JSON_EXTRACT(data, '$.value')) AS UNSIGNED)
                    WHERE value = 0",
                "CREATE INDEX idx_tree_leaves_slim
                    ON tree_leaves(status, is_missing_from_operators, reservation_id, value)",
            ],
        ]
    }

    fn notify_balance_change(&self) {
        let _ = self.balance_changed_tx.send(());
    }

    /// Acquires the write lock for this connection. Held until `release_write_lock`
    /// is called or the connection is returned to the pool.
    async fn acquire_write_lock(conn: &mut Conn) -> Result<(), TreeServiceError> {
        let acquired: Option<i64> = conn
            .exec_first(
                "SELECT GET_LOCK(?, ?)",
                (TREE_STORE_WRITE_LOCK_NAME, WRITE_LOCK_TIMEOUT_SECS),
            )
            .await
            .map_err(map_err)?;
        if acquired != Some(1) {
            return Err(TreeServiceError::Generic(format!(
                "Failed to acquire tree store write lock within {WRITE_LOCK_TIMEOUT_SECS}s"
            )));
        }
        Ok(())
    }

    /// Releases the write lock, swallowing any error so it doesn't mask the
    /// caller's actual result.
    async fn release_write_lock_quiet(conn: &mut Conn) {
        let _ = conn
            .exec_drop("SELECT RELEASE_LOCK(?)", (TREE_STORE_WRITE_LOCK_NAME,))
            .await;
    }

    fn serialize_node(node: &TreeNode) -> Result<String, TreeServiceError> {
        serde_json::to_string(node)
            .map_err(|e| TreeServiceError::Generic(format!("Failed to serialize TreeNode: {e}")))
    }

    fn deserialize_node(data: &str) -> Result<TreeNode, TreeServiceError> {
        serde_json::from_str(data)
            .map_err(|e| TreeServiceError::Generic(format!("Failed to deserialize TreeNode: {e}")))
    }

    async fn add_leaves_inner(
        conn: &mut Conn,
        leaves: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let leaf_ids: Vec<String> = leaves.iter().map(|l| l.id.to_string()).collect();
        Self::batch_remove_spent_leaves(&mut tx, &leaf_ids).await?;
        Self::batch_upsert_leaves(&mut tx, leaves, false, None).await?;

        tx.commit().await.map_err(map_err)?;
        tracing::trace!(
            "MysqlTreeStore::add_leaves: committed {} leaves",
            leaves.len()
        );
        Ok(())
    }

    async fn set_leaves_inner(
        conn: &mut Conn,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
        refresh_timestamp: DateTime<Utc>,
    ) -> Result<(), TreeServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        Self::cleanup_stale_reservations(&mut tx).await?;

        // Check if any swap reservation is currently active, or if a swap completed
        // after this refresh started (making the refresh data potentially inconsistent).
        let row: Option<(i64, i64)> = tx
            .exec_first(
                r"SELECT
                    (SELECT EXISTS(SELECT 1 FROM tree_reservations WHERE purpose = 'Swap')) AS has_active_swap,
                    COALESCE(
                        (SELECT (last_completed_at >= ?) FROM tree_swap_status WHERE id = 1),
                        0
                    ) AS swap_completed_during_refresh",
                (refresh_timestamp.naive_utc(),),
            )
            .await
            .map_err(map_err)?;
        let (has_active_swap, swap_completed_during_refresh) = match row {
            Some((a, b)) => (a != 0, b != 0),
            None => (false, false),
        };

        if has_active_swap || swap_completed_during_refresh {
            info!(
                "leaf_lifecycle set_leaves: SKIP active_swap={} swap_completed_during_refresh={} refresh_timestamp={:?}",
                has_active_swap, swap_completed_during_refresh, refresh_timestamp
            );
            // Commit the cleanup that already ran.
            tx.commit().await.map_err(map_err)?;
            return Ok(());
        }

        Self::cleanup_spent_markers(&mut tx, refresh_timestamp).await?;

        let spent_rows: Vec<String> = tx
            .exec(
                "SELECT leaf_id FROM tree_spent_leaves WHERE spent_at >= ?",
                (refresh_timestamp.naive_utc(),),
            )
            .await
            .map_err(map_err)?;
        let spent_ids: HashSet<String> = spent_rows.into_iter().collect();
        info!(
            "leaf_lifecycle set_leaves: PROCEED refresh_timestamp={:?} active_spent_ids={} (ids={:?})",
            refresh_timestamp,
            spent_ids.len(),
            spent_ids
        );

        // Delete non-reserved leaves added before refresh started.
        tx.exec_drop(
            "DELETE FROM tree_leaves WHERE reservation_id IS NULL AND added_at < ?",
            (refresh_timestamp.naive_utc(),),
        )
        .await
        .map_err(map_err)?;

        Self::batch_upsert_leaves(&mut tx, leaves, false, Some(&spent_ids)).await?;
        Self::batch_upsert_leaves(&mut tx, missing_operators_leaves, true, Some(&spent_ids))
            .await?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn cancel_reservation_inner(
        conn: &mut Conn,
        id: &LeavesReservationId,
        leaves_to_keep: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let exists: Option<String> = tx
            .exec_first("SELECT id FROM tree_reservations WHERE id = ?", (id,))
            .await
            .map_err(map_err)?;
        if exists.is_none() {
            tx.commit().await.map_err(map_err)?;
            return Ok(());
        }

        let prior_leaf_ids: Vec<String> = tx
            .exec("SELECT id FROM tree_leaves WHERE reservation_id = ?", (id,))
            .await
            .map_err(map_err)?;
        let keep_ids: Vec<String> = leaves_to_keep.iter().map(|l| l.id.to_string()).collect();
        let dropped_ids: Vec<&String> = prior_leaf_ids
            .iter()
            .filter(|id| !keep_ids.contains(id))
            .collect();
        info!(
            "leaf_lifecycle cancel: reservation={} prior_leaves={:?} keeping={:?} dropping={:?}",
            id, prior_leaf_ids, keep_ids, dropped_ids
        );

        tx.exec_drop("DELETE FROM tree_leaves WHERE reservation_id = ?", (id,))
            .await
            .map_err(map_err)?;

        tx.exec_drop("DELETE FROM tree_reservations WHERE id = ?", (id,))
            .await
            .map_err(map_err)?;

        Self::batch_upsert_leaves(&mut tx, leaves_to_keep, false, None).await?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn finalize_reservation_inner(
        conn: &mut Conn,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let purpose: Option<String> = tx
            .exec_first("SELECT purpose FROM tree_reservations WHERE id = ?", (id,))
            .await
            .map_err(map_err)?;

        let (is_swap, reserved_leaf_ids) = if let Some(purpose) = purpose {
            let is_swap = purpose == "Swap";
            let leaf_ids: Vec<String> = tx
                .exec("SELECT id FROM tree_leaves WHERE reservation_id = ?", (id,))
                .await
                .map_err(map_err)?;
            (is_swap, leaf_ids)
        } else {
            (false, Vec::new())
        };

        info!(
            "leaf_lifecycle finalize: reservation={} is_swap={} marking_spent={:?} new_leaves={}",
            id,
            is_swap,
            reserved_leaf_ids,
            new_leaves.map_or(0, <[TreeNode]>::len)
        );
        Self::batch_insert_spent_leaves(&mut tx, &reserved_leaf_ids).await?;

        tx.exec_drop("DELETE FROM tree_leaves WHERE reservation_id = ?", (id,))
            .await
            .map_err(map_err)?;

        tx.exec_drop("DELETE FROM tree_reservations WHERE id = ?", (id,))
            .await
            .map_err(map_err)?;

        if let Some(leaves) = new_leaves {
            for l in leaves {
                trace!(
                    "leaf_lifecycle finalize: adding new leaf={} value={} reservation={}",
                    l.id, l.value, id
                );
            }
            Self::batch_upsert_leaves(&mut tx, leaves, false, None).await?;
        }

        if is_swap && new_leaves.is_some() {
            tx.query_drop("UPDATE tree_swap_status SET last_completed_at = NOW(6) WHERE id = 1")
                .await
                .map_err(map_err)?;
        }

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    #[allow(clippy::arithmetic_side_effects, clippy::too_many_lines)]
    async fn try_reserve_leaves_inner(
        conn: &mut Conn,
        reservation_id: &str,
        target_amounts: Option<&TargetAmounts>,
        target_amount: u64,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<ReserveResult, TreeServiceError> {
        let total_start = Instant::now();
        let max_target = Self::slim_max_target(target_amounts);
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        // True total available across ALL eligible leaves — required for the
        // WaitForPending decision. Must NOT be derived from the prefiltered
        // slim set since the prefilter excludes big leaves.
        let total_row: Option<i64> = tx
            .query_first(
                r"SELECT COALESCE(SUM(value), 0) AS total
                  FROM tree_leaves
                  WHERE status = 'Available'
                    AND is_missing_from_operators = 0
                    AND reservation_id IS NULL",
            )
            .await
            .map_err(map_err)?;
        let available: u64 = u64::try_from(total_row.unwrap_or(0)).unwrap_or(0);

        // Slim projection of selection candidates: id + value only.
        // Includes all leaves with value <= max_target (covers exact-match +
        // minimum-amount accumulators) plus the smallest leaf with value >
        // max_target (covers the minimum-amount fallback case where one larger
        // leaf is sufficient).
        let max_target_signed: i64 = i64::try_from(max_target).unwrap_or(i64::MAX);
        let slim_rows: Vec<(String, i64)> = tx
            .exec(
                r"SELECT id, value
                  FROM tree_leaves
                  WHERE status = 'Available'
                    AND is_missing_from_operators = 0
                    AND reservation_id IS NULL
                    AND (
                      value <= ?
                      OR id = (
                        SELECT id FROM (
                          SELECT id FROM tree_leaves
                          WHERE status = 'Available'
                            AND is_missing_from_operators = 0
                            AND reservation_id IS NULL
                            AND value > ?
                          ORDER BY value
                          LIMIT 1
                        ) AS smallest_over
                      )
                    )",
                (max_target_signed, max_target_signed),
            )
            .await
            .map_err(map_err)?;

        let slim: Vec<SlimLeaf> = slim_rows
            .into_iter()
            .map(|(id, value)| SlimLeaf {
                id,
                value: u64::try_from(value).unwrap_or(0),
            })
            .collect();

        let pending = Self::calculate_pending_balance(&mut tx).await?;

        // Try exact selection on the slim set — uses the same generic
        // `select_helper` algorithm as the in-memory store.
        let selected_exact = select_leaves_by_target_amounts(&slim, target_amounts);

        let result = match selected_exact {
            Ok(target_leaves) => {
                let selected_ids: Vec<String> = target_leaves
                    .amount_leaves
                    .iter()
                    .chain(target_leaves.fee_leaves.iter().flatten())
                    .map(|l| l.id.clone())
                    .collect();
                if selected_ids.is_empty() {
                    return Err(TreeServiceError::NonReservableLeaves);
                }
                let selected_leaves = Self::resolve_full_leaves(&mut tx, &selected_ids).await?;
                Self::create_reservation(&mut tx, reservation_id, &selected_leaves, purpose, 0)
                    .await?;
                tx.commit().await.map_err(map_err)?;
                Ok(ReserveResult::Success(LeavesReservation::new(
                    selected_leaves,
                    reservation_id.to_string(),
                )))
            }
            Err(_) if !exact_only => {
                if let Ok(Some(min_slim)) = select_leaves_by_minimum_amount(&slim, target_amount) {
                    let min_ids: Vec<String> = min_slim.iter().map(|l| l.id.clone()).collect();
                    let selected_leaves = Self::resolve_full_leaves(&mut tx, &min_ids).await?;
                    let reserved_amount: u64 = selected_leaves.iter().map(|l| l.value).sum();
                    let pending_change = if reserved_amount > target_amount && target_amount > 0 {
                        reserved_amount - target_amount
                    } else {
                        0
                    };
                    Self::create_reservation(
                        &mut tx,
                        reservation_id,
                        &selected_leaves,
                        purpose,
                        pending_change,
                    )
                    .await?;
                    tx.commit().await.map_err(map_err)?;
                    Ok(ReserveResult::Success(LeavesReservation::new(
                        selected_leaves,
                        reservation_id.to_string(),
                    )))
                } else if available + pending >= target_amount {
                    tx.commit().await.map_err(map_err)?;
                    Ok(ReserveResult::WaitForPending {
                        needed: target_amount,
                        available,
                        pending,
                    })
                } else {
                    tx.commit().await.map_err(map_err)?;
                    Ok(ReserveResult::InsufficientFunds)
                }
            }
            Err(_) => {
                tx.commit().await.map_err(map_err)?;
                if available + pending >= target_amount {
                    Ok(ReserveResult::WaitForPending {
                        needed: target_amount,
                        available,
                        pending,
                    })
                } else {
                    Ok(ReserveResult::InsufficientFunds)
                }
            }
        };

        let outcome = match &result {
            Ok(ReserveResult::Success(r)) => format!("success(leaves={})", r.leaves.len()),
            Ok(ReserveResult::WaitForPending { .. }) => "waitForPending".to_string(),
            Ok(ReserveResult::InsufficientFunds) => "insufficientFunds".to_string(),
            Err(e) => format!("err({e:?})"),
        };
        info!(
            "MysqlTreeStore::try_reserve_leaves: {} (slim_candidates={}, max_target={}, exact_only={}, took {:?})",
            outcome,
            slim.len(),
            max_target,
            exact_only,
            total_start.elapsed()
        );
        result
    }

    /// Largest single leaf value the selection algorithm could possibly need.
    /// Used to bound the slim projection in `try_reserve_leaves`. For an
    /// unbounded request we have to keep all leaves available.
    fn slim_max_target(target_amounts: Option<&TargetAmounts>) -> u64 {
        match target_amounts {
            Some(TargetAmounts::AmountAndFee {
                amount_sats,
                fee_sats,
            }) => std::cmp::max(*amount_sats, fee_sats.unwrap_or(0)),
            Some(TargetAmounts::ExactDenominations { denominations }) => {
                denominations.iter().copied().max().unwrap_or(0)
            }
            None => u64::MAX,
        }
    }

    /// Pull the full `TreeNode` JSON only for the leaves the slim selection
    /// picked, preserving the algorithm's selection order. Typically 1-3 rows
    /// even when the slim candidate set was thousands.
    async fn resolve_full_leaves(
        tx: &mut mysql_async::Transaction<'_>,
        ids: &[String],
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = build_placeholders(ids.len());
        let sql = format!("SELECT id, data FROM tree_leaves WHERE id IN ({placeholders})");
        let params: Vec<Value> = ids.iter().cloned().map(Value::from).collect();
        let rows: Vec<(String, String)> = tx
            .exec(&sql, Params::Positional(params))
            .await
            .map_err(map_err)?;
        let mut by_id: HashMap<String, TreeNode> = HashMap::with_capacity(rows.len());
        for (id, data) in rows {
            let node = Self::deserialize_node(&data)?;
            by_id.insert(id, node);
        }
        let ordered: Vec<TreeNode> = ids.iter().filter_map(|id| by_id.remove(id)).collect();
        if ordered.len() != ids.len() {
            return Err(TreeServiceError::Generic(format!(
                "Could not resolve full data for all selected leaves (wanted {}, got {})",
                ids.len(),
                ordered.len()
            )));
        }
        Ok(ordered)
    }

    async fn update_reservation_inner(
        conn: &mut Conn,
        reservation_id: &LeavesReservationId,
        reserved_leaves: &[TreeNode],
        change_leaves: &[TreeNode],
    ) -> Result<LeavesReservation, TreeServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let exists: Option<String> = tx
            .exec_first(
                "SELECT id FROM tree_reservations WHERE id = ?",
                (reservation_id,),
            )
            .await
            .map_err(map_err)?;

        if exists.is_none() {
            return Err(TreeServiceError::Generic(format!(
                "Reservation {reservation_id} not found"
            )));
        }

        let old_reserved_leaf_ids: Vec<String> = tx
            .exec(
                "SELECT id FROM tree_leaves WHERE reservation_id = ?",
                (reservation_id,),
            )
            .await
            .map_err(map_err)?;

        Self::batch_insert_spent_leaves(&mut tx, &old_reserved_leaf_ids).await?;
        tx.exec_drop(
            "DELETE FROM tree_leaves WHERE reservation_id = ?",
            (reservation_id,),
        )
        .await
        .map_err(map_err)?;

        Self::batch_upsert_leaves(&mut tx, change_leaves, false, None).await?;
        Self::batch_upsert_leaves(&mut tx, reserved_leaves, false, None).await?;

        let leaf_ids: Vec<String> = reserved_leaves.iter().map(|l| l.id.to_string()).collect();
        Self::batch_set_reservation_id(&mut tx, reservation_id, &leaf_ids).await?;

        tx.exec_drop(
            "UPDATE tree_reservations SET pending_change_amount = 0 WHERE id = ?",
            (reservation_id,),
        )
        .await
        .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;

        Ok(LeavesReservation::new(
            reserved_leaves.to_vec(),
            reservation_id.clone(),
        ))
    }

    async fn calculate_pending_balance(
        tx: &mut mysql_async::Transaction<'_>,
    ) -> Result<u64, TreeServiceError> {
        let row: Option<i64> = tx
            .query_first("SELECT COALESCE(SUM(pending_change_amount), 0) FROM tree_reservations")
            .await
            .map_err(map_err)?;

        Ok(u64::try_from(row.unwrap_or(0)).unwrap_or(0))
    }

    /// Batch upserts leaves into `tree_leaves` table.
    /// Optionally skips leaves whose IDs are in the `skip_ids` set.
    /// Uses `ON DUPLICATE KEY UPDATE` to replace existing leaves.
    #[allow(clippy::arithmetic_side_effects)] // `len * 4` for params capacity, bounded by leaves slice
    async fn batch_upsert_leaves(
        tx: &mut mysql_async::Transaction<'_>,
        leaves: &[TreeNode],
        is_missing_from_operators: bool,
        skip_ids: Option<&HashSet<String>>,
    ) -> Result<(), TreeServiceError> {
        let filtered: Vec<&TreeNode> = if let Some(skip) = skip_ids {
            let mut kept = Vec::new();
            for l in leaves {
                let id_str = l.id.to_string();
                if skip.contains(&id_str) {
                    trace!(
                        "leaf_lifecycle batch_upsert: skipped leaf={} (in spent_ids) is_missing_from_operators={}",
                        id_str, is_missing_from_operators
                    );
                } else {
                    kept.push(l);
                }
            }
            kept
        } else {
            leaves.iter().collect()
        };

        if filtered.is_empty() {
            return Ok(());
        }

        // Build VALUES (?, ?, ?, ?, ?, NOW(6)), …
        let mut sql = String::from(
            "INSERT INTO tree_leaves (id, status, is_missing_from_operators, data, value, added_at) VALUES ",
        );
        let mut params: Vec<Value> = Vec::with_capacity(filtered.len() * 5);
        for (i, leaf) in filtered.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str("(?, ?, ?, ?, ?, NOW(6))");
            #[allow(clippy::cast_possible_wrap)]
            let value_i64 = leaf.value as i64;
            params.push(Value::from(leaf.id.to_string()));
            params.push(Value::from(leaf.status.to_string()));
            params.push(Value::from(is_missing_from_operators));
            params.push(Value::from(Self::serialize_node(leaf)?));
            params.push(Value::from(value_i64));
        }
        sql.push_str(
            " ON DUPLICATE KEY UPDATE
                status = VALUES(status),
                is_missing_from_operators = VALUES(is_missing_from_operators),
                data = VALUES(data),
                value = VALUES(value),
                added_at = NOW(6)",
        );

        tx.exec_drop(&sql, Params::Positional(params))
            .await
            .map_err(map_err)?;

        Ok(())
    }

    #[allow(clippy::arithmetic_side_effects)] // `len + 1` for params capacity
    async fn batch_set_reservation_id(
        tx: &mut mysql_async::Transaction<'_>,
        reservation_id: &str,
        leaf_ids: &[String],
    ) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }

        let placeholders = build_placeholders(leaf_ids.len());
        let sql = format!("UPDATE tree_leaves SET reservation_id = ? WHERE id IN ({placeholders})");

        let mut params: Vec<Value> = Vec::with_capacity(leaf_ids.len() + 1);
        params.push(Value::from(reservation_id));
        for id in leaf_ids {
            params.push(Value::from(id.clone()));
        }

        tx.exec_drop(&sql, Params::Positional(params))
            .await
            .map_err(map_err)?;

        Ok(())
    }

    async fn batch_insert_spent_leaves(
        tx: &mut mysql_async::Transaction<'_>,
        leaf_ids: &[String],
    ) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }

        let mut sql = String::from("INSERT IGNORE INTO tree_spent_leaves (leaf_id) VALUES ");
        let mut params: Vec<Value> = Vec::with_capacity(leaf_ids.len());
        for (i, id) in leaf_ids.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str("(?)");
            params.push(Value::from(id.clone()));
        }

        tx.exec_drop(&sql, Params::Positional(params))
            .await
            .map_err(map_err)?;

        Ok(())
    }

    async fn batch_remove_spent_leaves(
        tx: &mut mysql_async::Transaction<'_>,
        leaf_ids: &[String],
    ) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }

        let placeholders = build_placeholders(leaf_ids.len());
        let sql = format!("DELETE FROM tree_spent_leaves WHERE leaf_id IN ({placeholders})");

        let params: Vec<Value> = leaf_ids.iter().cloned().map(Value::from).collect();
        let mut result = tx
            .exec_iter(&sql, Params::Positional(params))
            .await
            .map_err(map_err)?;
        let affected = result.affected_rows();
        // Drain and drop the result.
        let _: Vec<mysql_async::Row> = result.collect().await.map_err(map_err)?;

        if affected > 0 {
            trace!(
                "Removed {} leaves from spent_leaves (receiving them back)",
                affected
            );
        }

        Ok(())
    }

    async fn cleanup_stale_reservations(
        tx: &mut mysql_async::Transaction<'_>,
    ) -> Result<u64, TreeServiceError> {
        let mut result = tx
            .exec_iter(
                "DELETE FROM tree_reservations
                 WHERE created_at < DATE_SUB(NOW(6), INTERVAL ? SECOND)",
                (RESERVATION_TIMEOUT_SECS,),
            )
            .await
            .map_err(map_err)?;
        let affected = result.affected_rows();
        let _: Vec<mysql_async::Row> = result.collect().await.map_err(map_err)?;

        if affected > 0 {
            trace!("Cleaned up {} stale reservations", affected);
        }

        Ok(affected)
    }

    async fn cleanup_spent_markers(
        tx: &mut mysql_async::Transaction<'_>,
        refresh_timestamp: DateTime<Utc>,
    ) -> Result<u64, TreeServiceError> {
        let threshold = chrono::Duration::milliseconds(SPENT_MARKER_CLEANUP_THRESHOLD_MS);
        let cleanup_cutoff = refresh_timestamp
            .checked_sub_signed(threshold)
            .unwrap_or(refresh_timestamp);

        let mut result = tx
            .exec_iter(
                "DELETE FROM tree_spent_leaves WHERE spent_at < ?",
                (cleanup_cutoff.naive_utc(),),
            )
            .await
            .map_err(map_err)?;
        let affected = result.affected_rows();
        let _: Vec<mysql_async::Row> = result.collect().await.map_err(map_err)?;

        if affected > 0 {
            trace!("Cleaned up {} spent markers", affected);
        }

        Ok(affected)
    }

    async fn create_reservation(
        tx: &mut mysql_async::Transaction<'_>,
        reservation_id: &str,
        leaves: &[TreeNode],
        purpose: ReservationPurpose,
        pending_change: u64,
    ) -> Result<(), TreeServiceError> {
        #[allow(clippy::cast_possible_wrap)]
        let pending_i64 = pending_change as i64;

        tx.exec_drop(
            "INSERT INTO tree_reservations (id, purpose, pending_change_amount) VALUES (?, ?, ?)",
            (reservation_id, purpose.to_string(), pending_i64),
        )
        .await
        .map_err(map_err)?;

        let leaf_ids: Vec<String> = leaves.iter().map(|l| l.id.to_string()).collect();
        debug!(
            "leaf_lifecycle reserve: reservation={} purpose={:?} leaf_ids={:?}",
            reservation_id, purpose, leaf_ids
        );
        Self::batch_set_reservation_id(tx, reservation_id, &leaf_ids).await?;

        Ok(())
    }
}

/// Generates `?, ?, ?, …` for `n` placeholders.
fn build_placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n.saturating_mul(3));
    for i in 0..n {
        if i > 0 {
            s.push_str(", ");
        }
        s.push('?');
    }
    s
}

fn map_err<E: std::fmt::Display>(e: E) -> TreeServiceError {
    TreeServiceError::Generic(e.to_string())
}

/// Creates a `MysqlTreeStore` instance from a configuration.
pub async fn create_mysql_tree_store(
    config: MysqlStorageConfig,
) -> Result<Arc<dyn TreeStore>, MysqlError> {
    Ok(Arc::new(MysqlTreeStore::from_config(config).await?))
}

/// Creates a `MysqlTreeStore` instance from an existing connection pool.
pub async fn create_mysql_tree_store_from_pool(
    pool: Pool,
) -> Result<Arc<dyn TreeStore>, MysqlError> {
    Ok(Arc::new(MysqlTreeStore::from_pool(pool).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::tree_store_tests as shared_tests;
    use std::sync::Arc;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::mysql::Mysql;

    /// Helper struct that holds the container and store together.
    /// The container must be kept alive for the duration of the test.
    struct MysqlTreeStoreTestFixture {
        store: MysqlTreeStore,
        #[allow(dead_code)]
        container: ContainerAsync<Mysql>,
    }

    impl MysqlTreeStoreTestFixture {
        async fn new() -> Self {
            let container = Mysql::default()
                .start()
                .await
                .expect("Failed to start MySQL container");

            let host_port = container
                .get_host_port_ipv4(3306)
                .await
                .expect("Failed to get host port");

            // testcontainers-modules' default Mysql exposes a database named `test` with user `root`
            // and no password.
            let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

            let store =
                MysqlTreeStore::from_config(MysqlStorageConfig::with_defaults(connection_string))
                    .await
                    .expect("Failed to create MysqlTreeStore");

            Self { store, container }
        }
    }

    fn create_test_tree_node(id: &str, value: u64) -> TreeNode {
        shared_tests::create_test_tree_node(id, value)
    }

    async fn reserve_leaves(
        store: &MysqlTreeStore,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        shared_tests::reserve_leaves(store, target_amounts, exact_only, purpose).await
    }

    // ==================== Shared tests ====================

    #[tokio::test]
    async fn test_new() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_new(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_add_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_add_leaves_duplicate_ids() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves_duplicate_ids(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_reserve_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_cancel_reservation() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_cancel_reservation(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_reservation() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_finalize_reservation(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_full_payment_cycle() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_full_payment_cycle(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves_skipped_during_active_swap() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_skipped_during_active_swap(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves_skipped_after_swap_completes_during_refresh() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_skipped_after_swap_completes_during_refresh(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_payment_reservation_does_not_block_set_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_payment_reservation_does_not_block_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_update_reservation_basic() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_update_reservation_basic(&fixture.store).await;
    }

    // ==================== MySQL-Specific Tests ====================

    #[tokio::test]
    async fn test_stale_reservation_cleanup() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        let reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.reserved_for_payment.len(), 1);
        assert_eq!(all_leaves.available.len(), 1);

        // Backdate the reservation past the timeout.
        let mut conn = fixture.store.pool.get_conn().await.unwrap();
        conn.exec_drop(
            "UPDATE tree_reservations SET created_at = DATE_SUB(NOW(6), INTERVAL 10 MINUTE) WHERE id = ?",
            (&reservation.id,),
        )
        .await
        .unwrap();
        drop(conn);

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let refresh_start = SystemTime::now();
        let refresh_leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture
            .store
            .set_leaves(&refresh_leaves, &[], refresh_start)
            .await
            .unwrap();

        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(
            all_leaves.reserved_for_payment.is_empty(),
            "Stale reservation should be cleaned up"
        );
        assert_eq!(
            all_leaves.available.len(),
            2,
            "Previously reserved leaf should be available again"
        );
    }

    #[tokio::test]
    #[allow(clippy::arithmetic_side_effects)]
    async fn test_concurrent_reserve_and_finalize() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        let store = Arc::new(fixture.store);

        let mut leaves = Vec::new();
        for i in 0..50 {
            leaves.push(create_test_tree_node(&format!("node{i}"), 10));
        }
        store.add_leaves(&leaves).await.unwrap();

        let mut join_set = tokio::task::JoinSet::new();
        for i in 0..10 {
            let store_clone = Arc::clone(&store);
            join_set.spawn(async move {
                let result = store_clone
                    .try_reserve_leaves(
                        Some(&TargetAmounts::new_amount_and_fee(10, None)),
                        true,
                        ReservationPurpose::Payment,
                    )
                    .await;

                match result {
                    Ok(ReserveResult::Success(reservation)) => store_clone
                        .finalize_reservation(&reservation.id, None)
                        .await
                        .map(|()| (i, "reserved and finalized")),
                    Ok(ReserveResult::InsufficientFunds) => Ok((i, "insufficient funds")),
                    Ok(ReserveResult::WaitForPending { .. }) => Ok((i, "wait for pending")),
                    Err(e) => Err(e),
                }
            });
        }

        let mut successes = 0;
        let timeout = tokio::time::timeout(std::time::Duration::from_mins(1), async {
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(Ok((i, msg))) => {
                        tracing::info!("Task {i}: {msg}");
                        if msg.contains("finalized") {
                            successes += 1;
                        }
                    }
                    Ok(Err(e)) => panic!("Task failed with error: {e:?}"),
                    Err(e) => panic!("Task panicked: {e:?}"),
                }
            }
            successes
        })
        .await
        .expect("Test timed out - possible deadlock");

        assert!(timeout > 0, "Expected at least one successful reservation");
    }
}
