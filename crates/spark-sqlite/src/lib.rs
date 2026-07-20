//! A `SQLite`-backed `TreeStore`: the single durable source of truth for a wallet's
//! leaves, reservations, spent records, and the node/ancestor chain.
//!
//! Leaves and ancestors live in two tables. `brz_tree_leaves` holds spendable
//! leaves with their pool metadata (reservation, missing-from-operators, timestamp);
//! `brz_tree_ancestors` holds the intermediate nodes a leaf's exit chain walks
//! through, carrying no pool metadata. A leaf that splits is deleted from the leaf
//! table, and its children carry it as an ancestor. Deleting a leaf garbage-collects
//! any ancestors it no longer shares with a surviving leaf. Only runtime
//! coordination (the balance-change notification) is held outside the database.
//!
//! Ports the proven reservation/refresh/spent logic of `spark-postgres`, reusing
//! the shared leaf-selection algorithm (`select_leaves_by_target_amounts`).
//!
//! The store shares the wallet's main `SQLite` database file, so its tables are
//! `brz_`-prefixed to stay clear of the main storage's.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::Duration;

use macros::async_trait;
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use platform_utils::tokio::sync::watch;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, params_from_iter};
use spark_wallet::{
    LeafLike, LeafPedigree, LeafSelection, Leaves, LeavesReservation, LeavesReservationId,
    PublicKey, ReservationPurpose, ReserveResult, TargetAmounts, TreeNode, TreeNodeId,
    TreeNodeStatus, TreeServiceError, TreeStore, VerifiedLeafKeys, assemble_exit_chains,
    select_leaves_by_minimum_amount, select_leaves_by_target_amounts,
};
use uuid::Uuid;

/// Reservations idle longer than this are treated as abandoned by a crashed
/// client and released during `set_leaves`.
const RESERVATION_TIMEOUT_MS: i64 = 5 * 60 * 1000;
/// Spent markers older than this (relative to a refresh) are pruned.
const SPENT_MARKER_CLEANUP_THRESHOLD_MS: i64 = 5 * 60 * 1000;

fn generic(ctx: &str, e: impl std::fmt::Display) -> TreeServiceError {
    TreeServiceError::Generic(format!("{ctx}: {e}"))
}

fn now_millis() -> i64 {
    system_time_to_millis(SystemTime::now())
}

fn system_time_to_millis(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn status_json(status: TreeNodeStatus) -> Result<String, TreeServiceError> {
    serde_json::to_string(&status).map_err(|e| generic("serialize status", e))
}

/// Schema migrations applied in order. A migration's version is its 1-based
/// position, recorded in `brz_tree_schema_migrations`. Append new migrations;
/// never reorder or edit an applied one.
const TREE_MIGRATIONS: &[&str] = &[
    // v1: initial two-table schema.
    "CREATE TABLE brz_tree_reservations (
        id                    TEXT PRIMARY KEY,
        purpose               TEXT NOT NULL,
        pending_change_amount INTEGER NOT NULL DEFAULT 0,
        created_at            INTEGER NOT NULL
    );
    CREATE TABLE brz_tree_leaves (
        id                        TEXT PRIMARY KEY,
        parent_node_id            TEXT,
        status                    TEXT NOT NULL,
        value                     INTEGER NOT NULL DEFAULT 0,
        verifying_public_key      TEXT NOT NULL,
        signing_public_key        TEXT NOT NULL DEFAULT '',
        data                      TEXT NOT NULL,
        is_missing_from_operators INTEGER NOT NULL DEFAULT 0,
        reservation_id            TEXT,
        added_at                  INTEGER
    );
    CREATE INDEX brz_idx_tree_leaves_parent ON brz_tree_leaves (parent_node_id);
    CREATE INDEX brz_idx_tree_leaves_reservation ON brz_tree_leaves (reservation_id);
    CREATE INDEX brz_idx_tree_leaves_slim
        ON brz_tree_leaves (status, is_missing_from_operators, value)
        WHERE reservation_id IS NULL;
    CREATE TABLE brz_tree_ancestors (
        id                   TEXT PRIMARY KEY,
        parent_node_id       TEXT,
        status               TEXT NOT NULL,
        value                INTEGER NOT NULL DEFAULT 0,
        verifying_public_key TEXT NOT NULL,
        data                 TEXT NOT NULL
    );
    CREATE INDEX brz_idx_tree_ancestors_parent ON brz_tree_ancestors (parent_node_id);
    CREATE TABLE brz_tree_spent (
        id       TEXT PRIMARY KEY,
        spent_at INTEGER NOT NULL
    );
    CREATE TABLE brz_tree_swap_status (
        id                INTEGER PRIMARY KEY CHECK (id = 1),
        last_completed_at INTEGER
    );
    INSERT INTO brz_tree_swap_status (id, last_completed_at) VALUES (1, NULL);",
];

/// Applies pending migrations, tracking the version in its own table rather than
/// the file's `PRAGMA user_version`, which the shared main storage owns.
fn run_tree_migrations(conn: &mut Connection) -> Result<(), TreeServiceError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS brz_tree_schema_migrations (version INTEGER PRIMARY KEY);",
    )
    .map_err(|e| generic("create tree migrations table", e))?;
    let current: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM brz_tree_schema_migrations",
            [],
            |row| row.get(0),
        )
        .map_err(|e| generic("read tree schema version", e))?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| generic("begin tree migration", e))?;
    for (index, sql) in TREE_MIGRATIONS.iter().enumerate() {
        let version = i64::try_from(index).unwrap_or(i64::MAX).saturating_add(1);
        if version > current {
            tx.execute_batch(sql)
                .map_err(|e| generic("apply tree migration", e))?;
            tx.execute(
                "INSERT INTO brz_tree_schema_migrations (version) VALUES (?)",
                params![version],
            )
            .map_err(|e| generic("record tree migration", e))?;
        }
    }
    tx.commit()
        .map_err(|e| generic("commit tree migration", e))?;
    Ok(())
}

/// `(id, value)` pair used to run the shared selection algorithm without pulling
/// each leaf's full `data` JSON.
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

/// A [`TreeStore`] backed by a local `SQLite` database file. Each operation opens
/// its own connection (see [`Self::get_connection`]) rather than sharing one behind
/// a lock, so a reader (every balance check) runs alongside the single writer under
/// WAL instead of serializing on a mutex held across blocking I/O.
pub struct SqliteTreeStore {
    db_path: String,
    balance_changed_tx: watch::Sender<()>,
    balance_changed_rx: watch::Receiver<()>,
}

impl SqliteTreeStore {
    /// Opens the tree store at `db_path`, creating its tables if needed.
    pub fn new(db_path: &str) -> Result<Self, TreeServiceError> {
        let (balance_changed_tx, balance_changed_rx) = watch::channel(());
        let store = Self {
            db_path: db_path.to_string(),
            balance_changed_tx,
            balance_changed_rx,
        };
        let mut conn = store.get_connection()?;
        // WAL is recorded in the database header, so enabling it once here applies
        // to every later connection (and to the main storage sharing this file).
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| generic("enable WAL", e))?;
        run_tree_migrations(&mut conn)?;
        Ok(store)
    }

    /// Opens a fresh connection to the store. `busy_timeout` makes a second writer
    /// wait for the lock rather than fail with `SQLITE_BUSY`.
    fn get_connection(&self) -> Result<Connection, TreeServiceError> {
        let conn =
            Connection::open(&self.db_path).map_err(|e| generic("open sqlite tree store", e))?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|e| generic("configure sqlite tree store", e))?;
        Ok(conn)
    }

    fn notify(&self) {
        let _ = self.balance_changed_tx.send(());
    }

    // ---- row (de)serialization ----

    fn row_to_node(data: &str) -> Result<TreeNode, TreeServiceError> {
        serde_json::from_str(data).map_err(|e| generic("deserialize node", e))
    }

    // ---- ancestor persistence ----

    /// Upserts the ancestors of a batch of pedigrees, deduplicated so a chain
    /// shared by sibling leaves is written once rather than once per leaf.
    fn upsert_pedigree_ancestors(
        conn: &Connection,
        pedigrees: &[LeafPedigree],
    ) -> Result<(), TreeServiceError> {
        let mut deduped: std::collections::HashMap<String, &TreeNode> =
            std::collections::HashMap::new();
        for pedigree in pedigrees {
            for ancestor in &pedigree.ancestors {
                deduped.entry(ancestor.id.to_string()).or_insert(ancestor);
            }
        }
        let nodes: Vec<TreeNode> = deduped.into_values().cloned().collect();
        Self::upsert_ancestors(conn, &nodes)
    }

    fn upsert_ancestors(conn: &Connection, nodes: &[TreeNode]) -> Result<(), TreeServiceError> {
        for node in nodes {
            Self::check_compatible(conn, node)?;
            let data = serde_json::to_string(node).map_err(|e| generic("serialize node", e))?;
            conn.execute(
                "INSERT INTO brz_tree_ancestors
                     (id, parent_node_id, status, value, verifying_public_key, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                     status = excluded.status,
                     parent_node_id = excluded.parent_node_id,
                     data = excluded.data",
                params![
                    node.id.to_string(),
                    node.parent_node_id.as_ref().map(ToString::to_string),
                    status_json(node.status)?,
                    i64::try_from(node.value).unwrap_or(i64::MAX),
                    node.verifying_public_key.to_string(),
                    data,
                ],
            )
            .map_err(|e| generic("upsert ancestor", e))?;
        }
        Ok(())
    }

    /// Deletes ancestors no longer on any leaf's parent chain (a deleted leaf's
    /// unshared ancestors); ancestors still shared by a surviving leaf are kept.
    fn gc_ancestors(conn: &Connection) -> Result<(), TreeServiceError> {
        conn.execute(
            "WITH RECURSIVE reachable(id) AS (
                 SELECT parent_node_id FROM brz_tree_leaves
                 WHERE parent_node_id IS NOT NULL
                 UNION
                 SELECT a.parent_node_id FROM brz_tree_ancestors a
                 JOIN reachable r ON a.id = r.id
                 WHERE a.parent_node_id IS NOT NULL
             )
             DELETE FROM brz_tree_ancestors WHERE id NOT IN (SELECT id FROM reachable)",
            [],
        )
        .map_err(|e| generic("gc ancestors", e))?;
        Ok(())
    }

    /// Errors if `node` conflicts with a stored node of the same id on a field
    /// that must not change (`value`, verifying key). Projects those two columns
    /// instead of loading and deserializing the full node blob on every write.
    fn check_compatible(conn: &Connection, node: &TreeNode) -> Result<(), TreeServiceError> {
        let existing: Option<(i64, String)> = conn
            .query_row(
                "SELECT value, verifying_public_key FROM brz_tree_leaves WHERE id = ?1
                 UNION ALL
                 SELECT value, verifying_public_key FROM brz_tree_ancestors WHERE id = ?1
                 LIMIT 1",
                params![node.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| generic("check compatible", e))?;
        let Some((value, verifying_key)) = existing else {
            return Ok(());
        };
        // Mirror `ensure_node_compatible`: value and verifying key are fixed for an id.
        if value != i64::try_from(node.value).unwrap_or(i64::MAX) {
            return Err(TreeServiceError::Generic(format!(
                "node {} value changed from {} to {}",
                node.id, value, node.value
            )));
        }
        if verifying_key != node.verifying_public_key.to_string() {
            return Err(TreeServiceError::Generic(format!(
                "node {} verifying public key changed",
                node.id
            )));
        }
        Ok(())
    }

    // ---- pool leaf helpers ----

    /// Upserts leaves into the pool, skipping any id in `skip_ids` (spent).
    /// Refreshes the leaf's mutable fields (status, value, parent, data,
    /// timestamp); preserves `reservation_id`.
    fn upsert_leaves<'a>(
        conn: &Connection,
        leaves: impl IntoIterator<Item = &'a TreeNode>,
        is_missing: bool,
        skip_ids: Option<&HashSet<String>>,
    ) -> Result<(), TreeServiceError> {
        let now = now_millis();
        for leaf in leaves {
            let id = leaf.id.to_string();
            if skip_ids.is_some_and(|s| s.contains(&id)) {
                continue;
            }
            Self::check_compatible(conn, leaf)?;
            let data = serde_json::to_string(leaf).map_err(|e| generic("serialize leaf", e))?;
            conn.execute(
                "INSERT INTO brz_tree_leaves
                     (id, parent_node_id, status, value, verifying_public_key,
                      signing_public_key, data, is_missing_from_operators, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                     parent_node_id = excluded.parent_node_id,
                     status = excluded.status,
                     value = excluded.value,
                     data = excluded.data,
                     is_missing_from_operators = excluded.is_missing_from_operators,
                     added_at = excluded.added_at",
                params![
                    id,
                    leaf.parent_node_id.as_ref().map(ToString::to_string),
                    status_json(leaf.status)?,
                    i64::try_from(leaf.value).unwrap_or(i64::MAX),
                    leaf.verifying_public_key.to_string(),
                    leaf.signing_keyshare.public_key.to_string(),
                    data,
                    i64::from(is_missing),
                    now,
                ],
            )
            .map_err(|e| generic("upsert leaf", e))?;
        }
        Ok(())
    }

    /// Deletes a reservation's leaves from the pool (they are spent or superseded),
    /// returning the number removed.
    fn delete_reserved(conn: &Connection, reservation_id: &str) -> Result<usize, TreeServiceError> {
        conn.execute(
            "DELETE FROM brz_tree_leaves WHERE reservation_id = ?1",
            params![reservation_id],
        )
        .map_err(|e| generic("delete reserved leaves", e))
    }

    fn reserved_leaf_ids(
        conn: &Connection,
        reservation_id: &str,
    ) -> Result<Vec<String>, TreeServiceError> {
        let mut stmt = conn
            .prepare("SELECT id FROM brz_tree_leaves WHERE reservation_id = ?1")
            .map_err(|e| generic("prepare reserved ids", e))?;
        let ids = stmt
            .query_map(params![reservation_id], |row| row.get::<_, String>(0))
            .map_err(|e| generic("query reserved ids", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| generic("read reserved ids", e))?;
        Ok(ids)
    }

    fn reservation_purpose(
        conn: &Connection,
        id: &str,
    ) -> Result<Option<String>, TreeServiceError> {
        conn.query_row(
            "SELECT purpose FROM brz_tree_reservations WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| generic("query reservation", e))
    }

    fn insert_spent(conn: &Connection, ids: &[String]) -> Result<(), TreeServiceError> {
        let now = now_millis();
        for id in ids {
            conn.execute(
                "INSERT OR IGNORE INTO brz_tree_spent (id, spent_at) VALUES (?1, ?2)",
                params![id, now],
            )
            .map_err(|e| generic("insert spent marker", e))?;
        }
        Ok(())
    }

    fn remove_spent(conn: &Connection, ids: &[String]) -> Result<(), TreeServiceError> {
        for id in ids {
            conn.execute("DELETE FROM brz_tree_spent WHERE id = ?1", params![id])
                .map_err(|e| generic("remove spent marker", e))?;
        }
        Ok(())
    }

    fn create_reservation(
        conn: &Connection,
        id: &str,
        leaves: &[TreeNode],
        purpose: ReservationPurpose,
        pending_change: u64,
    ) -> Result<(), TreeServiceError> {
        conn.execute(
            "INSERT INTO brz_tree_reservations (id, purpose, pending_change_amount, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                purpose.to_string(),
                i64::try_from(pending_change).unwrap_or(i64::MAX),
                now_millis(),
            ],
        )
        .map_err(|e| generic("insert reservation", e))?;
        Self::set_reservation_id(conn, id, leaves)
    }

    fn set_reservation_id(
        conn: &Connection,
        reservation_id: &str,
        leaves: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        for leaf in leaves {
            conn.execute(
                "UPDATE brz_tree_leaves SET reservation_id = ?1 WHERE id = ?2",
                params![reservation_id, leaf.id.to_string()],
            )
            .map_err(|e| generic("set reservation id", e))?;
        }
        Ok(())
    }

    fn cleanup_stale_reservations(conn: &Connection) -> Result<(), TreeServiceError> {
        let cutoff = now_millis().saturating_sub(RESERVATION_TIMEOUT_MS);
        conn.execute(
            "UPDATE brz_tree_leaves SET reservation_id = NULL
             WHERE reservation_id IN (
                 SELECT id FROM brz_tree_reservations WHERE created_at < ?1
             )",
            params![cutoff],
        )
        .map_err(|e| generic("release stale reservations", e))?;
        conn.execute(
            "DELETE FROM brz_tree_reservations WHERE created_at < ?1",
            params![cutoff],
        )
        .map_err(|e| generic("delete stale reservations", e))?;
        Ok(())
    }

    fn cleanup_spent_markers(conn: &Connection, refresh_ms: i64) -> Result<(), TreeServiceError> {
        conn.execute(
            "DELETE FROM brz_tree_spent WHERE spent_at < ?1",
            params![refresh_ms.saturating_sub(SPENT_MARKER_CLEANUP_THRESHOLD_MS)],
        )
        .map_err(|e| generic("cleanup spent markers", e))?;
        Ok(())
    }

    /// `(has_active_swap, swap_completed_during_refresh)`.
    fn swap_guard(conn: &Connection, refresh_ms: i64) -> Result<(bool, bool), TreeServiceError> {
        let has_active_swap: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM brz_tree_reservations WHERE purpose = 'Swap')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| generic("query active swap", e))?
            != 0;
        let completed: Option<i64> = conn
            .query_row(
                "SELECT last_completed_at FROM brz_tree_swap_status WHERE id = 1",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(|e| generic("query swap status", e))?;
        Ok((has_active_swap, completed.is_some_and(|c| c >= refresh_ms)))
    }

    fn mark_swap_completed(conn: &Connection) -> Result<(), TreeServiceError> {
        conn.execute(
            "UPDATE brz_tree_swap_status SET last_completed_at = ?1 WHERE id = 1",
            params![now_millis()],
        )
        .map_err(|e| generic("mark swap completed", e))?;
        Ok(())
    }

    fn spent_ids_since(
        conn: &Connection,
        refresh_ms: i64,
    ) -> Result<HashSet<String>, TreeServiceError> {
        let mut stmt = conn
            .prepare("SELECT id FROM brz_tree_spent WHERE spent_at >= ?1")
            .map_err(|e| generic("prepare spent ids", e))?;
        let ids = stmt
            .query_map(params![refresh_ms], |row| row.get::<_, String>(0))
            .map_err(|e| generic("query spent ids", e))?
            .collect::<Result<HashSet<_>, _>>()
            .map_err(|e| generic("read spent ids", e))?;
        Ok(ids)
    }

    /// Total value of unreserved available pool leaves (drives `WaitForPending`).
    fn available_total(conn: &Connection) -> Result<u64, TreeServiceError> {
        let available = status_json(TreeNodeStatus::Available)?;
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(value), 0) FROM brz_tree_leaves
                 WHERE status = ?1
                   AND is_missing_from_operators = 0 AND reservation_id IS NULL",
                params![available],
                |row| row.get(0),
            )
            .map_err(|e| generic("query available total", e))?;
        Ok(u64::try_from(total).unwrap_or(0))
    }

    fn pending_balance(conn: &Connection) -> Result<u64, TreeServiceError> {
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(pending_change_amount), 0) FROM brz_tree_reservations",
                [],
                |row| row.get(0),
            )
            .map_err(|e| generic("query pending balance", e))?;
        Ok(u64::try_from(total).unwrap_or(0))
    }

    /// Slim `(id, value)` candidates: every eligible leaf with `value <= max_target`
    /// plus the single smallest eligible leaf above it (the min-amount fallback).
    fn slim_candidates(
        conn: &Connection,
        max_target: u64,
    ) -> Result<Vec<SlimLeaf>, TreeServiceError> {
        let available = status_json(TreeNodeStatus::Available)?;
        let max_target_i = i64::try_from(max_target).unwrap_or(i64::MAX);
        let mut stmt = conn
            .prepare(
                "SELECT id, value FROM brz_tree_leaves
                 WHERE status = ?1
                   AND is_missing_from_operators = 0 AND reservation_id IS NULL
                   AND (
                     value <= ?2
                     OR id = (
                       SELECT id FROM brz_tree_leaves
                       WHERE status = ?1
                         AND is_missing_from_operators = 0 AND reservation_id IS NULL
                         AND value > ?2
                       ORDER BY value LIMIT 1
                     )
                   )",
            )
            .map_err(|e| generic("prepare slim query", e))?;
        let rows = stmt
            .query_map(params![available, max_target_i], |row| {
                Ok(SlimLeaf {
                    id: row.get(0)?,
                    value: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                })
            })
            .map_err(|e| generic("query slim candidates", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| generic("read slim candidates", e))?;
        Ok(rows)
    }

    /// Full `TreeNode`s for the selected ids, preserving the selection order.
    fn resolve_full_leaves(
        conn: &Connection,
        ids: &[String],
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        let mut leaves = Vec::with_capacity(ids.len());
        for id in ids {
            let data = conn
                .query_row(
                    "SELECT data FROM brz_tree_leaves WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| generic("resolve selected leaf", e))?
                .ok_or_else(|| {
                    TreeServiceError::Generic(format!("selected leaf {id} not found in store"))
                })?;
            leaves.push(Self::row_to_node(&data)?);
        }
        Ok(leaves)
    }

    fn slim_max_target(target_amounts: Option<&TargetAmounts>) -> u64 {
        match target_amounts {
            Some(TargetAmounts::AmountAndFee {
                amount_sats,
                fee_sats,
            }) => amount_sats.saturating_add(fee_sats.unwrap_or(0)),
            Some(TargetAmounts::ExactDenominations { denominations }) => denominations
                .iter()
                .copied()
                .try_fold(0u64, u64::checked_add)
                .unwrap_or(u64::MAX),
            None => u64::MAX,
        }
    }
}

#[async_trait]
impl TreeStore for SqliteTreeStore {
    // ---- node/ancestor chain (durable, exit-critical) ----

    async fn get_exit_chains(
        &self,
        leaf_ids: &[TreeNodeId],
    ) -> Result<Vec<LeafPedigree>, TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.get_connection()?;
        // One query loads the requested leaves plus every ancestor (the ancestor
        // table only holds this wallet's chains), then the walk happens in memory.
        // Ancestors come first so a leaf overwrites an ancestor of the same id.
        let placeholders = vec!["?"; leaf_ids.len()].join(",");
        let sql = format!(
            "SELECT data FROM brz_tree_ancestors
             UNION ALL
             SELECT data FROM brz_tree_leaves WHERE id IN ({placeholders})"
        );
        let id_strings: Vec<String> = leaf_ids.iter().map(ToString::to_string).collect();
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| generic("prepare get_exit_chains", e))?;
        let rows = stmt
            .query_map(params_from_iter(id_strings.iter()), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| generic("query get_exit_chains", e))?;
        let mut nodes: HashMap<TreeNodeId, TreeNode> = HashMap::new();
        for row in rows {
            let data = row.map_err(|e| generic("read get_exit_chains row", e))?;
            let node = Self::row_to_node(&data)?;
            nodes.insert(node.id.clone(), node);
        }
        Ok(assemble_exit_chains(&nodes, leaf_ids))
    }

    // ---- pool + reservations ----

    async fn add_leaves(&self, leaves: &[LeafPedigree]) -> Result<(), TreeServiceError> {
        if leaves.is_empty() {
            return Ok(());
        }
        let mut conn = self.get_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| generic("begin add_leaves", e))?;
        let ids: Vec<String> = leaves.iter().map(|p| p.leaf.id.to_string()).collect();
        // Receiving a leaf back clears any prior spent marker.
        Self::remove_spent(&tx, &ids)?;
        Self::upsert_pedigree_ancestors(&tx, leaves)?;
        Self::upsert_leaves(&tx, leaves.iter().map(|p| &p.leaf), false, None)?;
        tx.commit().map_err(|e| generic("commit add_leaves", e))?;
        self.notify();
        Ok(())
    }

    async fn get_leaves(&self) -> Result<Leaves, TreeServiceError> {
        let conn = self.get_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT n.data, n.is_missing_from_operators, r.purpose
                 FROM brz_tree_leaves n
                 LEFT JOIN brz_tree_reservations r ON n.reservation_id = r.id",
            )
            .map_err(|e| generic("prepare get_leaves", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| generic("query get_leaves", e))?;

        let mut leaves = Leaves {
            available: Vec::new(),
            not_available: Vec::new(),
            available_missing_from_operators: Vec::new(),
            reserved_for_payment: Vec::new(),
            reserved_for_swap: Vec::new(),
        };
        for row in rows {
            let (data, is_missing, purpose) = row.map_err(|e| generic("read get_leaves row", e))?;
            let node = Self::row_to_node(&data)?;
            let spendable = node.status == TreeNodeStatus::Available;
            if let Some(purpose) = purpose {
                match purpose
                    .parse::<ReservationPurpose>()
                    .map_err(TreeServiceError::Generic)?
                {
                    ReservationPurpose::Payment => leaves.reserved_for_payment.push(node),
                    ReservationPurpose::Swap => leaves.reserved_for_swap.push(node),
                }
            } else if !spendable {
                leaves.not_available.push(node);
            } else if is_missing {
                leaves.available_missing_from_operators.push(node);
            } else {
                leaves.available.push(node);
            }
        }
        Ok(leaves)
    }

    async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
        let conn = self.get_connection()?;
        let available = status_json(TreeNodeStatus::Available)?;
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(l.value), 0) FROM brz_tree_leaves l
                 LEFT JOIN brz_tree_reservations r ON l.reservation_id = r.id
                 WHERE (l.reservation_id IS NULL AND l.status = ?1)
                    OR r.purpose = ?2",
                params![available, ReservationPurpose::Swap.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| generic("query available balance", e))?;
        Ok(u64::try_from(total).unwrap_or(0))
    }

    async fn get_verified_leaf_keys(
        &self,
    ) -> Result<HashMap<TreeNodeId, VerifiedLeafKeys>, TreeServiceError> {
        let conn = self.get_connection()?;
        let available = status_json(TreeNodeStatus::Available)?;
        // Project the two pubkey columns, skipping each leaf's `data` blob. Covers
        // the same categories as `verified_leaf_keys_from_leaves`: every reserved
        // leaf plus every Available one, never a non-Available unreserved leaf.
        let mut stmt = conn
            .prepare(
                "SELECT l.id, l.verifying_public_key, l.signing_public_key
                 FROM brz_tree_leaves l
                 LEFT JOIN brz_tree_reservations r ON l.reservation_id = r.id
                 WHERE r.purpose IS NOT NULL OR l.status = ?1",
            )
            .map_err(|e| generic("prepare verified leaf keys", e))?;
        let rows = stmt
            .query_map(params![available], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| generic("query verified leaf keys", e))?;
        let mut keys = HashMap::new();
        for row in rows {
            let (id, verifying, keyshare) =
                row.map_err(|e| generic("read verified leaf key", e))?;
            // A valid leaf carries both keys; skip one missing either rather than
            // failing the refresh over a single unverifiable leaf.
            let (Ok(verifying_public_key), Ok(signing_keyshare_public_key)) = (
                PublicKey::from_str(&verifying),
                PublicKey::from_str(&keyshare),
            ) else {
                continue;
            };
            keys.insert(
                TreeNodeId::from_str(&id).map_err(TreeServiceError::Generic)?,
                VerifiedLeafKeys {
                    verifying_public_key,
                    signing_keyshare_public_key,
                },
            );
        }
        Ok(keys)
    }

    async fn set_leaves(
        &self,
        leaves: &[LeafPedigree],
        missing_operators_leaves: &[LeafPedigree],
        refresh_started_at: SystemTime,
    ) -> Result<(), TreeServiceError> {
        let refresh_ms = system_time_to_millis(refresh_started_at);
        let mut conn = self.get_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| generic("begin set_leaves", e))?;

        // Release abandoned reservations before evaluating the swap guard so a
        // stale swap cannot pin set_leaves forever.
        Self::cleanup_stale_reservations(&tx)?;
        let (has_active_swap, swap_completed) = Self::swap_guard(&tx, refresh_ms)?;
        if has_active_swap || swap_completed {
            // Skip the potentially-inconsistent refresh but commit the stale-
            // reservation cleanup, so every backend converges on the same state.
            tx.commit()
                .map_err(|e| generic("commit set_leaves skip", e))?;
            return Ok(());
        }

        Self::cleanup_spent_markers(&tx, refresh_ms)?;
        let spent = Self::spent_ids_since(&tx, refresh_ms)?;

        // Delete non-reserved pool leaves older than the refresh; reserved and
        // after-refresh leaves are immune. Leaves present in the refresh below are
        // re-inserted, and their now-unshared ancestors are collected afterwards.
        let deleted = tx
            .execute(
                "DELETE FROM brz_tree_leaves
                 WHERE reservation_id IS NULL AND added_at < ?1",
                params![refresh_ms],
            )
            .map_err(|e| generic("delete old leaves", e))?;

        Self::upsert_pedigree_ancestors(&tx, leaves)?;
        Self::upsert_pedigree_ancestors(&tx, missing_operators_leaves)?;
        Self::upsert_leaves(&tx, leaves.iter().map(|p| &p.leaf), false, Some(&spent))?;
        Self::upsert_leaves(
            &tx,
            missing_operators_leaves.iter().map(|p| &p.leaf),
            true,
            Some(&spent),
        )?;
        // Only a deleted leaf can orphan an ancestor; skip the walk otherwise.
        if deleted > 0 {
            Self::gc_ancestors(&tx)?;
        }

        tx.commit().map_err(|e| generic("commit set_leaves", e))?;
        self.notify();
        Ok(())
    }

    async fn cancel_reservation(
        &self,
        id: &LeavesReservationId,
        leaves_to_keep: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let mut conn = self.get_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| generic("begin cancel", e))?;
        // Return leaves_to_keep to the pool even when the reservation is already
        // gone (e.g. released by stale cleanup): dropping them here would lose the
        // leaves until the next refresh. The two deletes below no-op in that case.
        // Only the leaves are re-inserted: their ancestors stayed in the ancestor
        // table the whole time they were reserved.
        Self::delete_reserved(&tx, id)?;
        tx.execute(
            "DELETE FROM brz_tree_reservations WHERE id = ?1",
            params![id],
        )
        .map_err(|e| generic("delete reservation", e))?;
        Self::upsert_leaves(&tx, leaves_to_keep, false, None)?;
        tx.commit().map_err(|e| generic("commit cancel", e))?;
        self.notify();
        Ok(())
    }

    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[LeafPedigree]>,
    ) -> Result<(), TreeServiceError> {
        let mut conn = self.get_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| generic("begin finalize", e))?;
        let is_swap = Self::reservation_purpose(&tx, id)?.as_deref() == Some("Swap");
        let reserved = Self::reserved_leaf_ids(&tx, id)?;
        Self::insert_spent(&tx, &reserved)?;
        let deleted = Self::delete_reserved(&tx, id)?;
        tx.execute(
            "DELETE FROM brz_tree_reservations WHERE id = ?1",
            params![id],
        )
        .map_err(|e| generic("delete reservation", e))?;
        if let Some(new_leaves) = new_leaves {
            Self::upsert_pedigree_ancestors(&tx, new_leaves)?;
            Self::upsert_leaves(&tx, new_leaves.iter().map(|p| &p.leaf), false, None)?;
            if is_swap {
                Self::mark_swap_completed(&tx)?;
            }
        }
        // Only a deleted (spent) leaf can orphan an ancestor; skip the walk otherwise.
        if deleted > 0 {
            Self::gc_ancestors(&tx)?;
        }
        tx.commit().map_err(|e| generic("commit finalize", e))?;
        self.notify();
        Ok(())
    }

    #[allow(clippy::arithmetic_side_effects)]
    async fn try_reserve_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<ReserveResult, TreeServiceError> {
        let target_amount = target_amounts.map_or(0, TargetAmounts::total_sats);
        let max_target = Self::slim_max_target(target_amounts);
        let reservation_id = Uuid::now_v7().to_string();

        let mut conn = self.get_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| generic("begin reserve", e))?;
        let available = Self::available_total(&tx)?;
        let slim = Self::slim_candidates(&tx, max_target)?;
        let pending = Self::pending_balance(&tx)?;

        let result = match select_leaves_by_target_amounts(&slim, target_amounts) {
            Ok(target_leaves) => {
                let ids: Vec<String> = target_leaves
                    .amount_leaves
                    .iter()
                    .chain(target_leaves.fee_leaves.iter().flatten())
                    .map(|l| l.id.clone())
                    .collect();
                if ids.is_empty() {
                    return Err(TreeServiceError::NonReservableLeaves);
                }
                let selected = Self::resolve_full_leaves(&tx, &ids)?;
                Self::create_reservation(&tx, &reservation_id, &selected, purpose, 0)?;
                ReserveResult::Success(LeavesReservation::new(selected, reservation_id))
            }
            Err(_) if !exact_only => match select_leaves_by_minimum_amount(&slim, target_amount) {
                Ok(Some(min_slim)) => {
                    let ids: Vec<String> = min_slim.iter().map(|l| l.id.clone()).collect();
                    let selected = Self::resolve_full_leaves(&tx, &ids)?;
                    let reserved_amount: u64 = selected.iter().map(|l| l.value).sum();
                    let pending_change = if reserved_amount > target_amount && target_amount > 0 {
                        reserved_amount - target_amount
                    } else {
                        0
                    };
                    Self::create_reservation(
                        &tx,
                        &reservation_id,
                        &selected,
                        purpose,
                        pending_change,
                    )?;
                    ReserveResult::Success(LeavesReservation::new(selected, reservation_id))
                }
                _ if available + pending >= target_amount => ReserveResult::WaitForPending {
                    needed: target_amount,
                    available,
                    pending,
                },
                _ => ReserveResult::InsufficientFunds,
            },
            Err(_) if available + pending >= target_amount => ReserveResult::WaitForPending {
                needed: target_amount,
                available,
                pending,
            },
            Err(_) => ReserveResult::InsufficientFunds,
        };

        if matches!(result, ReserveResult::Success(_)) {
            tx.commit().map_err(|e| generic("commit reserve", e))?;
            self.notify();
        }
        Ok(result)
    }

    async fn try_select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeafSelection, TreeServiceError> {
        let target_amount = target_amounts.map_or(0, TargetAmounts::total_sats);
        let max_target = Self::slim_max_target(target_amounts);

        // One snapshot across both reads: a leaf deleted by a concurrent write
        // between selecting candidates and resolving them can't fail resolution.
        let mut conn = self.get_connection()?;
        let tx = conn.transaction().map_err(|e| generic("begin select", e))?;
        let slim = Self::slim_candidates(&tx, max_target)?;

        match select_leaves_by_target_amounts(&slim, target_amounts) {
            Ok(target_leaves) => {
                let ids: Vec<String> = target_leaves
                    .amount_leaves
                    .iter()
                    .chain(target_leaves.fee_leaves.iter().flatten())
                    .map(|l| l.id.clone())
                    .collect();
                if ids.is_empty() {
                    return Err(TreeServiceError::InsufficientFunds);
                }
                Ok(LeafSelection::Exact(Self::resolve_full_leaves(&tx, &ids)?))
            }
            Err(_) => match select_leaves_by_minimum_amount(&slim, target_amount) {
                Ok(Some(min_slim)) => {
                    let ids: Vec<String> = min_slim.iter().map(|l| l.id.clone()).collect();
                    Ok(LeafSelection::SwapNeeded(Self::resolve_full_leaves(
                        &tx, &ids,
                    )?))
                }
                _ => Err(TreeServiceError::InsufficientFunds),
            },
        }
    }

    async fn try_reserve_leaves_by_ids(
        &self,
        leaf_ids: &[TreeNodeId],
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        if leaf_ids.is_empty() {
            return Err(TreeServiceError::NonReservableLeaves);
        }
        // Reject duplicate ids: each would resolve to the same leaf, double-counting
        // it in the reservation. A repeated id is never a valid reservation.
        let unique: HashSet<&TreeNodeId> = leaf_ids.iter().collect();
        if unique.len() != leaf_ids.len() {
            return Err(TreeServiceError::NonReservableLeaves);
        }
        let reservation_id = Uuid::now_v7().to_string();
        let ids: Vec<String> = leaf_ids.iter().map(ToString::to_string).collect();
        let available = status_json(TreeNodeStatus::Available)?;

        let mut conn = self.get_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| generic("begin reserve by ids", e))?;

        // Every requested leaf must be available and unreserved; otherwise reserve
        // nothing (dropping the transaction without committing rolls back).
        for id in &ids {
            let reservable = tx
                .query_row(
                    "SELECT 1 FROM brz_tree_leaves
                     WHERE id = ?1 AND status = ?2
                       AND is_missing_from_operators = 0 AND reservation_id IS NULL",
                    params![id, available],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|e| generic("check reservable leaf", e))?
                .is_some();
            if !reservable {
                return Err(TreeServiceError::NonReservableLeaves);
            }
        }

        let selected = Self::resolve_full_leaves(&tx, &ids)?;
        Self::create_reservation(&tx, &reservation_id, &selected, purpose, 0)?;
        tx.commit()
            .map_err(|e| generic("commit reserve by ids", e))?;
        self.notify();

        Ok(LeavesReservation::new(selected, reservation_id))
    }

    async fn update_reservation(
        &self,
        reservation_id: &LeavesReservationId,
        reserved_leaves: &[LeafPedigree],
        change_leaves: &[LeafPedigree],
    ) -> Result<LeavesReservation, TreeServiceError> {
        let mut conn = self.get_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| generic("begin update reservation", e))?;
        if Self::reservation_purpose(&tx, reservation_id)?.is_none() {
            return Err(TreeServiceError::Generic(format!(
                "Reservation {reservation_id} not found"
            )));
        }
        let old = Self::reserved_leaf_ids(&tx, reservation_id)?;
        Self::insert_spent(&tx, &old)?;
        Self::delete_reserved(&tx, reservation_id)?;
        Self::upsert_pedigree_ancestors(&tx, change_leaves)?;
        Self::upsert_pedigree_ancestors(&tx, reserved_leaves)?;
        Self::upsert_leaves(&tx, change_leaves.iter().map(|p| &p.leaf), false, None)?;
        Self::upsert_leaves(&tx, reserved_leaves.iter().map(|p| &p.leaf), false, None)?;
        let reserved_nodes: Vec<TreeNode> =
            reserved_leaves.iter().map(|p| p.leaf.clone()).collect();
        Self::set_reservation_id(&tx, reservation_id, &reserved_nodes)?;
        tx.execute(
            "UPDATE brz_tree_reservations SET pending_change_amount = 0 WHERE id = ?1",
            params![reservation_id],
        )
        .map_err(|e| generic("clear pending", e))?;
        tx.commit()
            .map_err(|e| generic("commit update reservation", e))?;
        self.notify();
        Ok(LeavesReservation::new(
            reserved_nodes,
            reservation_id.clone(),
        ))
    }

    async fn now(&self) -> Result<SystemTime, TreeServiceError> {
        Ok(SystemTime::now())
    }

    fn subscribe_balance_changes(&self) -> watch::Receiver<()> {
        self.balance_changed_rx.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use spark_wallet::tree_store_tests as shared;

    use super::*;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A unique temp path for a test store's files.
    fn temp_db_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "spark_sqlite_tree_test_{}_{}.db",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// A file-backed store whose database (and WAL sidecars) are removed when the
    /// guard drops. Per-operation connections need a real file, so tests cannot use
    /// a private `:memory:` database. Derefs to the store so it passes as a
    /// `&dyn TreeStore`.
    struct TempStore {
        store: SqliteTreeStore,
        path: PathBuf,
    }

    impl std::ops::Deref for TempStore {
        type Target = SqliteTreeStore;
        fn deref(&self) -> &SqliteTreeStore {
            &self.store
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    fn store() -> TempStore {
        let path = temp_db_path();
        let store = SqliteTreeStore::new(path.to_str().unwrap()).unwrap();
        TempStore { store, path }
    }

    macro_rules! shared_tests {
        ($($name:ident),* $(,)?) => {
            $(
                #[tokio::test]
                async fn $name() {
                    shared::$name(&*store()).await;
                }
            )*
        };
    }

    shared_tests!(
        // node / exit chain
        test_upsert_and_get_leaf,
        test_get_exit_chains,
        test_get_exit_chain_missing_ancestor,
        test_node_update_in_place,
        test_leaf_reparented_by_renewal,
        test_ancestor_not_returned_as_leaf,
        test_unshared_ancestor_deleted_with_leaf,
        test_shared_ancestor_survives_leaf_deletion,
        test_incomplete_pedigree_still_spendable,
        test_exit_chain_after_swap_update,
        test_exit_chain_after_cancel_reparent,
        // add / get leaves
        test_new,
        test_add_leaves,
        test_add_leaves_duplicate_ids,
        test_add_leaves_empty_slice,
        test_add_leaves_clears_missing_from_operators,
        test_missing_from_operators_leaves_are_not_selectable,
        test_missing_from_operators_leaf_not_available,
        test_get_leaves_not_available,
        // set_leaves core
        test_set_leaves,
        test_set_leaves_replaces_fully,
        test_set_leaves_with_reservations,
        test_set_leaves_preserves_reservations_for_in_flight_swaps,
        test_old_leaves_deleted_by_set_leaves,
        test_add_leaves_not_deleted_by_set_leaves,
        // set_leaves swap guard
        test_set_leaves_skipped_during_active_swap,
        test_set_leaves_skipped_after_swap_completes_during_refresh,
        test_set_leaves_proceeds_after_swap_when_refresh_starts_later,
        test_payment_reservation_does_not_block_set_leaves,
        // spent markers
        test_spent_leaves_not_restored_by_set_leaves,
        test_spent_ids_cleaned_up_when_no_longer_in_refresh,
        test_add_leaves_clears_spent_status,
        test_change_leaves_from_swap_protected,
        test_finalize_with_new_leaves_protected,
        // missing operators
        test_get_leaves_missing_operators_filters_spent,
        test_missing_operators_replaced_on_set_leaves,
        // reserve
        test_reserve_leaves,
        test_reserve_leaves_empty,
        test_reserve_with_none_target_reserves_all,
        test_reserve_skips_non_available_leaves,
        test_non_reservable_leaves,
        test_multiple_reservations,
        test_reservation_ids_are_unique,
        test_reserve_leaves_by_ids,
        test_reserve_leaves_by_ids_preserves_order,
        test_reserve_leaves_by_ids_not_available,
        test_try_select_leaves,
        test_get_verified_leaf_keys,
        // cancel
        test_cancel_reservation,
        test_cancel_reservation_drops_unkept_leaves,
        test_cancel_reservation_drops_all_when_keep_empty,
        test_cancel_reservation_nonexistent,
        test_cancel_reservation_nonexistent_keeps_leaves,
        // finalize
        test_finalize_reservation,
        test_finalize_reservation_nonexistent,
        test_full_payment_cycle,
        // balance buckets
        test_swap_reservation_included_in_balance,
        test_payment_reservation_excluded_from_balance,
        // try_reserve results
        test_try_reserve_success,
        test_try_reserve_insufficient_funds,
        test_try_reserve_wait_for_pending,
        test_try_reserve_fail_immediately_when_insufficient,
        test_try_reserve_min_amount_with_leaves_above_individual_target,
        test_try_reserve_min_amount_exact_denominations_above_individual,
        // notifications
        test_balance_change_notification,
        test_notification_after_swap_with_exact_amount,
        test_notification_on_pending_balance_change,
        // pending
        test_pending_cleared_on_cancel,
        test_pending_cleared_on_finalize,
        // update reservation
        test_update_reservation_basic,
        test_update_reservation_nonexistent,
        test_update_reservation_clears_pending,
        test_update_reservation_preserves_purpose,
    );

    // Durability: persisted nodes survive dropping and reopening the database.
    #[tokio::test]
    async fn test_nodes_persist_across_reopen() {
        let store = store();
        let path_str = store.path.to_str().unwrap().to_string();

        let root = shared::create_test_node_with_parent("root", None, TreeNodeStatus::Available);
        let leaf =
            shared::create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        store
            .add_leaves(&[LeafPedigree {
                leaf: leaf.clone(),
                ancestors: vec![root.clone()],
            }])
            .await
            .unwrap();

        let reopened = SqliteTreeStore::new(&path_str).unwrap();
        let pedigrees = reopened
            .get_exit_chains(std::slice::from_ref(&leaf.id))
            .await
            .unwrap();
        assert_eq!(pedigrees[0].leaf.id, leaf.id);
        let ids: Vec<String> = pedigrees[0]
            .ancestors
            .iter()
            .map(|n| n.id.to_string())
            .collect();
        assert_eq!(ids, vec!["root"]);
    }

    // Sharing: the tree store coexists in the wallet's main database file and does
    // not disturb the `user_version` the main storage tracks its own schema with.
    #[tokio::test]
    async fn test_shares_db_file_without_clobbering_user_version() {
        let path = temp_db_path();
        let path_str = path.to_str().unwrap().to_string();

        // Stand in for the main storage: it owns `user_version` and its own tables.
        {
            let conn = Connection::open(&path_str).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE main_payments (id TEXT PRIMARY KEY);
                 PRAGMA user_version = 42;",
            )
            .unwrap();
        }

        // The tree store opens the same file and runs its own migrations.
        let store = SqliteTreeStore::new(&path_str).unwrap();
        let leaf = shared::create_test_node_with_parent("leaf", None, TreeNodeStatus::Available);
        store
            .add_leaves(&[LeafPedigree {
                leaf: leaf.clone(),
                ancestors: vec![],
            }])
            .await
            .unwrap();
        assert_eq!(
            store
                .get_exit_chains(std::slice::from_ref(&leaf.id))
                .await
                .unwrap()
                .len(),
            1,
            "the tree store works in the shared file"
        );

        // The main storage's schema version and tables survive, and the tree store
        // tracks its own schema in a separate table.
        let conn = Connection::open(&path_str).unwrap();
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(user_version, 42, "main storage's user_version must survive");
        let main_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='main_payments'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(main_table, 1, "main storage's tables must survive");
        let tree_version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM brz_tree_schema_migrations",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tree_version, i64::try_from(TREE_MIGRATIONS.len()).unwrap());
        drop(conn);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{path_str}-wal"));
        let _ = std::fs::remove_file(format!("{path_str}-shm"));
    }
}
