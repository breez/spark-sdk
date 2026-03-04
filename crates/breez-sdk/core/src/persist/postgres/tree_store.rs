//! `PostgreSQL`-backed implementation of the `TreeStore` trait.
//!
//! This module provides a persistent tree store backed by `PostgreSQL`,
//! suitable for server-side or multi-instance deployments where
//! in-memory storage is insufficient.

use std::collections::HashSet;
use std::sync::Arc;

use web_time::SystemTime;

use deadpool_postgres::Pool;
use macros::async_trait;
use spark_wallet::{
    Leaves, LeavesReservation, LeavesReservationId, ReservationPurpose, ReserveResult,
    TargetAmounts, TreeNode, TreeNodeStatus, TreeServiceError, TreeStore,
    select_leaves_by_minimum_amount, select_leaves_by_target_amounts,
};
use tokio::sync::watch;
use tracing::trace;
use uuid::Uuid;

use crate::persist::StorageError;

use super::base::{PostgresStorageConfig, create_pool, run_migrations};

/// Name of the schema migrations table for `PostgresTreeStore`.
const TREE_MIGRATIONS_TABLE: &str = "tree_schema_migrations";

/// Advisory lock key for serializing tree store write operations.
/// This prevents deadlocks by ensuring only one write transaction runs at a time.
/// The lock is automatically released when the transaction commits or rolls back.
const TREE_STORE_WRITE_LOCK_KEY: i64 = 0x7472_6565_5354_4f52; // "treeTOR" as hex

/// Timeout for reservations in seconds. Reservations older than this are considered stale
/// and will be cleaned up during `set_leaves()` to release leaves locked by crashed clients.
const RESERVATION_TIMEOUT_SECS: f64 = 300.0; // 5 minutes

/// Threshold in milliseconds for cleaning up spent leaf markers.
/// Spent markers are kept in the database for this duration to support multiple
/// SDK instances sharing the same postgres database. During `set_leaves`, spent
/// markers older than `refresh_timestamp` are ignored (treated as deleted).
/// Actual deletion only happens for markers older than this threshold.
const SPENT_MARKER_CLEANUP_THRESHOLD_MS: i64 = 5 * 60 * 1000; // 5 minutes

/// `PostgreSQL`-backed tree store implementation.
///
/// This implementation uses database-level concurrency control (row locking)
/// to safely handle concurrent operations, making it suitable for multi-instance
/// deployments.
pub(crate) struct PostgresTreeStore {
    pool: Pool,
    balance_changed_tx: Arc<watch::Sender<()>>,
    balance_changed_rx: watch::Receiver<()>,
}

#[async_trait]
impl TreeStore for PostgresTreeStore {
    async fn add_leaves(&self, leaves: &[TreeNode]) -> Result<(), TreeServiceError> {
        if leaves.is_empty() {
            return Ok(());
        }

        tracing::trace!(
            "PostgresTreeStore::add_leaves: adding {} leaves",
            leaves.len()
        );
        for leaf in leaves {
            tracing::trace!(
                "PostgresTreeStore::add_leaves: leaf {} owner={:?} value={} status={:?}",
                leaf.id,
                leaf.owner_identity_public_key,
                leaf.value,
                leaf.status
            );
        }

        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        // Acquire advisory lock to prevent deadlocks with concurrent operations
        Self::acquire_write_lock(&tx).await?;

        // Remove these leaves from spent_leaves table - when we receive a leaf through
        // add_leaves (e.g., from a claimed transfer), it's no longer "spent" from
        // our perspective. This handles the case where the same leaf returns to us
        // after we sent it to someone else.
        let leaf_ids: Vec<String> = leaves.iter().map(|l| l.id.to_string()).collect();
        Self::batch_remove_spent_leaves(&tx, &leaf_ids).await?;

        // Batch insert all leaves (no filtering needed since we just removed any
        // that were in spent_leaves)
        Self::batch_upsert_leaves(&tx, leaves, false, None).await?;

        tx.commit().await.map_err(map_err)?;
        tracing::trace!(
            "PostgresTreeStore::add_leaves: committed {} leaves",
            leaves.len()
        );
        self.notify_balance_change();
        Ok(())
    }

    async fn get_leaves(&self) -> Result<Leaves, TreeServiceError> {
        let client = self.pool.get().await.map_err(map_err)?;

        let rows = client
            .query(
                r"
                SELECT l.id, l.status, l.is_missing_from_operators, l.data,
                       l.reservation_id, r.purpose
                FROM tree_leaves l
                LEFT JOIN tree_reservations r ON l.reservation_id = r.id
                ",
                &[],
            )
            .await
            .map_err(map_err)?;

        let mut available = Vec::new();
        let mut not_available = Vec::new();
        let mut available_missing_from_operators = Vec::new();
        let mut reserved_for_payment = Vec::new();
        let mut reserved_for_swap = Vec::new();

        for row in rows {
            let data: serde_json::Value = row.get("data");
            let node = Self::deserialize_node(data)?;
            let is_missing: bool = row.get("is_missing_from_operators");
            let purpose: Option<String> = row.get("purpose");

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
        // Convert SystemTime to chrono for PostgreSQL
        let refresh_timestamp: chrono::DateTime<chrono::Utc> = refresh_started_at.into();

        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        // Acquire advisory lock to prevent deadlocks with concurrent operations
        Self::acquire_write_lock(&tx).await?;

        // Check if any swap reservation is currently active, or if a swap completed
        // after this refresh started (making the refresh data potentially inconsistent).
        let (has_active_swap, swap_completed_during_refresh): (bool, bool) = {
            let row = tx
                .query_one(
                    r"
                    SELECT
                        EXISTS(SELECT 1 FROM tree_reservations WHERE purpose = 'Swap'),
                        COALESCE((SELECT last_completed_at >= $1 FROM tree_swap_status WHERE id = 1), FALSE)
                    ",
                    &[&refresh_timestamp],
                )
                .await
                .map_err(map_err)?;
            (row.get(0), row.get(1))
        };

        if has_active_swap || swap_completed_during_refresh {
            trace!(
                "Skipping set_leaves: active_swap={}, swap_completed_during_refresh={}",
                has_active_swap, swap_completed_during_refresh
            );
            return Ok(());
        }

        // Clean up old spent markers (older than threshold).
        // We don't immediately delete all spent markers because this postgres store
        // may be shared by multiple SDK instances. Instead, we keep markers for a
        // threshold period and ignore (treat as deleted) markers where spent_at < refresh_timestamp.
        Self::cleanup_spent_markers(&tx, refresh_timestamp).await?;

        // Get recent spent leaf IDs (spent_at >= refresh_timestamp).
        // Older spent markers are ignored - if the refresh started after the spend,
        // operators had time to process it. A spent leaf won't appear in `leaves`
        // (coordinator processed the spend), it may only appear in `missing_operators_leaves`
        // if returned (e.g., HTLC refund).
        let spent_ids: HashSet<String> = {
            let rows = tx
                .query(
                    "SELECT leaf_id FROM tree_spent_leaves WHERE spent_at >= $1",
                    &[&refresh_timestamp],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get(0)).collect()
        };

        // Delete non-reserved leaves that were added BEFORE refresh started.
        // The advisory lock acquired at the start of this transaction prevents deadlocks.
        tx.execute(
            "DELETE FROM tree_leaves WHERE reservation_id IS NULL AND added_at < $1",
            &[&refresh_timestamp],
        )
        .await
        .map_err(map_err)?;

        // Clean up stale reservations from crashed clients.
        // This MUST be done AFTER the leaf delete above, because DELETE on tree_reservations
        // can affect tree_leaves through the ON DELETE SET NULL foreign key constraint,
        // which interferes with the timestamp-based leaf deletion.
        Self::cleanup_stale_reservations(&tx).await?;

        // Upsert all leaves. batch_upsert_leaves handles spent filtering via skip_ids,
        // and its ON CONFLICT clause preserves reservation_id (not in the UPDATE SET list).
        // Reserved leaves are also immune to timestamp-based deletion (WHERE reservation_id IS NULL).
        Self::batch_upsert_leaves(&tx, leaves, false, Some(&spent_ids)).await?;
        Self::batch_upsert_leaves(&tx, missing_operators_leaves, true, Some(&spent_ids)).await?;

        tx.commit().await.map_err(map_err)?;
        self.notify_balance_change();
        Ok(())
    }

    async fn cancel_reservation(&self, id: &LeavesReservationId) -> Result<(), TreeServiceError> {
        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        // Acquire advisory lock to prevent deadlocks with concurrent operations
        Self::acquire_write_lock(&tx).await?;

        // Check if reservation exists (advisory lock provides serialization, no row locking needed)
        let reservation = tx
            .query_opt("SELECT id FROM tree_reservations WHERE id = $1", &[id])
            .await
            .map_err(map_err)?;

        if reservation.is_none() {
            // Already cancelled or finalized
            return Ok(());
        }

        // Delete the reservation (ON DELETE SET NULL will clear reservation_id on leaves)
        tx.execute("DELETE FROM tree_reservations WHERE id = $1", &[id])
            .await
            .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;
        trace!("Cancelled reservation: {id}");
        self.notify_balance_change();
        Ok(())
    }

    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        // Acquire advisory lock to prevent deadlocks with concurrent operations
        Self::acquire_write_lock(&tx).await?;

        // Check if reservation exists and get its purpose
        let reservation = tx
            .query_opt(
                "SELECT id, purpose FROM tree_reservations WHERE id = $1",
                &[id],
            )
            .await
            .map_err(map_err)?;

        let Some(reservation_row) = reservation else {
            // Already finalized or cancelled - match in-memory behavior by returning Ok
            return Ok(());
        };

        let is_swap = reservation_row.get::<_, String>("purpose") == "Swap";

        // Get reserved leaf IDs and mark as spent.
        // The advisory lock prevents concurrent modifications.
        let reserved_leaf_ids: Vec<String> = {
            let rows = tx
                .query(
                    "SELECT id FROM tree_leaves WHERE reservation_id = $1",
                    &[id],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get(0)).collect()
        };

        // Batch insert spent leaf markers
        Self::batch_insert_spent_leaves(&tx, &reserved_leaf_ids).await?;

        // Delete reserved leaves and reservation
        tx.execute("DELETE FROM tree_leaves WHERE reservation_id = $1", &[id])
            .await
            .map_err(map_err)?;

        tx.execute("DELETE FROM tree_reservations WHERE id = $1", &[id])
            .await
            .map_err(map_err)?;

        // Batch upsert new leaves if provided (with fresh timestamp for race condition fix)
        if let Some(leaves) = new_leaves {
            Self::batch_upsert_leaves(&tx, leaves, false, None).await?;
        }

        // If this was a swap with new leaves, update last_completed_at.
        // This is used to detect if a refresh started before a swap finished,
        // which would cause stale data to be applied.
        if is_swap && new_leaves.is_some() {
            tx.execute(
                "UPDATE tree_swap_status SET last_completed_at = NOW() WHERE id = 1",
                &[],
            )
            .await
            .map_err(map_err)?;
        }

        tx.commit().await.map_err(map_err)?;
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

        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        // Acquire advisory lock to prevent deadlocks with concurrent operations
        Self::acquire_write_lock(&tx).await?;

        // Get available leaves (advisory lock provides serialization, no row locking needed)
        let rows = tx
            .query(
                r"
                SELECT data
                FROM tree_leaves
                WHERE status = 'Available'
                  AND is_missing_from_operators = FALSE
                  AND reservation_id IS NULL
                ",
                &[],
            )
            .await
            .map_err(map_err)?;

        let available_leaves: Vec<TreeNode> = rows
            .iter()
            .map(|r| Self::deserialize_node(r.get("data")))
            .collect::<Result<Vec<_>, _>>()?;

        tracing::trace!(
            "PostgresTreeStore::try_reserve_leaves: found {} available leaves",
            available_leaves.len()
        );
        for leaf in &available_leaves {
            tracing::trace!(
                "PostgresTreeStore::try_reserve_leaves: available leaf {} owner={:?} value={}",
                leaf.id,
                leaf.owner_identity_public_key,
                leaf.value
            );
        }

        let available: u64 = available_leaves.iter().map(|l| l.value).sum();
        // Calculate pending balance within the same transaction for consistency
        let pending = Self::calculate_pending_balance(&tx).await?;

        // Try exact selection first
        let selected = select_leaves_by_target_amounts(&available_leaves, target_amounts);

        match selected {
            Ok(target_leaves) => {
                let selected_leaves = [
                    target_leaves.amount_leaves,
                    target_leaves.fee_leaves.unwrap_or_default(),
                ]
                .concat();

                // Reject empty reservations (matches in-memory behavior)
                if selected_leaves.is_empty() {
                    return Err(TreeServiceError::NonReservableLeaves);
                }

                self.create_reservation(&tx, &reservation_id, &selected_leaves, purpose, 0)
                    .await?;

                tx.commit().await.map_err(map_err)?;
                self.notify_balance_change();
                Ok(ReserveResult::Success(LeavesReservation::new(
                    selected_leaves,
                    reservation_id,
                )))
            }
            Err(_) if !exact_only => {
                // Try minimum amount selection
                if let Ok(Some(selected_leaves)) =
                    select_leaves_by_minimum_amount(&available_leaves, target_amount)
                {
                    let reserved_amount: u64 = selected_leaves.iter().map(|l| l.value).sum();
                    let pending_change = if reserved_amount > target_amount && target_amount > 0 {
                        reserved_amount - target_amount
                    } else {
                        0
                    };

                    self.create_reservation(
                        &tx,
                        &reservation_id,
                        &selected_leaves,
                        purpose,
                        pending_change,
                    )
                    .await?;

                    tx.commit().await.map_err(map_err)?;
                    self.notify_balance_change();
                    return Ok(ReserveResult::Success(LeavesReservation::new(
                        selected_leaves,
                        reservation_id,
                    )));
                }

                // No suitable leaves found
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
            Err(_) => {
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
        }
    }

    async fn now(&self) -> Result<SystemTime, TreeServiceError> {
        let client = self.pool.get().await.map_err(map_err)?;
        let row = client
            .query_one("SELECT NOW()", &[])
            .await
            .map_err(map_err)?;
        let now: chrono::DateTime<chrono::Utc> = row.get(0);
        Ok(now.into())
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
        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        // Acquire advisory lock to prevent deadlocks with concurrent operations
        Self::acquire_write_lock(&tx).await?;

        // Check if reservation exists (advisory lock provides serialization, no row locking needed)
        let reservation = tx
            .query_opt(
                "SELECT id FROM tree_reservations WHERE id = $1",
                &[reservation_id],
            )
            .await
            .map_err(map_err)?;

        if reservation.is_none() {
            return Err(TreeServiceError::Generic(format!(
                "Reservation {reservation_id} not found"
            )));
        }

        // Get old reserved leaf IDs and mark them as spent (they were consumed by the swap)
        let old_reserved_leaf_ids: Vec<String> = {
            let rows = tx
                .query(
                    "SELECT id FROM tree_leaves WHERE reservation_id = $1",
                    &[reservation_id],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get(0)).collect()
        };

        // Mark old leaves as spent and delete them (they no longer exist after the swap)
        Self::batch_insert_spent_leaves(&tx, &old_reserved_leaf_ids).await?;
        tx.execute(
            "DELETE FROM tree_leaves WHERE reservation_id = $1",
            &[reservation_id],
        )
        .await
        .map_err(map_err)?;

        // Batch upsert change leaves to available pool with fresh timestamp (race condition fix)
        Self::batch_upsert_leaves(&tx, change_leaves, false, None).await?;

        // Batch upsert reserved leaves with fresh timestamp
        Self::batch_upsert_leaves(&tx, reserved_leaves, false, None).await?;

        // Set reservation_id on reserved leaves
        let leaf_ids: Vec<String> = reserved_leaves.iter().map(|l| l.id.to_string()).collect();
        Self::batch_set_reservation_id(&tx, reservation_id, &leaf_ids).await?;

        // Clear pending change amount
        tx.execute(
            "UPDATE tree_reservations SET pending_change_amount = 0 WHERE id = $1",
            &[reservation_id],
        )
        .await
        .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;

        trace!(
            "Updated reservation {}: reserved {} leaves, added {} change leaves",
            reservation_id,
            reserved_leaves.len(),
            change_leaves.len()
        );

        self.notify_balance_change();
        Ok(LeavesReservation::new(
            reserved_leaves.to_vec(),
            reservation_id.clone(),
        ))
    }
}

impl PostgresTreeStore {
    /// Creates a new `PostgresTreeStore`.
    pub async fn new(config: PostgresStorageConfig) -> Result<Self, StorageError> {
        let pool = create_pool(&config)?;

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

    /// Runs database migrations for tree store tables.
    async fn migrate(&self) -> Result<(), StorageError> {
        run_migrations(&self.pool, TREE_MIGRATIONS_TABLE, &Self::migrations()).await
    }

    /// Returns the list of migrations for the tree store.
    fn migrations() -> Vec<&'static [&'static str]> {
        vec![
            // Migration 1: Initial tree tables
            &[
                "CREATE TABLE IF NOT EXISTS tree_reservations (
                    id TEXT PRIMARY KEY,
                    purpose TEXT NOT NULL,
                    pending_change_amount BIGINT NOT NULL DEFAULT 0,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )",
                "CREATE TABLE IF NOT EXISTS tree_leaves (
                    id TEXT PRIMARY KEY,
                    status TEXT NOT NULL,
                    is_missing_from_operators BOOLEAN NOT NULL DEFAULT FALSE,
                    reservation_id TEXT REFERENCES tree_reservations(id) ON DELETE SET NULL,
                    data JSONB NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )",
                "CREATE TABLE IF NOT EXISTS tree_spent_leaves (
                    leaf_id TEXT PRIMARY KEY,
                    spent_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )",
                "CREATE INDEX IF NOT EXISTS idx_tree_leaves_available ON tree_leaves(status, is_missing_from_operators)
                    WHERE status = 'Available' AND is_missing_from_operators = FALSE",
                "CREATE INDEX IF NOT EXISTS idx_tree_leaves_reservation ON tree_leaves(reservation_id)
                    WHERE reservation_id IS NOT NULL",
                "CREATE INDEX IF NOT EXISTS idx_tree_leaves_added_at ON tree_leaves(added_at)",
            ],
            // Migration 2: Add swap status tracking for race condition fix
            &[
                "CREATE TABLE IF NOT EXISTS tree_swap_status (
                    id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
                    last_completed_at TIMESTAMPTZ
                )",
                "INSERT INTO tree_swap_status (id) VALUES (1) ON CONFLICT DO NOTHING",
            ],
        ]
    }

    /// Notifies balance change watchers that a balance change occurred.
    /// Sends an empty notification - subscribers only use this as a trigger
    /// to re-check the balance, not the actual value.
    fn notify_balance_change(&self) {
        // Just send a notification without calculating the balance.
        // This saves a database query and pool connection.
        let _ = self.balance_changed_tx.send(());
    }

    /// Calculates the pending balance from in-flight swaps within a transaction.
    async fn calculate_pending_balance(
        tx: &tokio_postgres::Transaction<'_>,
    ) -> Result<u64, TreeServiceError> {
        let row = tx
            .query_one(
                "SELECT COALESCE(SUM(pending_change_amount), 0)::BIGINT FROM tree_reservations",
                &[],
            )
            .await
            .map_err(map_err)?;

        let pending: i64 = row.get(0);
        Ok(u64::try_from(pending).unwrap_or(0))
    }

    /// Serializes a `TreeNode` to JSON.
    fn serialize_node(node: &TreeNode) -> Result<serde_json::Value, TreeServiceError> {
        serde_json::to_value(node)
            .map_err(|e| TreeServiceError::Generic(format!("Failed to serialize TreeNode: {e}")))
    }

    /// Deserializes a `TreeNode` from JSON.
    fn deserialize_node(data: serde_json::Value) -> Result<TreeNode, TreeServiceError> {
        serde_json::from_value(data)
            .map_err(|e| TreeServiceError::Generic(format!("Failed to deserialize TreeNode: {e}")))
    }

    /// Batch upserts leaves into `tree_leaves` table using UNNEST.
    /// Optionally skips leaves whose IDs are in the `skip_ids` set.
    /// Uses ON CONFLICT DO UPDATE to replace existing leaves (matching `InMemoryTreeStore` behavior).
    async fn batch_upsert_leaves(
        tx: &tokio_postgres::Transaction<'_>,
        leaves: &[TreeNode],
        is_missing_from_operators: bool,
        skip_ids: Option<&HashSet<String>>,
    ) -> Result<(), TreeServiceError> {
        let filtered: Vec<&TreeNode> = if let Some(skip) = skip_ids {
            leaves
                .iter()
                .filter(|l| !skip.contains(&l.id.to_string()))
                .collect()
        } else {
            leaves.iter().collect()
        };

        if filtered.is_empty() {
            return Ok(());
        }

        let mut ids: Vec<String> = Vec::with_capacity(filtered.len());
        let mut statuses: Vec<String> = Vec::with_capacity(filtered.len());
        let mut missing_flags: Vec<bool> = Vec::with_capacity(filtered.len());
        let mut data_values: Vec<serde_json::Value> = Vec::with_capacity(filtered.len());

        for leaf in filtered {
            ids.push(leaf.id.to_string());
            statuses.push(leaf.status.to_string());
            missing_flags.push(is_missing_from_operators);
            data_values.push(Self::serialize_node(leaf)?);
        }

        tx.execute(
            r"
            INSERT INTO tree_leaves (id, status, is_missing_from_operators, data, added_at)
            SELECT id, status, missing, data, NOW()
            FROM UNNEST($1::text[], $2::text[], $3::bool[], $4::jsonb[])
                AS t(id, status, missing, data)
            ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status,
                is_missing_from_operators = EXCLUDED.is_missing_from_operators,
                data = EXCLUDED.data,
                added_at = NOW()
            ",
            &[&ids, &statuses, &missing_flags, &data_values],
        )
        .await
        .map_err(map_err)?;

        Ok(())
    }

    /// Batch sets `reservation_id` on leaves using UNNEST.
    async fn batch_set_reservation_id(
        tx: &tokio_postgres::Transaction<'_>,
        reservation_id: &str,
        leaf_ids: &[String],
    ) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }

        tx.execute(
            r"
            UPDATE tree_leaves
            SET reservation_id = $1
            WHERE id = ANY($2)
            ",
            &[&reservation_id, &leaf_ids],
        )
        .await
        .map_err(map_err)?;

        Ok(())
    }

    /// Batch inserts spent leaf markers using UNNEST.
    async fn batch_insert_spent_leaves(
        tx: &tokio_postgres::Transaction<'_>,
        leaf_ids: &[String],
    ) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }

        tx.execute(
            r"
            INSERT INTO tree_spent_leaves (leaf_id)
            SELECT * FROM UNNEST($1::text[])
            ON CONFLICT DO NOTHING
            ",
            &[&leaf_ids],
        )
        .await
        .map_err(map_err)?;

        Ok(())
    }

    /// Batch removes spent leaf markers using UNNEST.
    /// This is called when receiving a leaf back (e.g., from a claimed transfer)
    /// to clear the "spent" status from when we previously sent it.
    async fn batch_remove_spent_leaves(
        tx: &tokio_postgres::Transaction<'_>,
        leaf_ids: &[String],
    ) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }

        let result = tx
            .execute(
                r"
                DELETE FROM tree_spent_leaves
                WHERE leaf_id = ANY($1)
                ",
                &[&leaf_ids],
            )
            .await
            .map_err(map_err)?;

        if result > 0 {
            trace!(
                "Removed {} leaves from spent_leaves (receiving them back)",
                result
            );
        }

        Ok(())
    }

    /// Acquires an exclusive advisory lock for write operations.
    /// This serializes all tree store writes to prevent deadlocks.
    /// The lock is automatically released when the transaction commits or rolls back.
    async fn acquire_write_lock(
        tx: &tokio_postgres::Transaction<'_>,
    ) -> Result<(), TreeServiceError> {
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&TREE_STORE_WRITE_LOCK_KEY],
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }

    /// Deletes reservations that have exceeded the timeout.
    /// Called during `set_leaves` to clean up stale reservations from crashed clients.
    /// The `ON DELETE SET NULL` foreign key constraint automatically releases the leaves.
    async fn cleanup_stale_reservations(
        tx: &tokio_postgres::Transaction<'_>,
    ) -> Result<u64, TreeServiceError> {
        let result = tx
            .execute(
                r"DELETE FROM tree_reservations
                  WHERE created_at < NOW() - make_interval(secs => $1)",
                &[&RESERVATION_TIMEOUT_SECS],
            )
            .await
            .map_err(map_err)?;

        if result > 0 {
            trace!("Cleaned up {} stale reservations", result);
        }

        Ok(result)
    }

    /// Cleans up old spent markers that are older than the cleanup threshold.
    /// We keep spent markers for a threshold period to support multiple SDK instances
    /// sharing the same postgres database. During `set_leaves`, spent markers where
    /// `spent_at < refresh_timestamp` are ignored (treated as deleted) but not actually
    /// removed until they exceed this threshold.
    async fn cleanup_spent_markers(
        tx: &tokio_postgres::Transaction<'_>,
        refresh_timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, TreeServiceError> {
        let threshold = chrono::Duration::milliseconds(SPENT_MARKER_CLEANUP_THRESHOLD_MS);
        let cleanup_cutoff = refresh_timestamp
            .checked_sub_signed(threshold)
            .unwrap_or(refresh_timestamp);

        let result = tx
            .execute(
                r"DELETE FROM tree_spent_leaves WHERE spent_at < $1",
                &[&cleanup_cutoff],
            )
            .await
            .map_err(map_err)?;

        if result > 0 {
            trace!("Cleaned up {} spent markers", result);
        }

        Ok(result)
    }
}

impl PostgresTreeStore {
    /// Creates a reservation with the given leaves.
    async fn create_reservation(
        &self,
        tx: &tokio_postgres::Transaction<'_>,
        reservation_id: &str,
        leaves: &[TreeNode],
        purpose: ReservationPurpose,
        pending_change: u64,
    ) -> Result<(), TreeServiceError> {
        #[allow(clippy::cast_possible_wrap)]
        let pending_i64 = pending_change as i64;

        tx.execute(
            "INSERT INTO tree_reservations (id, purpose, pending_change_amount) VALUES ($1, $2, $3)",
            &[&reservation_id, &purpose.to_string(), &pending_i64],
        )
        .await
        .map_err(map_err)?;

        // Set reservation_id on leaves
        let leaf_ids: Vec<String> = leaves.iter().map(|l| l.id.to_string()).collect();
        Self::batch_set_reservation_id(tx, reservation_id, &leaf_ids).await?;

        Ok(())
    }
}

/// Maps any error to `TreeServiceError`.
fn map_err<E: std::fmt::Display>(e: E) -> TreeServiceError {
    TreeServiceError::Generic(e.to_string())
}

/// Creates a `PostgresTreeStore` instance for use with the SDK.
///
/// # Arguments
///
/// * `config` - Configuration for the `PostgreSQL` connection pool
pub async fn create_postgres_tree_store(
    config: PostgresStorageConfig,
) -> Result<Arc<dyn TreeStore>, StorageError> {
    Ok(Arc::new(PostgresTreeStore::new(config).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::tree_store_tests as shared_tests;
    use std::sync::Arc;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::postgres::Postgres;

    /// Helper struct that holds the container and store together.
    /// The container must be kept alive for the duration of the test.
    struct PostgresTreeStoreTestFixture {
        store: PostgresTreeStore,
        #[allow(dead_code)]
        container: ContainerAsync<Postgres>,
    }

    impl PostgresTreeStoreTestFixture {
        async fn new() -> Self {
            let container = Postgres::default()
                .start()
                .await
                .expect("Failed to start PostgreSQL container");

            let host_port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get host port");

            let connection_string = format!(
                "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
            );

            let store =
                PostgresTreeStore::new(PostgresStorageConfig::with_defaults(connection_string))
                    .await
                    .expect("Failed to create PostgresTreeStore");

            Self { store, container }
        }
    }

    fn create_test_tree_node(id: &str, value: u64) -> TreeNode {
        shared_tests::create_test_tree_node(id, value)
    }

    /// Helper function to reserve leaves in tests.
    /// Wraps `try_reserve_leaves` and expects success.
    async fn reserve_leaves(
        store: &PostgresTreeStore,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        shared_tests::reserve_leaves(store, target_amounts, exact_only, purpose).await
    }

    // ==================== Shared tests ====================

    #[tokio::test]
    async fn test_new() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_new(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_add_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_add_leaves_duplicate_ids() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves_duplicate_ids(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_reserve_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_cancel_reservation() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_cancel_reservation(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_cancel_reservation_nonexistent() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_cancel_reservation_nonexistent(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_reservation() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_finalize_reservation(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_reservation_nonexistent() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_finalize_reservation_nonexistent(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_multiple_reservations() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_multiple_reservations(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reservation_ids_are_unique() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_reservation_ids_are_unique(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_non_reservable_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_non_reservable_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_leaves_empty() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_reserve_leaves_empty(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_swap_reservation_included_in_balance() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_swap_reservation_included_in_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_payment_reservation_excluded_from_balance() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_payment_reservation_excluded_from_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_try_reserve_success() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_success(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_try_reserve_insufficient_funds() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_insufficient_funds(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_try_reserve_wait_for_pending() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_wait_for_pending(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_try_reserve_fail_immediately_when_insufficient() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_try_reserve_fail_immediately_when_insufficient(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_balance_change_notification() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_balance_change_notification(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_pending_cleared_on_cancel() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_pending_cleared_on_cancel(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_pending_cleared_on_finalize() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_pending_cleared_on_finalize(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_notification_after_swap_with_exact_amount() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_notification_after_swap_with_exact_amount(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_notification_on_pending_balance_change() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_notification_on_pending_balance_change(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves_with_reservations() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_with_reservations(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves_preserves_reservations_for_in_flight_swaps() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_preserves_reservations_for_in_flight_swaps(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_spent_leaves_not_restored_by_set_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_spent_leaves_not_restored_by_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_spent_ids_cleaned_up_when_no_longer_in_refresh() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_spent_ids_cleaned_up_when_no_longer_in_refresh(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_add_leaves_not_deleted_by_set_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves_not_deleted_by_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_old_leaves_deleted_by_set_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_old_leaves_deleted_by_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_change_leaves_from_swap_protected() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_change_leaves_from_swap_protected(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_with_new_leaves_protected() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_finalize_with_new_leaves_protected(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_add_leaves_clears_spent_status() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves_clears_spent_status(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves_skipped_during_active_swap() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_skipped_during_active_swap(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves_skipped_after_swap_completes_during_refresh() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_skipped_after_swap_completes_during_refresh(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_set_leaves_proceeds_after_swap_when_refresh_starts_later() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_proceeds_after_swap_when_refresh_starts_later(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_payment_reservation_does_not_block_set_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_payment_reservation_does_not_block_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_update_reservation_basic() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_update_reservation_basic(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_update_reservation_nonexistent() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_update_reservation_nonexistent(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_update_reservation_clears_pending() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_update_reservation_clears_pending(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_update_reservation_preserves_purpose() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_update_reservation_preserves_purpose(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_leaves_not_available() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_get_leaves_not_available(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_leaves_missing_operators_filters_spent() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_get_leaves_missing_operators_filters_spent(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_missing_operators_replaced_on_set_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_missing_operators_replaced_on_set_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_with_none_target_reserves_all() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_reserve_with_none_target_reserves_all(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_skips_non_available_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_reserve_skips_non_available_leaves(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_add_leaves_empty_slice() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_add_leaves_empty_slice(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_full_payment_cycle() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_full_payment_cycle(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_leaves_replaces_fully() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        shared_tests::test_set_leaves_replaces_fully(&fixture.store).await;
    }

    // ==================== Postgres-Specific Tests ====================

    // ==================== Stale Reservation Cleanup ====================

    #[tokio::test]
    async fn test_stale_reservation_cleanup() {
        // Test that stale reservations are cleaned up during set_leaves
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Create a reservation
        let reservation = reserve_leaves(
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
        assert_eq!(all_leaves.available.len(), 1);

        // Manually update the reservation's created_at to be older than the timeout
        // (RESERVATION_TIMEOUT_SECS = 300 seconds = 5 minutes)
        let client = fixture.store.pool.get().await.unwrap();
        client
            .execute(
                "UPDATE tree_reservations SET created_at = NOW() - INTERVAL '10 minutes' WHERE id = $1",
                &[&reservation.id],
            )
            .await
            .unwrap();

        // Call set_leaves which should trigger cleanup of stale reservations
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

        // Verify the stale reservation was cleaned up and leaves are available again
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
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1")
        );
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node2")
        );
    }

    #[tokio::test]
    async fn test_fresh_reservation_not_cleaned_up() {
        // Test that fresh (non-stale) reservations are NOT cleaned up during set_leaves
        let fixture = PostgresTreeStoreTestFixture::new().await;
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

    // ==================== Concurrency Stress Tests ====================

    #[tokio::test]
    #[allow(clippy::arithmetic_side_effects)]
    async fn test_concurrent_reserve_and_finalize() {
        // Test that concurrent reserve and finalize operations don't deadlock.
        // Uses a JoinSet to wait for any task to complete, avoiding sequential waiting issues.
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let store = Arc::new(fixture.store);

        // Add many leaves
        let mut leaves = Vec::new();
        for i in 0..50 {
            leaves.push(create_test_tree_node(&format!("node{i}"), 10));
        }
        store.add_leaves(&leaves).await.unwrap();

        // Spawn concurrent reserve operations using JoinSet
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
                    Ok(ReserveResult::Success(reservation)) => {
                        // Finalize the reservation
                        store_clone
                            .finalize_reservation(&reservation.id, None)
                            .await
                            .map(|()| (i, "reserved and finalized"))
                    }
                    Ok(ReserveResult::InsufficientFunds) => Ok((i, "insufficient funds")),
                    Ok(ReserveResult::WaitForPending { .. }) => Ok((i, "wait for pending")),
                    Err(e) => Err(e),
                }
            });
        }

        // Wait for all with global timeout
        let mut successes = 0;
        let timeout = tokio::time::timeout(std::time::Duration::from_secs(60), async {
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

        // At least some should succeed
        assert!(timeout > 0, "Expected at least one successful reservation");
    }

    #[tokio::test]
    async fn test_concurrent_reserve_cancel_cycle() {
        // Test rapid reserve/cancel cycles don't deadlock
        let fixture = PostgresTreeStoreTestFixture::new().await;
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
                        store_clone.cancel_reservation(&reservation.id).await?;
                    }
                    tracing::debug!("Task {i} cycle {cycle} complete");
                }
                Ok::<_, TreeServiceError>((i, "completed cycles"))
            });
        }

        // Wait for all with global timeout
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
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
        let fixture = PostgresTreeStoreTestFixture::new().await;
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
                        store_clone.cancel_reservation(&reservation.id).await?;
                        Ok((100 + i, "reserve success"))
                    }
                    Ok(_) => Ok((100 + i, "no leaves available")),
                    Err(e) => Err(e),
                }
            });
        }

        // Wait for all with global timeout
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
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
        let fixture = PostgresTreeStoreTestFixture::new().await;
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
        let timeout_result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
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
}
