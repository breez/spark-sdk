//! `MySQL`-backed implementation of the `TreeStore` trait.
//!
//! Direct port of `crates/spark-postgres/src/tree_store.rs`. SQL syntax
//! differences vs. `PostgreSQL`:
//!
//! - `JSONB` → `JSON`
//! - `TIMESTAMPTZ NOT NULL DEFAULT NOW()` → `DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)`
//! - `TEXT PRIMARY KEY` → `VARCHAR(255) PRIMARY KEY` (TEXT cannot be a primary key in `MySQL` without prefix length)
//! - `ON CONFLICT (id) DO UPDATE SET … = EXCLUDED.…` → `ON DUPLICATE KEY UPDATE … = VALUES(…)`
//! - `ON CONFLICT DO NOTHING` → `INSERT … ON DUPLICATE KEY UPDATE <pk> = <pk>`
//!   (avoid `INSERT IGNORE`: it silently swallows non-PK errors too)
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

use crate::advisory_lock::identity_lock_name;
use crate::config::MysqlStorageConfig;
use crate::error::MysqlError;
use crate::migrations::{Migration, run_migrations};
use crate::pool::create_pool;

/// Name of the schema migrations table for `MysqlTreeStore`.
const TREE_MIGRATIONS_TABLE: &str = "tree_schema_migrations";

/// Domain prefix mixed into the per-tenant `GET_LOCK` name so the tree store's
/// locks never collide with the token store's, even when two tenants share a
/// database. The full lock name is `<prefix><hex(sha256(prefix||identity)[..8])>`,
/// ensuring distinct tenants never serialize on each other's writes.
const TREE_STORE_LOCK_PREFIX: &str = "breez-spark-sdk:tree:";

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
/// row-level FK constraints to keep reservations and leaves in sync. Each
/// instance is scoped to a single tenant identity so that multiple tenants
/// can share one `MySQL` database without cross-pollinating tree state.
pub struct MysqlTreeStore {
    pool: Pool,
    /// 33-byte secp256k1 compressed pubkey identifying this tenant. All reads
    /// and writes are filtered by `user_id = self.identity`.
    identity: Vec<u8>,
    /// Stable per-tenant `GET_LOCK` name derived from `identity`. Two tenants
    /// don't serialize on each other's writes; same-tenant writes still
    /// serialize on the same lock.
    lock_name: String,
    balance_changed_tx: Arc<watch::Sender<()>>,
    balance_changed_rx: watch::Receiver<()>,
}

/// Builds the multi-tenant scoping migration for the tree store. Adds
/// `user_id VARBINARY(33)` to every per-user table, backfills with the
/// connecting tenant's identity, and rewrites primary keys / FKs to lead
/// with `user_id`.
#[allow(clippy::too_many_lines)]
fn tree_store_multi_tenant_migration(identity: &[u8]) -> Vec<Migration> {
    let id_hex = hex::encode(identity);
    let id_lit = format!("UNHEX('{id_hex}')");

    let mut stmts: Vec<Migration> = Vec::new();

    // Drop the existing FK from tree_leaves(reservation_id) -> tree_reservations(id)
    // FIRST, before we touch the parent's PK that it depends on. The `MySQL`
    // FK was named via a CONSTRAINT clause on creation as
    // `fk_tree_leaves_reservation`.
    stmts.push(Migration::DropForeignKey {
        name: "fk_tree_leaves_reservation",
        table: "tree_leaves",
    });

    // tree_reservations: scope by user_id. Add the column nullable, backfill,
    // make NOT NULL, and rewrite the PK to lead with user_id.
    stmts.push(Migration::AddColumn {
        table: "tree_reservations",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE tree_reservations SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE tree_reservations MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE tree_reservations DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)",
    ));

    // tree_leaves: same pattern, plus re-add the composite FK to the new
    // tree_reservations PK. The composite FK uses the default ON DELETE NO
    // ACTION instead of the previous `ON DELETE SET NULL`: a whole-row SET NULL
    // would null `user_id` (NOT NULL) and `MySQL` doesn't support column-list
    // SET NULL. Callers (`cleanup_stale_reservations`, `cancel_reservation`,
    // `finalize_reservation`) explicitly clear `reservation_id` before
    // deleting the parent reservation row.
    stmts.push(Migration::AddColumn {
        table: "tree_leaves",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE tree_leaves SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE tree_leaves MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE tree_leaves DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE tree_leaves \
         ADD CONSTRAINT fk_tree_leaves_reservation_user \
         FOREIGN KEY (user_id, reservation_id) \
         REFERENCES tree_reservations(user_id, id)",
    ));
    stmts.push(Migration::DropIndex {
        name: "idx_tree_leaves_available",
        table: "tree_leaves",
    });
    stmts.push(Migration::DropIndex {
        name: "idx_tree_leaves_reservation",
        table: "tree_leaves",
    });
    stmts.push(Migration::DropIndex {
        name: "idx_tree_leaves_added_at",
        table: "tree_leaves",
    });
    stmts.push(Migration::DropIndex {
        name: "idx_tree_leaves_slim",
        table: "tree_leaves",
    });
    stmts.push(Migration::CreateIndex {
        name: "idx_tree_leaves_user_available",
        table: "tree_leaves",
        columns: "(user_id, status, is_missing_from_operators)",
    });
    stmts.push(Migration::CreateIndex {
        name: "idx_tree_leaves_user_reservation",
        table: "tree_leaves",
        columns: "(user_id, reservation_id)",
    });
    stmts.push(Migration::CreateIndex {
        name: "idx_tree_leaves_user_added_at",
        table: "tree_leaves",
        columns: "(user_id, added_at)",
    });
    stmts.push(Migration::CreateIndex {
        name: "idx_tree_leaves_user_slim",
        table: "tree_leaves",
        columns: "(user_id, status, is_missing_from_operators, reservation_id, value)",
    });

    // tree_spent_leaves: scope by user_id.
    stmts.push(Migration::AddColumn {
        table: "tree_spent_leaves",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE tree_spent_leaves SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE tree_spent_leaves MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE tree_spent_leaves DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, leaf_id)",
    ));

    // tree_swap_status was a singleton (PK id=1, CHECK id=1). Drop the PK and
    // the id column, then re-key by user_id so each tenant has its own
    // swap-status row.
    stmts.push(Migration::DropPrimaryKey {
        table: "tree_swap_status",
    });
    stmts.push(Migration::DropColumn {
        table: "tree_swap_status",
        column: "id",
    });
    stmts.push(Migration::AddColumn {
        table: "tree_swap_status",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE tree_swap_status SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE tree_swap_status MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE tree_swap_status ADD PRIMARY KEY (user_id)",
    ));

    stmts
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
        self.add_leaves_inner(&mut conn, leaves).await?;
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
            .exec_first(
                r"SELECT COALESCE(SUM(l.value), 0) AS balance
                  FROM tree_leaves l
                  LEFT JOIN tree_reservations r
                    ON l.reservation_id = r.id AND l.user_id = r.user_id
                  WHERE l.user_id = ?
                    AND (
                        (l.reservation_id IS NULL AND l.status = 'Available')
                        OR r.purpose = 'Swap'
                    )",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_err)?;
        Ok(u64::try_from(row.unwrap_or(0)).unwrap_or(0))
    }

    async fn get_leaves(&self) -> Result<Leaves, TreeServiceError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;

        let rows: Vec<(String, String, bool, String, Option<String>, Option<String>)> = conn
            .exec(
                r"SELECT l.id, l.status, l.is_missing_from_operators, l.data,
                         l.reservation_id, r.purpose
                  FROM tree_leaves l
                  LEFT JOIN tree_reservations r
                    ON l.reservation_id = r.id AND l.user_id = r.user_id
                  WHERE l.user_id = ?",
                (self.identity.clone(),),
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
        self.acquire_write_lock(&mut conn).await?;
        let result = self
            .set_leaves_inner(
                &mut conn,
                leaves,
                missing_operators_leaves,
                refresh_timestamp,
            )
            .await;
        self.release_write_lock_quiet(&mut conn).await;
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
        self.cancel_reservation_inner(&mut conn, id, leaves_to_keep)
            .await?;
        self.notify_balance_change();
        Ok(())
    }

    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        // Serialize against `set_leaves` so its `tree_spent_leaves` snapshot
        // and the upsert that consumes it cannot interleave with this
        // transaction's spent-marker write — otherwise the snapshot would miss
        // our marker and the upsert would write the just-spent leaf back as
        // Available.
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        self.acquire_write_lock(&mut conn).await?;
        let result = self
            .finalize_reservation_inner(&mut conn, id, new_leaves)
            .await;
        self.release_write_lock_quiet(&mut conn).await;
        result?;
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
        self.acquire_write_lock(&mut conn).await?;

        let result = self
            .try_reserve_leaves_inner(
                &mut conn,
                &reservation_id,
                target_amounts,
                target_amount,
                exact_only,
                purpose,
            )
            .await;
        self.release_write_lock_quiet(&mut conn).await;
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
        let reservation = self
            .update_reservation_inner(&mut conn, reservation_id, reserved_leaves, change_leaves)
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
    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_config(
        config: MysqlStorageConfig,
        identity: &[u8],
    ) -> Result<Self, MysqlError> {
        let pool = create_pool(&config)?;
        Self::init(pool, identity).await
    }

    /// Creates a new `MysqlTreeStore` from an existing connection pool.
    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_pool(pool: Pool, identity: &[u8]) -> Result<Self, MysqlError> {
        Self::init(pool, identity).await
    }

    async fn init(pool: Pool, identity: &[u8]) -> Result<Self, MysqlError> {
        let (balance_changed_tx, balance_changed_rx) = watch::channel(());

        let store = Self {
            pool,
            identity: identity.to_vec(),
            lock_name: identity_lock_name(TREE_STORE_LOCK_PREFIX, identity),
            balance_changed_tx: Arc::new(balance_changed_tx),
            balance_changed_rx,
        };

        store.migrate().await?;
        store.notify_balance_change();

        Ok(store)
    }

    async fn migrate(&self) -> Result<(), MysqlError> {
        run_migrations(
            &self.pool,
            TREE_MIGRATIONS_TABLE,
            &Self::migrations(&self.identity),
        )
        .await
    }

    fn migrations(identity: &[u8]) -> Vec<Vec<Migration>> {
        vec![
            // Migration 1: Initial tree tables.
            //
            // Reservations are referenced via FK so that ON DELETE SET NULL
            // releases the leaves automatically when a reservation is dropped.
            vec![
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS tree_reservations (
                        id VARCHAR(255) NOT NULL PRIMARY KEY,
                        purpose VARCHAR(64) NOT NULL,
                        pending_change_amount BIGINT NOT NULL DEFAULT 0,
                        created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
                    )",
                ),
                Migration::sql(
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
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS tree_spent_leaves (
                        leaf_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        spent_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
                    )",
                ),
                Migration::CreateIndex {
                    name: "idx_tree_leaves_available",
                    table: "tree_leaves",
                    columns: "(status, is_missing_from_operators)",
                },
                Migration::CreateIndex {
                    name: "idx_tree_leaves_reservation",
                    table: "tree_leaves",
                    columns: "(reservation_id)",
                },
                Migration::CreateIndex {
                    name: "idx_tree_leaves_added_at",
                    table: "tree_leaves",
                    columns: "(added_at)",
                },
            ],
            // Migration 2: Swap status tracking.
            vec![
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS tree_swap_status (
                        id INT NOT NULL PRIMARY KEY DEFAULT 1,
                        last_completed_at DATETIME(6) NULL,
                        CHECK (id = 1)
                    )",
                ),
                Migration::sql(
                    "INSERT INTO tree_swap_status (id) VALUES (1)
                     ON DUPLICATE KEY UPDATE id = id",
                ),
            ],
            // Migration 3: Promote `value` out of the JSON `data` column into a
            // dedicated BIGINT. JSON_EXTRACT/JSON_UNQUOTE on every reservation
            // and balance query was the dominant cost vs. postgres's
            // `(data->>'value')::bigint` expression. Also adds a composite index
            // `(status, is_missing_from_operators, reservation_id, value)` so
            // the slim selection in `try_reserve_leaves` is index-only.
            vec![
                Migration::AddColumn {
                    table: "tree_leaves",
                    column: "value",
                    definition: "BIGINT NOT NULL DEFAULT 0",
                },
                // Backfill existing rows from the JSON. Re-running this is a
                // no-op because by then `value` is already populated and the
                // `WHERE value = 0` predicate filters everything out.
                Migration::sql(
                    "UPDATE tree_leaves
                        SET value = CAST(JSON_UNQUOTE(JSON_EXTRACT(data, '$.value')) AS UNSIGNED)
                        WHERE value = 0",
                ),
                Migration::CreateIndex {
                    name: "idx_tree_leaves_slim",
                    table: "tree_leaves",
                    columns: "(status, is_missing_from_operators, reservation_id, value)",
                },
            ],
            // Migration 4: Multi-tenant scoping. Adds user_id to every tree-
            // store table, backfills with the connecting tenant's identity, and
            // rewrites primary keys / FKs / indexes to lead with user_id. The
            // `tree_swap_status` singleton is restructured the same way as
            // `sync_revision` in the SDK-core storage.
            tree_store_multi_tenant_migration(identity),
        ]
    }

    fn notify_balance_change(&self) {
        let _ = self.balance_changed_tx.send(());
    }

    /// Acquires the per-tenant write lock for this connection. Held until
    /// `release_write_lock_quiet` is called or the connection is returned to
    /// the pool.
    async fn acquire_write_lock(&self, conn: &mut Conn) -> Result<(), TreeServiceError> {
        let acquired: Option<i64> = conn
            .exec_first(
                "SELECT GET_LOCK(?, ?)",
                (self.lock_name.as_str(), WRITE_LOCK_TIMEOUT_SECS),
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
    async fn release_write_lock_quiet(&self, conn: &mut Conn) {
        let _ = conn
            .exec_drop("SELECT RELEASE_LOCK(?)", (self.lock_name.as_str(),))
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
        &self,
        conn: &mut Conn,
        leaves: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let leaf_ids: Vec<String> = leaves.iter().map(|l| l.id.to_string()).collect();
        self.batch_remove_spent_leaves(&mut tx, &leaf_ids).await?;
        self.batch_upsert_leaves(&mut tx, leaves, false, None)
            .await?;

        tx.commit().await.map_err(map_err)?;
        tracing::trace!(
            "MysqlTreeStore::add_leaves: committed {} leaves",
            leaves.len()
        );
        Ok(())
    }

    async fn set_leaves_inner(
        &self,
        conn: &mut Conn,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
        refresh_timestamp: DateTime<Utc>,
    ) -> Result<(), TreeServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        self.cleanup_stale_reservations(&mut tx).await?;

        // Check if any swap reservation is currently active, or if a swap completed
        // after this refresh started (making the refresh data potentially inconsistent).
        let row: Option<(i64, i64)> = tx
            .exec_first(
                r"SELECT
                    (SELECT EXISTS(SELECT 1 FROM tree_reservations WHERE user_id = ? AND purpose = 'Swap')) AS has_active_swap,
                    COALESCE(
                        (SELECT (last_completed_at >= ?) FROM tree_swap_status WHERE user_id = ?),
                        0
                    ) AS swap_completed_during_refresh",
                (
                    self.identity.clone(),
                    refresh_timestamp.naive_utc(),
                    self.identity.clone(),
                ),
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
            return Ok(());
        }

        self.cleanup_spent_markers(&mut tx, refresh_timestamp)
            .await?;

        let spent_rows: Vec<String> = tx
            .exec(
                "SELECT leaf_id FROM tree_spent_leaves WHERE user_id = ? AND spent_at >= ?",
                (self.identity.clone(), refresh_timestamp.naive_utc()),
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

        // Delete non-reserved leaves added before refresh started. Includes
        // leaves released earlier in this transaction by
        // `cleanup_stale_reservations` (which clears `reservation_id`
        // explicitly because the composite FK uses NO ACTION).
        tx.exec_drop(
            "DELETE FROM tree_leaves WHERE user_id = ? AND reservation_id IS NULL AND added_at < ?",
            (self.identity.clone(), refresh_timestamp.naive_utc()),
        )
        .await
        .map_err(map_err)?;

        self.batch_upsert_leaves(&mut tx, leaves, false, Some(&spent_ids))
            .await?;
        self.batch_upsert_leaves(&mut tx, missing_operators_leaves, true, Some(&spent_ids))
            .await?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn cancel_reservation_inner(
        &self,
        conn: &mut Conn,
        id: &LeavesReservationId,
        leaves_to_keep: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let exists: Option<String> = tx
            .exec_first(
                "SELECT id FROM tree_reservations WHERE user_id = ? AND id = ?",
                (self.identity.clone(), id),
            )
            .await
            .map_err(map_err)?;
        if exists.is_none() {
            tx.commit().await.map_err(map_err)?;
            return Ok(());
        }

        let prior_leaf_ids: Vec<String> = tx
            .exec(
                "SELECT id FROM tree_leaves WHERE user_id = ? AND reservation_id = ?",
                (self.identity.clone(), id),
            )
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

        tx.exec_drop(
            "DELETE FROM tree_leaves WHERE user_id = ? AND reservation_id = ?",
            (self.identity.clone(), id),
        )
        .await
        .map_err(map_err)?;

        tx.exec_drop(
            "DELETE FROM tree_reservations WHERE user_id = ? AND id = ?",
            (self.identity.clone(), id),
        )
        .await
        .map_err(map_err)?;

        self.batch_upsert_leaves(&mut tx, leaves_to_keep, false, None)
            .await?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn finalize_reservation_inner(
        &self,
        conn: &mut Conn,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let purpose: Option<String> = tx
            .exec_first(
                "SELECT purpose FROM tree_reservations WHERE user_id = ? AND id = ?",
                (self.identity.clone(), id),
            )
            .await
            .map_err(map_err)?;

        let (is_swap, reserved_leaf_ids) = if let Some(purpose) = purpose {
            let is_swap = purpose == "Swap";
            let leaf_ids: Vec<String> = tx
                .exec(
                    "SELECT id FROM tree_leaves WHERE user_id = ? AND reservation_id = ?",
                    (self.identity.clone(), id),
                )
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
        self.batch_insert_spent_leaves(&mut tx, &reserved_leaf_ids)
            .await?;

        tx.exec_drop(
            "DELETE FROM tree_leaves WHERE user_id = ? AND reservation_id = ?",
            (self.identity.clone(), id),
        )
        .await
        .map_err(map_err)?;

        tx.exec_drop(
            "DELETE FROM tree_reservations WHERE user_id = ? AND id = ?",
            (self.identity.clone(), id),
        )
        .await
        .map_err(map_err)?;

        if let Some(leaves) = new_leaves {
            for l in leaves {
                trace!(
                    "leaf_lifecycle finalize: adding new leaf={} value={} reservation={}",
                    l.id, l.value, id
                );
            }
            self.batch_upsert_leaves(&mut tx, leaves, false, None)
                .await?;
        }

        // If swap with new leaves, update last_completed_at. UPSERT so a
        // tenant that joined after the multi-tenant migration (and thus has
        // no row) gets one created lazily.
        if is_swap && new_leaves.is_some() {
            tx.exec_drop(
                "INSERT INTO tree_swap_status (user_id, last_completed_at) VALUES (?, NOW(6))
                 ON DUPLICATE KEY UPDATE last_completed_at = VALUES(last_completed_at)",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_err)?;
        }

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    #[allow(clippy::arithmetic_side_effects, clippy::too_many_lines)]
    async fn try_reserve_leaves_inner(
        &self,
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
            .exec_first(
                r"SELECT COALESCE(SUM(value), 0) AS total
                  FROM tree_leaves
                  WHERE user_id = ?
                    AND status = 'Available'
                    AND is_missing_from_operators = 0
                    AND reservation_id IS NULL",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_err)?;
        let available: u64 = u64::try_from(total_row.unwrap_or(0)).unwrap_or(0);

        // Slim projection of selection candidates: id + value only.
        let max_target_signed: i64 = i64::try_from(max_target).unwrap_or(i64::MAX);
        let slim_rows: Vec<(String, i64)> = tx
            .exec(
                r"SELECT id, value
                  FROM tree_leaves
                  WHERE user_id = ?
                    AND status = 'Available'
                    AND is_missing_from_operators = 0
                    AND reservation_id IS NULL
                    AND (
                      value <= ?
                      OR id = (
                        SELECT id FROM (
                          SELECT id FROM tree_leaves
                          WHERE user_id = ?
                            AND status = 'Available'
                            AND is_missing_from_operators = 0
                            AND reservation_id IS NULL
                            AND value > ?
                          ORDER BY value
                          LIMIT 1
                        ) AS smallest_over
                      )
                    )",
                (
                    self.identity.clone(),
                    max_target_signed,
                    self.identity.clone(),
                    max_target_signed,
                ),
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

        let pending = self.calculate_pending_balance(&mut tx).await?;

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
                let selected_leaves = self.resolve_full_leaves(&mut tx, &selected_ids).await?;
                self.create_reservation(&mut tx, reservation_id, &selected_leaves, purpose, 0)
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
                    let selected_leaves = self.resolve_full_leaves(&mut tx, &min_ids).await?;
                    let reserved_amount: u64 = selected_leaves.iter().map(|l| l.value).sum();
                    let pending_change = if reserved_amount > target_amount && target_amount > 0 {
                        reserved_amount - target_amount
                    } else {
                        0
                    };
                    self.create_reservation(
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

    /// Pull the full `TreeNode` JSON only for the leaves the slim selection
    /// picked, preserving the algorithm's selection order. Typically 1-3 rows
    /// even when the slim candidate set was thousands.
    async fn resolve_full_leaves(
        &self,
        tx: &mut mysql_async::Transaction<'_>,
        ids: &[String],
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = build_placeholders(ids.len());
        let sql = format!(
            "SELECT id, data FROM tree_leaves WHERE user_id = ? AND id IN ({placeholders})"
        );
        let mut params: Vec<Value> = Vec::with_capacity(ids.len().saturating_add(1));
        params.push(Value::from(self.identity.clone()));
        params.extend(ids.iter().cloned().map(Value::from));
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
        &self,
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
                "SELECT id FROM tree_reservations WHERE user_id = ? AND id = ?",
                (self.identity.clone(), reservation_id),
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
                "SELECT id FROM tree_leaves WHERE user_id = ? AND reservation_id = ?",
                (self.identity.clone(), reservation_id),
            )
            .await
            .map_err(map_err)?;

        self.batch_insert_spent_leaves(&mut tx, &old_reserved_leaf_ids)
            .await?;
        tx.exec_drop(
            "DELETE FROM tree_leaves WHERE user_id = ? AND reservation_id = ?",
            (self.identity.clone(), reservation_id),
        )
        .await
        .map_err(map_err)?;

        self.batch_upsert_leaves(&mut tx, change_leaves, false, None)
            .await?;
        self.batch_upsert_leaves(&mut tx, reserved_leaves, false, None)
            .await?;

        let leaf_ids: Vec<String> = reserved_leaves.iter().map(|l| l.id.to_string()).collect();
        self.batch_set_reservation_id(&mut tx, reservation_id, &leaf_ids)
            .await?;

        tx.exec_drop(
            "UPDATE tree_reservations SET pending_change_amount = 0 WHERE user_id = ? AND id = ?",
            (self.identity.clone(), reservation_id),
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
        &self,
        tx: &mut mysql_async::Transaction<'_>,
    ) -> Result<u64, TreeServiceError> {
        let row: Option<i64> = tx
            .exec_first(
                "SELECT COALESCE(SUM(pending_change_amount), 0) FROM tree_reservations WHERE user_id = ?",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_err)?;

        Ok(u64::try_from(row.unwrap_or(0)).unwrap_or(0))
    }

    /// Batch upserts leaves into `tree_leaves` table.
    /// Optionally skips leaves whose IDs are in the `skip_ids` set.
    /// Uses `ON DUPLICATE KEY UPDATE` to replace existing leaves.
    #[allow(clippy::arithmetic_side_effects)] // `len * 4` for params capacity, bounded by leaves slice
    async fn batch_upsert_leaves(
        &self,
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

        // Build VALUES (?, ?, ?, ?, ?, ?, NOW(6)), … with user_id as the first column.
        let mut sql = String::from(
            "INSERT INTO tree_leaves (user_id, id, status, is_missing_from_operators, data, value, added_at) VALUES ",
        );
        let mut params: Vec<Value> = Vec::with_capacity(filtered.len() * 6);
        for (i, leaf) in filtered.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str("(?, ?, ?, ?, ?, ?, NOW(6))");
            #[allow(clippy::cast_possible_wrap)]
            let value_i64 = leaf.value as i64;
            params.push(Value::from(self.identity.clone()));
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

    #[allow(clippy::arithmetic_side_effects)] // `len + 2` for params capacity
    async fn batch_set_reservation_id(
        &self,
        tx: &mut mysql_async::Transaction<'_>,
        reservation_id: &str,
        leaf_ids: &[String],
    ) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }

        let placeholders = build_placeholders(leaf_ids.len());
        let sql = format!(
            "UPDATE tree_leaves SET reservation_id = ? WHERE user_id = ? AND id IN ({placeholders})"
        );

        let mut params: Vec<Value> = Vec::with_capacity(leaf_ids.len() + 2);
        params.push(Value::from(reservation_id));
        params.push(Value::from(self.identity.clone()));
        for id in leaf_ids {
            params.push(Value::from(id.clone()));
        }

        tx.exec_drop(&sql, Params::Positional(params))
            .await
            .map_err(map_err)?;

        Ok(())
    }

    async fn batch_insert_spent_leaves(
        &self,
        tx: &mut mysql_async::Transaction<'_>,
        leaf_ids: &[String],
    ) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }

        let mut sql = String::from("INSERT INTO tree_spent_leaves (user_id, leaf_id) VALUES ");
        let mut params: Vec<Value> = Vec::with_capacity(leaf_ids.len().saturating_mul(2));
        for (i, id) in leaf_ids.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str("(?, ?)");
            params.push(Value::from(self.identity.clone()));
            params.push(Value::from(id.clone()));
        }
        // Suppress duplicate-PK errors only — unlike INSERT IGNORE, real
        // problems (FK violations, NOT NULL violations, type errors) still
        // propagate.
        sql.push_str(" ON DUPLICATE KEY UPDATE leaf_id = leaf_id");

        tx.exec_drop(&sql, Params::Positional(params))
            .await
            .map_err(map_err)?;

        Ok(())
    }

    async fn batch_remove_spent_leaves(
        &self,
        tx: &mut mysql_async::Transaction<'_>,
        leaf_ids: &[String],
    ) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }

        let placeholders = build_placeholders(leaf_ids.len());
        let sql = format!(
            "DELETE FROM tree_spent_leaves WHERE user_id = ? AND leaf_id IN ({placeholders})"
        );

        let mut params: Vec<Value> = Vec::with_capacity(leaf_ids.len().saturating_add(1));
        params.push(Value::from(self.identity.clone()));
        params.extend(leaf_ids.iter().cloned().map(Value::from));
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

    /// Cleans up stale reservations for THIS tenant. Releases dependent leaves
    /// by clearing their `reservation_id` first, then deletes the parent
    /// reservation rows — the composite FK uses NO ACTION because column-list
    /// SET NULL would null `user_id` (NOT NULL).
    async fn cleanup_stale_reservations(
        &self,
        tx: &mut mysql_async::Transaction<'_>,
    ) -> Result<u64, TreeServiceError> {
        // Release dependent leaves before dropping the parent rows.
        tx.exec_drop(
            r"UPDATE tree_leaves SET reservation_id = NULL
              WHERE user_id = ?
                AND reservation_id IN (
                    SELECT id FROM (
                        SELECT id FROM tree_reservations
                        WHERE user_id = ?
                          AND created_at < DATE_SUB(NOW(6), INTERVAL ? SECOND)
                    ) AS stale
                )",
            (
                self.identity.clone(),
                self.identity.clone(),
                RESERVATION_TIMEOUT_SECS,
            ),
        )
        .await
        .map_err(map_err)?;

        let mut result = tx
            .exec_iter(
                "DELETE FROM tree_reservations
                 WHERE user_id = ? AND created_at < DATE_SUB(NOW(6), INTERVAL ? SECOND)",
                (self.identity.clone(), RESERVATION_TIMEOUT_SECS),
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
        &self,
        tx: &mut mysql_async::Transaction<'_>,
        refresh_timestamp: DateTime<Utc>,
    ) -> Result<u64, TreeServiceError> {
        let threshold = chrono::Duration::milliseconds(SPENT_MARKER_CLEANUP_THRESHOLD_MS);
        let cleanup_cutoff = refresh_timestamp
            .checked_sub_signed(threshold)
            .unwrap_or(refresh_timestamp);

        let mut result = tx
            .exec_iter(
                "DELETE FROM tree_spent_leaves WHERE user_id = ? AND spent_at < ?",
                (self.identity.clone(), cleanup_cutoff.naive_utc()),
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
        &self,
        tx: &mut mysql_async::Transaction<'_>,
        reservation_id: &str,
        leaves: &[TreeNode],
        purpose: ReservationPurpose,
        pending_change: u64,
    ) -> Result<(), TreeServiceError> {
        #[allow(clippy::cast_possible_wrap)]
        let pending_i64 = pending_change as i64;

        tx.exec_drop(
            "INSERT INTO tree_reservations (user_id, id, purpose, pending_change_amount) VALUES (?, ?, ?, ?)",
            (self.identity.clone(), reservation_id, purpose.to_string(), pending_i64),
        )
        .await
        .map_err(map_err)?;

        let leaf_ids: Vec<String> = leaves.iter().map(|l| l.id.to_string()).collect();
        debug!(
            "leaf_lifecycle reserve: reservation={} purpose={:?} leaf_ids={:?}",
            reservation_id, purpose, leaf_ids
        );
        self.batch_set_reservation_id(tx, reservation_id, &leaf_ids)
            .await?;

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
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_mysql_tree_store(
    config: MysqlStorageConfig,
    identity: &[u8],
) -> Result<Arc<dyn TreeStore>, MysqlError> {
    Ok(Arc::new(
        MysqlTreeStore::from_config(config, identity).await?,
    ))
}

/// Creates a `MysqlTreeStore` instance from an existing connection pool.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_mysql_tree_store_from_pool(
    pool: Pool,
    identity: &[u8],
) -> Result<Arc<dyn TreeStore>, MysqlError> {
    Ok(Arc::new(MysqlTreeStore::from_pool(pool, identity).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::tree_store_tests as shared_tests;
    use std::sync::Arc;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::mysql::Mysql;

    /// Fixed 33-byte test identity. Tests run in their own ephemeral container,
    /// so a single shared identity is fine — the schema still gets exercised.
    pub(super) const TEST_IDENTITY: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];

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

            let store = MysqlTreeStore::from_config(
                MysqlStorageConfig::with_defaults(connection_string),
                &TEST_IDENTITY,
            )
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

    #[tokio::test]
    async fn test_try_reserve_min_amount_with_leaves_above_individual_target() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_min_amount_with_leaves_above_individual_target(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn test_try_reserve_min_amount_exact_denominations_above_individual() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_min_amount_exact_denominations_above_individual(
            &fixture.store,
        )
        .await;
    }

    // ---- newly wired shared tests, parity with spark-postgres ----

    #[tokio::test]
    async fn test_add_leaves_clears_spent_status() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves_clears_spent_status(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_add_leaves_empty_slice() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves_empty_slice(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_add_leaves_not_deleted_by_set_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves_not_deleted_by_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_balance_change_notification() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_balance_change_notification(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_cancel_reservation_drops_all_when_keep_empty() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_cancel_reservation_drops_all_when_keep_empty(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_cancel_reservation_drops_unkept_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_cancel_reservation_drops_unkept_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_cancel_reservation_nonexistent() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_cancel_reservation_nonexistent(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_change_leaves_from_swap_protected() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_change_leaves_from_swap_protected(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_reservation_nonexistent() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_finalize_reservation_nonexistent(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_with_new_leaves_protected() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_finalize_with_new_leaves_protected(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_leaves_missing_operators_filters_spent() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_get_leaves_missing_operators_filters_spent(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_leaves_not_available() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_get_leaves_not_available(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_missing_operators_replaced_on_set_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_missing_operators_replaced_on_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_multiple_reservations() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_multiple_reservations(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_non_reservable_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_non_reservable_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_notification_after_swap_with_exact_amount() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_notification_after_swap_with_exact_amount(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_notification_on_pending_balance_change() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_notification_on_pending_balance_change(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_old_leaves_deleted_by_set_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_old_leaves_deleted_by_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_payment_reservation_excluded_from_balance() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_payment_reservation_excluded_from_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_pending_cleared_on_cancel() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_pending_cleared_on_cancel(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_pending_cleared_on_finalize() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_pending_cleared_on_finalize(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reservation_ids_are_unique() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_reservation_ids_are_unique(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_leaves_empty() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_reserve_leaves_empty(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_skips_non_available_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_reserve_skips_non_available_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_with_none_target_reserves_all() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_reserve_with_none_target_reserves_all(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves_preserves_reservations_for_in_flight_swaps() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_preserves_reservations_for_in_flight_swaps(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_set_leaves_proceeds_after_swap_when_refresh_starts_later() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_proceeds_after_swap_when_refresh_starts_later(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_set_leaves_replaces_fully() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_replaces_fully(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves_with_reservations() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_with_reservations(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_spent_ids_cleaned_up_when_no_longer_in_refresh() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_spent_ids_cleaned_up_when_no_longer_in_refresh(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_spent_leaves_not_restored_by_set_leaves() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_spent_leaves_not_restored_by_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_swap_reservation_included_in_balance() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_swap_reservation_included_in_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_try_reserve_fail_immediately_when_insufficient() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_fail_immediately_when_insufficient(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_try_reserve_insufficient_funds() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_insufficient_funds(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_try_reserve_success() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_success(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_try_reserve_wait_for_pending() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_wait_for_pending(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_update_reservation_clears_pending() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_update_reservation_clears_pending(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_update_reservation_nonexistent() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_update_reservation_nonexistent(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_update_reservation_preserves_purpose() {
        let fixture = MysqlTreeStoreTestFixture::new().await;
        shared_tests::test_update_reservation_preserves_purpose(&fixture.store).await;
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

    #[tokio::test]
    async fn test_finalize_reservation_blocked_by_write_lock() {
        // Regression: `finalize_reservation` must acquire the same named lock
        // as `set_leaves` to serialize them. Without the lock, a concurrent
        // set_leaves could read the spent_leaves snapshot before finalize
        // commits, then upsert the just-spent leaf back as Available.
        //
        // We assert the lock is acquired by holding it manually on a separate
        // connection and verifying that finalize blocks until we release.
        let fixture = MysqlTreeStoreTestFixture::new().await;
        let leaf = create_test_tree_node("locked_leaf", 100);
        fixture
            .store
            .add_leaves(std::slice::from_ref(&leaf))
            .await
            .unwrap();
        let reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Hold the per-tenant named lock on a separate connection so finalize
        // must wait. Must use the same lock name as `acquire_write_lock`.
        let lock_name = fixture.store.lock_name.clone();
        let mut holder = fixture.store.pool.get_conn().await.unwrap();
        let acquired: Option<i64> = holder
            .exec_first(
                "SELECT GET_LOCK(?, ?)",
                (lock_name.as_str(), WRITE_LOCK_TIMEOUT_SECS),
            )
            .await
            .unwrap();
        assert_eq!(acquired, Some(1), "holder failed to acquire the lock");

        let store = Arc::new(fixture.store);
        let store_for_task = store.clone();
        let res_id = reservation.id.clone();
        let finalize_task =
            tokio::spawn(async move { store_for_task.finalize_reservation(&res_id, None).await });

        // Without the fix, finalize would complete almost instantly. With it,
        // it must wait for the held lock.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            !finalize_task.is_finished(),
            "finalize_reservation completed while named lock was held — \
             the lock is not being acquired"
        );

        // Release the lock — finalize should complete shortly.
        holder
            .exec_drop("SELECT RELEASE_LOCK(?)", (lock_name.as_str(),))
            .await
            .unwrap();
        drop(holder);

        tokio::time::timeout(std::time::Duration::from_secs(5), finalize_task)
            .await
            .expect("finalize_reservation did not complete after lock released")
            .unwrap()
            .unwrap();

        let leaves = store.get_leaves().await.unwrap();
        assert!(
            !leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "locked_leaf"),
            "Spent leaf should not be Available"
        );
    }

    #[tokio::test]
    async fn test_fresh_reservation_not_cleaned_up() {
        // Test that fresh (non-stale) reservations are NOT cleaned up during set_leaves
        let fixture = MysqlTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Create a reservation (this will have a fresh created_at timestamp)
        let _reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Verify the reservation exists
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.reserved_for_payment.len(), 1);

        // Call set_leaves - should NOT clean up fresh reservation
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

        // Verify the fresh reservation was NOT cleaned up
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(
            all_leaves.reserved_for_payment.len(),
            1,
            "Fresh reservation should NOT be cleaned up"
        );
        assert_eq!(all_leaves.available.len(), 1);
    }

    #[tokio::test]
    async fn test_stale_swap_reservation_does_not_block_set_leaves() {
        // Regression test: a stale Swap reservation must be cleaned up before
        // has_active_swap is evaluated, otherwise the reservation pins itself in
        // place — set_leaves early-returns on has_active_swap, never reaches the
        // cleanup, and the wallet's leaf set freezes at the snapshot when the swap
        // started.
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
            ReservationPurpose::Swap,
        )
        .await
        .unwrap();

        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.reserved_for_swap.len(), 1);
        assert_eq!(all_leaves.available.len(), 1);

        // Backdate the swap reservation past the 5-minute timeout.
        let mut conn = fixture.store.pool.get_conn().await.unwrap();
        conn.exec_drop(
            "UPDATE tree_reservations SET created_at = DATE_SUB(NOW(6), INTERVAL 10 MINUTE) WHERE id = ?",
            (&reservation.id,),
        )
        .await
        .unwrap();
        drop(conn);

        // set_leaves brings fresh data from the operator that includes both leaves
        // plus a new one. Pre-fix: skipped on has_active_swap, the new leaf is lost
        // and the reservation lingers forever. Post-fix: cleanup runs first, the
        // stale reservation is dropped, has_active_swap is false, set_leaves applies.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let refresh_start = SystemTime::now();
        let refresh_leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        fixture
            .store
            .set_leaves(&refresh_leaves, &[], refresh_start)
            .await
            .unwrap();

        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(
            all_leaves.reserved_for_swap.is_empty(),
            "Stale swap reservation should be cleaned up"
        );
        assert_eq!(
            all_leaves.available.len(),
            3,
            "set_leaves should have proceeded and applied the operator's view (3 leaves)"
        );
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node3"),
            "node3 from the refresh should be present"
        );
    }

    #[tokio::test]
    async fn test_concurrent_reserve_cancel_cycle() {
        // Test rapid reserve/cancel cycles don't deadlock
        let fixture = MysqlTreeStoreTestFixture::new().await;
        let store = Arc::new(fixture.store);

        // Add leaves
        let mut leaves = Vec::new();
        for i in 0..20 {
            leaves.push(create_test_tree_node(&format!("node{i}"), 10));
        }
        store.add_leaves(&leaves).await.unwrap();

        // Spawn concurrent reserve/cancel cycles using JoinSet
        let mut join_set = tokio::task::JoinSet::new();
        for i in 0..5 {
            let store_clone = Arc::clone(&store);
            join_set.spawn(async move {
                for cycle in 0..3 {
                    let result = store_clone
                        .try_reserve_leaves(
                            Some(&TargetAmounts::new_amount_and_fee(10, None)),
                            true,
                            ReservationPurpose::Payment,
                        )
                        .await?;

                    if let ReserveResult::Success(reservation) = result {
                        store_clone
                            .cancel_reservation(&reservation.id, &reservation.leaves)
                            .await?;
                    }
                    tracing::debug!("Task {i} cycle {cycle} complete");
                }
                Ok::<_, TreeServiceError>((i, "completed cycles"))
            });
        }

        // Wait for all with global timeout
        tokio::time::timeout(std::time::Duration::from_mins(1), async {
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(Ok((i, msg))) => tracing::info!("Task {i}: {msg}"),
                    Ok(Err(e)) => panic!("Task failed with error: {e:?}"),
                    Err(e) => panic!("Task panicked: {e:?}"),
                }
            }
        })
        .await
        .expect("Test timed out - possible deadlock");
    }

    #[tokio::test]
    async fn test_concurrent_set_leaves_and_reserve() {
        // Test that concurrent set_leaves and reserve operations don't deadlock
        let fixture = MysqlTreeStoreTestFixture::new().await;
        let store = Arc::new(fixture.store);

        // Add initial leaves
        let mut leaves = Vec::new();
        for i in 0..50 {
            leaves.push(create_test_tree_node(&format!("node{i}"), 10));
        }
        store.add_leaves(&leaves).await.unwrap();

        // Small delay to ensure leaves are added
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Spawn concurrent operations using JoinSet
        let mut join_set = tokio::task::JoinSet::new();

        // Spawn set_leaves tasks
        for i in 0..2 {
            let store_clone = Arc::clone(&store);
            join_set.spawn(async move {
                let refresh_start = SystemTime::now();
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;

                let mut new_leaves = Vec::new();
                for j in 0..50 {
                    new_leaves.push(create_test_tree_node(&format!("node{j}"), 10));
                }

                store_clone
                    .set_leaves(&new_leaves, &[], refresh_start)
                    .await
                    .map(|()| (i, "set_leaves complete"))
            });
        }

        // Spawn reserve tasks
        for i in 0..5 {
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
                    Ok(ReserveResult::Success(reservation)) => {
                        store_clone
                            .cancel_reservation(&reservation.id, &reservation.leaves)
                            .await?;
                        Ok((100 + i, "reserve success"))
                    }
                    Ok(_) => Ok((100 + i, "no leaves available")),
                    Err(e) => Err(e),
                }
            });
        }

        // Wait for all with global timeout
        tokio::time::timeout(std::time::Duration::from_mins(1), async {
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(Ok((i, msg))) => tracing::info!("Task {i}: {msg}"),
                    Ok(Err(e)) => panic!("Task failed with error: {e:?}"),
                    Err(e) => panic!("Task panicked: {e:?}"),
                }
            }
        })
        .await
        .expect("Test timed out - possible deadlock");
    }

    #[tokio::test]
    #[allow(clippy::arithmetic_side_effects)]
    async fn test_high_concurrency_reserve_finalize() {
        // Stress test: 50 concurrent payment-like operations (reserve -> finalize)
        // This simulates the parallel_perf benchmark scenario.
        let fixture = MysqlTreeStoreTestFixture::new().await;
        let store = Arc::new(fixture.store);

        // Add many small leaves
        let mut leaves = Vec::new();
        for i in 0..200 {
            leaves.push(create_test_tree_node(&format!("leaf{i}"), 1));
        }
        store.add_leaves(&leaves).await.unwrap();

        // Spawn 50 concurrent reserve->finalize operations
        let start_time = std::time::Instant::now();
        let mut join_set: tokio::task::JoinSet<Result<(i32, &'static str), TreeServiceError>> =
            tokio::task::JoinSet::new();
        for i in 0..50 {
            let store_clone = Arc::clone(&store);
            join_set.spawn(async move {
                // Reserve 1 sat
                let result = store_clone
                    .try_reserve_leaves(
                        Some(&TargetAmounts::new_amount_and_fee(1, None)),
                        true,
                        ReservationPurpose::Payment,
                    )
                    .await?;

                match result {
                    ReserveResult::Success(reservation) => {
                        // Finalize immediately (simulating successful payment)
                        store_clone
                            .finalize_reservation(&reservation.id, None)
                            .await?;
                        Ok((i, "success"))
                    }
                    ReserveResult::InsufficientFunds => Ok((i, "insufficient")),
                    ReserveResult::WaitForPending { .. } => Ok((i, "wait_pending")),
                }
            });
        }

        // Wait for all with timeout
        let mut successes = 0;
        let mut insufficient = 0;
        let timeout_result = tokio::time::timeout(std::time::Duration::from_mins(2), async {
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(Ok((i, status))) => {
                        tracing::debug!("Task {i}: {status}");
                        if status == "success" {
                            successes += 1;
                        } else if status == "insufficient" {
                            insufficient += 1;
                        }
                    }
                    Ok(Err(e)) => panic!("Task failed with error: {e:?}"),
                    Err(e) => panic!("Task panicked: {e:?}"),
                }
            }
            (successes, insufficient)
        })
        .await
        .expect("Test timed out after 120s - possible deadlock");

        let elapsed = start_time.elapsed();
        eprintln!(
            "50 concurrent reserve+finalize completed in {:?} ({} successes, {} insufficient)",
            elapsed, timeout_result.0, timeout_result.1
        );

        // With 200 leaves and 50 concurrent requests for 1 sat each,
        // we should have at least some successes
        assert!(
            timeout_result.0 > 0,
            "Expected at least one successful reservation"
        );
    }

    // ==================== Multi-tenant isolation ====================

    /// A second 33-byte test identity (must differ from `TEST_IDENTITY`).
    const TEST_IDENTITY_B: [u8; 33] = [
        0x03, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae,
        0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd,
        0xbe, 0xbf, 0xc0,
    ];

    /// Two `MysqlTreeStore` instances with distinct identities sharing one
    /// connection pool / DB. The container must be kept alive for the test.
    struct TwoTenantTreeFixture {
        a: MysqlTreeStore,
        b: MysqlTreeStore,
        #[allow(dead_code)]
        container: ContainerAsync<Mysql>,
    }

    impl TwoTenantTreeFixture {
        async fn new() -> Self {
            let container = Mysql::default()
                .start()
                .await
                .expect("Failed to start MySQL container");

            let host_port = container
                .get_host_port_ipv4(3306)
                .await
                .expect("Failed to get host port");

            let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

            let config = MysqlStorageConfig::with_defaults(connection_string);
            let pool = create_pool(&config).expect("Failed to create pool");

            let a = MysqlTreeStore::from_pool(pool.clone(), &TEST_IDENTITY)
                .await
                .expect("Failed to create tenant A");
            let b = MysqlTreeStore::from_pool(pool, &TEST_IDENTITY_B)
                .await
                .expect("Failed to create tenant B");

            Self { a, b, container }
        }
    }

    /// End-to-end isolation: every `TreeStore` method must keep tenants A and B
    /// from observing each other's data. Exercises leaves, reservations, the
    /// finalize→spent path, and the per-tenant swap-status row, asserting that
    /// writes by A are invisible to B (and vice versa). Uses identical leaf
    /// IDs across tenants to also confirm the composite primary keys land.
    #[tokio::test]
    async fn test_two_tenant_isolation() {
        let fx = TwoTenantTreeFixture::new().await;

        // --- add_leaves: same IDs, different per-tenant values ---
        let leaves_a = vec![
            create_test_tree_node("shared_leaf_1", 100),
            create_test_tree_node("shared_leaf_2", 200),
        ];
        let leaves_b = vec![
            create_test_tree_node("shared_leaf_1", 1_000),
            create_test_tree_node("shared_leaf_2", 2_000),
            create_test_tree_node("only_b_leaf", 500),
        ];
        fx.a.add_leaves(&leaves_a).await.unwrap();
        fx.b.add_leaves(&leaves_b).await.unwrap();

        let view_a = fx.a.get_leaves().await.unwrap();
        let view_b = fx.b.get_leaves().await.unwrap();
        assert_eq!(view_a.available.len(), 2, "A sees its 2 leaves");
        assert_eq!(view_b.available.len(), 3, "B sees its 3 leaves");
        assert!(
            !view_a
                .available
                .iter()
                .any(|l| l.id.to_string() == "only_b_leaf")
        );
        assert_eq!(
            view_a.available.iter().map(|l| l.value).sum::<u64>(),
            300,
            "A's total reflects A's amounts only"
        );
        assert_eq!(
            view_b.available.iter().map(|l| l.value).sum::<u64>(),
            3_500,
            "B's total reflects B's amounts only"
        );

        assert_eq!(fx.a.get_available_balance().await.unwrap(), 300);
        assert_eq!(fx.b.get_available_balance().await.unwrap(), 3_500);

        // --- reserve_leaves on A must not consume B's leaves ---
        let res_a = shared_tests::reserve_leaves(
            &fx.a,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
        assert_eq!(res_a.leaves.len(), 1);

        let view_b = fx.b.get_leaves().await.unwrap();
        assert!(view_b.reserved_for_payment.is_empty());
        assert_eq!(view_b.available.len(), 3);
        assert_eq!(fx.b.get_available_balance().await.unwrap(), 3_500);

        let view_a = fx.a.get_leaves().await.unwrap();
        assert_eq!(view_a.reserved_for_payment.len(), 1);
        assert_eq!(view_a.available.len(), 1);

        // --- finalize on A (spent marker) does not touch B's identical-ID leaf ---
        fx.a.finalize_reservation(&res_a.id, None).await.unwrap();

        let view_a = fx.a.get_leaves().await.unwrap();
        assert!(
            !view_a
                .available
                .iter()
                .any(|l| l.id.to_string() == "shared_leaf_1")
        );
        let view_b = fx.b.get_leaves().await.unwrap();
        assert!(
            view_b
                .available
                .iter()
                .any(|l| l.id.to_string() == "shared_leaf_1"),
            "B's shared_leaf_1 must survive A's finalize"
        );

        // --- set_leaves on A must not touch B's data ---
        let refresh_start = SystemTime::now();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let refresh_a = vec![create_test_tree_node("a_only_post_refresh", 42)];
        fx.a.set_leaves(&refresh_a, &[], refresh_start)
            .await
            .unwrap();

        let view_a = fx.a.get_leaves().await.unwrap();
        assert_eq!(view_a.available.len(), 1);
        assert_eq!(view_a.available[0].id.to_string(), "a_only_post_refresh");
        let view_b = fx.b.get_leaves().await.unwrap();
        assert_eq!(view_b.available.len(), 3, "B unaffected by A's set_leaves");

        // --- B can perform a swap-finalize without A's swap-status row ever
        // existing (lazy upsert per tenant). B's reservation must not block A. ---
        let res_b_swap = shared_tests::reserve_leaves(
            &fx.b,
            Some(&TargetAmounts::new_amount_and_fee(1_000, None)),
            true,
            ReservationPurpose::Swap,
        )
        .await
        .unwrap();
        let view_a = fx.a.get_leaves().await.unwrap();
        assert!(
            view_a.reserved_for_swap.is_empty(),
            "A must not see B's swap reservation"
        );
        let new_b_leaf = create_test_tree_node("b_swap_change", 600);
        fx.b.finalize_reservation(&res_b_swap.id, Some(&[new_b_leaf]))
            .await
            .unwrap();

        let view_a = fx.a.get_leaves().await.unwrap();
        assert_eq!(view_a.available.len(), 1);
        assert_eq!(view_a.available[0].id.to_string(), "a_only_post_refresh");
    }
}
