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

/// Grace period in milliseconds for preserving recently added leaves during refresh.
/// Leaves added within this period before `refresh_started_at` are preserved
/// to handle race conditions where a refresh starts right after a leaf is added
/// but before operators have synced the new leaf data.
const LEAF_PRESERVATION_GRACE_PERIOD_MS: i64 = 5_000;

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
pub struct PostgresTreeStore {
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

        tracing::info!(
            "PostgresTreeStore::add_leaves: adding {} leaves",
            leaves.len()
        );
        for leaf in leaves {
            tracing::info!(
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
        tracing::info!(
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
                match Self::purpose_from_string(&purpose_str)? {
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

        // Check if any swap reservation is currently active
        let has_active_swap: bool = {
            let row = tx
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM tree_reservations WHERE purpose = 'Swap')",
                    &[],
                )
                .await
                .map_err(map_err)?;
            row.get(0)
        };

        // Check if a swap completed after this refresh started.
        // If so, the refresh data may be inconsistent (operators have new leaves,
        // but we're about to apply stale data from before the swap completed).
        let swap_completed_during_refresh: bool = {
            let row = tx
                .query_one(
                    "SELECT COALESCE(last_completed_at >= $1, FALSE) FROM tree_swap_status WHERE id = 1",
                    &[&refresh_timestamp],
                )
                .await
                .map_err(map_err)?;
            row.get(0)
        };

        if has_active_swap || swap_completed_during_refresh {
            trace!(
                "Skipping set_leaves: active_swap={}, swap_completed_during_refresh={}",
                has_active_swap, swap_completed_during_refresh
            );
            tx.rollback().await.ok();
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

        let reserved_ids: HashSet<String> = {
            let rows = tx
                .query(
                    "SELECT id FROM tree_leaves WHERE reservation_id IS NOT NULL",
                    &[],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get(0)).collect()
        };

        // Delete non-reserved leaves that were added BEFORE refresh started (minus grace period).
        // Leaves added within the grace period before refresh_started_at are preserved
        // to handle race conditions where a refresh starts right after a leaf is added
        // but before operators have synced the new leaf data.
        // The advisory lock acquired at the start of this transaction prevents deadlocks.
        let grace_period = chrono::Duration::milliseconds(LEAF_PRESERVATION_GRACE_PERIOD_MS);
        let cutoff_timestamp = refresh_timestamp
            .checked_sub_signed(grace_period)
            .unwrap_or(refresh_timestamp);
        tx.execute(
            "DELETE FROM tree_leaves WHERE reservation_id IS NULL AND added_at < $1",
            &[&cutoff_timestamp],
        )
        .await
        .map_err(map_err)?;

        // Clean up stale reservations from crashed clients.
        // This MUST be done AFTER the leaf delete above, because DELETE on tree_reservations
        // can affect tree_leaves through the ON DELETE SET NULL foreign key constraint,
        // which interferes with the timestamp-based leaf deletion.
        Self::cleanup_stale_reservations(&tx).await?;

        // Separate leaves into reserved (for update) and non-reserved (for upsert)
        let (reserved_leaves, non_reserved_leaves): (Vec<&TreeNode>, Vec<&TreeNode>) = leaves
            .iter()
            .filter(|l| !spent_ids.contains(&l.id.to_string()))
            .partition(|l| reserved_ids.contains(&l.id.to_string()));

        let (reserved_missing, non_reserved_missing): (Vec<&TreeNode>, Vec<&TreeNode>) =
            missing_operators_leaves
                .iter()
                .filter(|l| !spent_ids.contains(&l.id.to_string()))
                .partition(|l| reserved_ids.contains(&l.id.to_string()));

        // Batch update reserved leaves
        let reserved_leaves_owned: Vec<TreeNode> = reserved_leaves.into_iter().cloned().collect();
        Self::batch_update_reserved_leaves(&tx, &reserved_leaves_owned, false).await?;

        let reserved_missing_owned: Vec<TreeNode> = reserved_missing.into_iter().cloned().collect();
        Self::batch_update_reserved_leaves(&tx, &reserved_missing_owned, true).await?;

        // Batch upsert non-reserved leaves (with fresh added_at timestamp)
        let non_reserved_owned: Vec<TreeNode> = non_reserved_leaves.into_iter().cloned().collect();
        Self::batch_upsert_leaves(&tx, &non_reserved_owned, false, None).await?;

        let non_reserved_missing_owned: Vec<TreeNode> =
            non_reserved_missing.into_iter().cloned().collect();
        Self::batch_upsert_leaves(&tx, &non_reserved_missing_owned, true, None).await?;

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
            tx.rollback().await.ok();
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
            tx.rollback().await.ok();
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

        tracing::info!(
            "PostgresTreeStore::try_reserve_leaves: found {} available leaves",
            available_leaves.len()
        );
        for leaf in &available_leaves {
            tracing::info!(
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
                    tx.rollback().await.ok();
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

                // No suitable leaves found, rollback and return appropriate result
                tx.rollback().await.ok();

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
                tx.rollback().await.ok();

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
            tx.rollback().await.ok();
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
        let mut statuses: Vec<&str> = Vec::with_capacity(filtered.len());
        let mut missing_flags: Vec<bool> = Vec::with_capacity(filtered.len());
        let mut data_values: Vec<serde_json::Value> = Vec::with_capacity(filtered.len());

        for leaf in filtered {
            ids.push(leaf.id.to_string());
            statuses.push(Self::status_to_string(leaf.status));
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

    /// Batch updates reserved leaves (leaves that already exist and are reserved).
    async fn batch_update_reserved_leaves(
        tx: &tokio_postgres::Transaction<'_>,
        leaves: &[TreeNode],
        is_missing_from_operators: bool,
    ) -> Result<(), TreeServiceError> {
        if leaves.is_empty() {
            return Ok(());
        }

        let mut ids: Vec<String> = Vec::with_capacity(leaves.len());
        let mut statuses: Vec<&str> = Vec::with_capacity(leaves.len());
        let mut data_values: Vec<serde_json::Value> = Vec::with_capacity(leaves.len());

        for leaf in leaves {
            ids.push(leaf.id.to_string());
            statuses.push(Self::status_to_string(leaf.status));
            data_values.push(Self::serialize_node(leaf)?);
        }

        tx.execute(
            r"
            UPDATE tree_leaves
            SET status = u.status,
                is_missing_from_operators = $4,
                data = u.data
            FROM (SELECT * FROM UNNEST($1::text[], $2::text[], $3::jsonb[])) AS u(id, status, data)
            WHERE tree_leaves.id = u.id
            ",
            &[&ids, &statuses, &data_values, &is_missing_from_operators],
        )
        .await
        .map_err(map_err)?;

        Ok(())
    }

    /// Converts `TreeNodeStatus` to string for storage.
    fn status_to_string(status: TreeNodeStatus) -> &'static str {
        match status {
            TreeNodeStatus::Creating => "Creating",
            TreeNodeStatus::Available => "Available",
            TreeNodeStatus::FrozenByIssuer => "FrozenByIssuer",
            TreeNodeStatus::TransferLocked => "TransferLocked",
            TreeNodeStatus::SplitLocked => "SplitLocked",
            TreeNodeStatus::Splitted => "Splitted",
            TreeNodeStatus::Aggregated => "Aggregated",
            TreeNodeStatus::OnChain => "OnChain",
            TreeNodeStatus::Exited => "Exited",
            TreeNodeStatus::AggregateLock => "AggregateLock",
            TreeNodeStatus::Investigation => "Investigation",
            TreeNodeStatus::Lost => "Lost",
            TreeNodeStatus::Reimbursed => "Reimbursed",
        }
    }

    /// Converts `ReservationPurpose` to string.
    fn purpose_to_string(purpose: ReservationPurpose) -> &'static str {
        match purpose {
            ReservationPurpose::Payment => "Payment",
            ReservationPurpose::Swap => "Swap",
        }
    }

    /// Parses `ReservationPurpose` from string.
    fn purpose_from_string(s: &str) -> Result<ReservationPurpose, TreeServiceError> {
        match s {
            "Payment" => Ok(ReservationPurpose::Payment),
            "Swap" => Ok(ReservationPurpose::Swap),
            _ => Err(TreeServiceError::Generic(format!(
                "Unknown reservation purpose: {s}"
            ))),
        }
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
            &[&reservation_id, &Self::purpose_to_string(purpose), &pending_i64],
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
    use bitcoin::{Transaction, absolute::LockTime, secp256k1::PublicKey, transaction::Version};
    use frost_secp256k1_tr::Identifier;
    use spark_wallet::TreeNodeId;
    use std::str::FromStr;
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
        TreeNode {
            id: TreeNodeId::from_str(id).unwrap(),
            tree_id: "test_tree".to_string(),
            value,
            parent_node_id: None,
            node_tx: Transaction {
                version: Version::non_standard(3),
                lock_time: LockTime::ZERO,
                input: vec![],
                output: vec![],
            },
            refund_tx: None,
            direct_tx: None,
            direct_refund_tx: None,
            direct_from_cpfp_refund_tx: None,
            vout: 0,
            verifying_public_key: PublicKey::from_str(
                "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
            )
            .unwrap(),
            owner_identity_public_key: PublicKey::from_str(
                "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
            )
            .unwrap(),
            signing_keyshare: spark_wallet::SigningKeyshare {
                public_key: PublicKey::from_str(
                    "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
                )
                .unwrap(),
                owner_identifiers: vec![Identifier::try_from(1u16).unwrap()],
                threshold: 2,
            },
            status: TreeNodeStatus::Available,
        }
    }

    /// Helper function to reserve leaves in tests.
    /// Wraps `try_reserve_leaves` and expects success.
    async fn reserve_leaves(
        store: &PostgresTreeStore,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        match store
            .try_reserve_leaves(target_amounts, exact_only, purpose)
            .await?
        {
            ReserveResult::Success(reservation) => Ok(reservation),
            ReserveResult::InsufficientFunds => Err(TreeServiceError::InsufficientFunds),
            ReserveResult::WaitForPending { .. } => Err(TreeServiceError::Generic(
                "Unexpected WaitForPending".into(),
            )),
        }
    }

    // ==================== Basic Operations ====================

    #[tokio::test]
    async fn test_new() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        assert!(
            fixture
                .store
                .get_leaves()
                .await
                .unwrap()
                .available
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_add_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];

        fixture.store.add_leaves(&leaves).await.unwrap();

        let stored_leaves = fixture.store.get_leaves().await.unwrap().available;
        assert_eq!(stored_leaves.len(), 2);
        assert!(
            stored_leaves
                .iter()
                .any(|l| l.id.to_string() == "node1" && l.value == 100)
        );
        assert!(
            stored_leaves
                .iter()
                .any(|l| l.id.to_string() == "node2" && l.value == 200)
        );
    }

    #[tokio::test]
    async fn test_add_leaves_duplicate_ids() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaf1 = create_test_tree_node("node1", 100);
        let leaf2 = create_test_tree_node("node1", 200); // Same ID, different value

        fixture.store.add_leaves(&[leaf1]).await.unwrap();
        fixture.store.add_leaves(&[leaf2]).await.unwrap();

        let stored_leaves = fixture.store.get_leaves().await.unwrap().available;
        assert_eq!(stored_leaves.len(), 1);
        // With ON CONFLICT DO UPDATE, the second value (200) replaces the first
        // This matches InMemoryTreeStore behavior (HashMap::insert replaces)
        assert_eq!(stored_leaves[0].value, 200);
    }

    #[tokio::test]
    async fn test_set_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let initial_leaves = vec![create_test_tree_node("node1", 100)];
        fixture.store.add_leaves(&initial_leaves).await.unwrap();

        // Use a refresh_start far enough in the future to exceed the grace period.
        // This simulates an "old" refresh that should delete leaves added "now".
        let refresh_start = SystemTime::now()
            + std::time::Duration::from_millis((LEAF_PRESERVATION_GRACE_PERIOD_MS + 1000) as u64);

        let new_leaves = vec![
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        // Use a refresh_start that's AFTER the initial leaves were added
        // so they're considered "old" and can be replaced
        fixture
            .store
            .set_leaves(&new_leaves, &[], refresh_start)
            .await
            .unwrap();

        let stored_leaves = fixture.store.get_leaves().await.unwrap().available;
        assert_eq!(stored_leaves.len(), 2);
        assert!(stored_leaves.iter().any(|l| l.id.to_string() == "node2"));
        assert!(stored_leaves.iter().any(|l| l.id.to_string() == "node3"));
        assert!(!stored_leaves.iter().any(|l| l.id.to_string() == "node1"));
    }

    // ==================== Reservation Operations ====================

    #[tokio::test]
    async fn test_reserve_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
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

        // Check that reservation was created by verifying leaves are reserved
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.reserved_for_payment.len(), 1);
        assert_eq!(all_leaves.reserved_for_payment[0].id, leaves[0].id);
        // Check that leaf was removed from main pool
        assert_eq!(all_leaves.available.len(), 1);
        assert_eq!(all_leaves.available[0].id, leaves[1].id);

        // Verify reservation ID is valid UUID
        assert!(!reservation.id.is_empty());
    }

    #[tokio::test]
    async fn test_cancel_reservation() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
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

        // Cancel the reservation
        fixture
            .store
            .cancel_reservation(&reservation.id)
            .await
            .unwrap();

        // Check that leaf was returned to main pool
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(all_leaves.reserved_for_payment.is_empty());
        assert_eq!(all_leaves.available.len(), 2);
        assert!(all_leaves.available.iter().any(|l| l.id == leaves[0].id));
        assert!(all_leaves.available.iter().any(|l| l.id == leaves[1].id));
    }

    #[tokio::test]
    async fn test_cancel_reservation_nonexistent() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        fixture.store.cancel_reservation(&fake_id).await.unwrap();

        let main_leaves = fixture.store.get_leaves().await.unwrap().available;
        assert!(main_leaves.is_empty());
    }

    #[tokio::test]
    async fn test_finalize_reservation() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
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

        // Finalize the reservation
        fixture
            .store
            .finalize_reservation(&reservation.id, None)
            .await
            .unwrap();

        // Check that reservation was removed and leaf was NOT returned to main pool
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(all_leaves.reserved_for_payment.is_empty());
        assert_eq!(all_leaves.available.len(), 1);
        assert_eq!(all_leaves.available[0].id, leaves[1].id);
    }

    #[tokio::test]
    async fn test_finalize_reservation_nonexistent() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues - returns Ok like in-memory
        fixture
            .store
            .finalize_reservation(&fake_id, None)
            .await
            .unwrap();

        let main_leaves = fixture.store.get_leaves().await.unwrap().available;
        assert!(main_leaves.is_empty());
    }

    #[tokio::test]
    async fn test_multiple_reservations() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Create multiple reservations
        let reservation1 = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
        let reservation2 = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(200, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Check both reservations exist
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.reserved_for_payment.len(), 2);
        assert_eq!(all_leaves.available.len(), 1);
        assert_eq!(all_leaves.available[0].id, leaves[2].id);

        // Cancel one reservation
        fixture
            .store
            .cancel_reservation(&reservation1.id)
            .await
            .unwrap();
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.reserved_for_payment.len(), 1);
        assert_eq!(all_leaves.available.len(), 2);

        // Finalize the other
        fixture
            .store
            .finalize_reservation(&reservation2.id, None)
            .await
            .unwrap();
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(all_leaves.reserved_for_payment.is_empty());
        assert_eq!(all_leaves.available.len(), 2);
    }

    #[tokio::test]
    async fn test_reservation_ids_are_unique() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaf = create_test_tree_node("node1", 100);
        fixture
            .store
            .add_leaves(std::slice::from_ref(&leaf))
            .await
            .unwrap();

        let r1 = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
        fixture.store.cancel_reservation(&r1.id).await.unwrap();
        let r2 = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        assert_ne!(r1.id, r2.id);
    }

    #[tokio::test]
    async fn test_non_reservable_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaf = create_test_tree_node("node1", 100);
        fixture
            .store
            .add_leaves(std::slice::from_ref(&leaf))
            .await
            .unwrap();

        reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        let result = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap_err();
        assert!(matches!(result, TreeServiceError::InsufficientFunds));
    }

    #[tokio::test]
    async fn test_reserve_leaves_empty() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let err = reserve_leaves(&fixture.store, None, false, ReservationPurpose::Payment)
            .await
            .unwrap_err();

        // With no target amounts and no leaves, we get NonReservableLeaves
        assert!(matches!(err, TreeServiceError::NonReservableLeaves));
    }

    // ==================== Balance and Reserved Types ====================

    #[tokio::test]
    async fn test_swap_reservation_included_in_balance() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve some leaves for swap
        let _reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            true,
            ReservationPurpose::Swap,
        )
        .await
        .unwrap();

        // Check that swap-reserved leaves are included in balance
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.swap_reserved_balance(), 300);
        assert_eq!(all_leaves.available_balance(), 300); // node1 + node2 remaining
        // balance() should include swap-reserved leaves
        assert_eq!(all_leaves.balance(), 300 + 300); // available + swap-reserved
    }

    #[tokio::test]
    async fn test_payment_reservation_excluded_from_balance() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve some leaves for payment
        let _reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Check that payment-reserved leaves are excluded from balance
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.payment_reserved_balance(), 300);
        assert_eq!(all_leaves.available_balance(), 300); // node1 + node2 remaining
        // balance() should NOT include payment-reserved leaves
        assert_eq!(all_leaves.balance(), 300); // only available
    }

    // ==================== Try Reserve with Result Handling ====================

    #[tokio::test]
    async fn test_try_reserve_success() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        let result = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        assert!(matches!(result, ReserveResult::Success(_)));
        if let ReserveResult::Success(reservation) = result {
            assert_eq!(reservation.sum(), 100);
        }
    }

    #[tokio::test]
    async fn test_try_reserve_insufficient_funds() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![create_test_tree_node("node1", 100)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        let result = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(500, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        assert!(matches!(result, ReserveResult::InsufficientFunds));
    }

    #[tokio::test]
    async fn test_try_reserve_wait_for_pending() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        // Add a single 1000 sat leaf
        let leaves = vec![create_test_tree_node("node1", 1000)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve with target 100 - store will reserve 1000 and auto-track pending=900
        let r1 = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();
        assert!(matches!(r1, ReserveResult::Success(_)));

        // Try to reserve 300 more - should get WaitForPending since pending=900 > 300
        let r2 = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(300, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        match r2 {
            ReserveResult::WaitForPending {
                needed,
                available,
                pending,
            } => {
                assert_eq!(needed, 300);
                assert_eq!(available, 0);
                assert_eq!(pending, 900);
            }
            _ => panic!("Expected WaitForPending, got {r2:?}"),
        }
    }

    #[tokio::test]
    async fn test_try_reserve_fail_immediately_when_insufficient() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        // Add 100 sat leaf
        let leaves = vec![create_test_tree_node("node1", 100)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve it for 50 sats - pending will be 50
        let r1 = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(50, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();
        assert!(matches!(r1, ReserveResult::Success(_)));

        // Request 500 - more than available + pending (0 + 50 < 500)
        let result = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(500, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();
        assert!(matches!(result, ReserveResult::InsufficientFunds));
    }

    // ==================== Balance Change Notifications ====================

    #[tokio::test]
    async fn test_balance_change_notification() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let mut rx = fixture.store.subscribe_balance_changes();

        // Mark initial value as seen so changed() waits for actual updates
        rx.borrow_and_update();

        // Add leaves
        let leaves = vec![create_test_tree_node("node1", 100)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Wait for notification with timeout (longer timeout for CI stability under load)
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            rx.changed().await.ok();
        })
        .await;

        // Just verify we received a notification (the value is () and doesn't matter)
        assert!(result.is_ok(), "Timed out waiting for balance notification");
    }

    #[tokio::test]
    async fn test_pending_cleared_on_cancel() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![create_test_tree_node("node1", 1000)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve with target 100 - auto-tracks pending=900
        let r1 = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        let reservation_id = match r1 {
            ReserveResult::Success(r) => r.id,
            _ => panic!("Expected Success"),
        };

        // Cancel the reservation - pending should be cleared
        fixture
            .store
            .cancel_reservation(&reservation_id)
            .await
            .unwrap();

        // Try to reserve 300 - should succeed since 1000 sat leaf is back
        let r2 = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(300, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Now 1000 sat leaf is back, so we should succeed
        assert!(matches!(r2, ReserveResult::Success(_)));
    }

    #[tokio::test]
    async fn test_pending_cleared_on_finalize() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![create_test_tree_node("node1", 1000)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve with target 100 - auto-tracks pending=900
        let r1 = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        let reservation_id = match r1 {
            ReserveResult::Success(r) => r.id,
            _ => panic!("Expected Success"),
        };

        // Finalize with new leaves (the change from swap)
        let change_leaf = create_test_tree_node("node2", 900);
        fixture
            .store
            .finalize_reservation(&reservation_id, Some(&[change_leaf]))
            .await
            .unwrap();

        // Try to reserve 300 - should succeed since change is now available
        let r2 = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(300, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        assert!(matches!(r2, ReserveResult::Success(_)));
    }

    // ==================== Swap Updates ====================

    #[tokio::test]
    async fn test_notification_after_swap_with_exact_amount() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let mut rx = fixture.store.subscribe_balance_changes();

        // Add a single 1000 sat leaf
        let leaves = vec![create_test_tree_node("node1", 1000)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Consume the initial notification
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed()).await;

        // Reserve it with target 100 - will reserve all 1000, pending=900
        let r1 = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        let reservation_id = match r1 {
            ReserveResult::Success(r) => r.id,
            _ => panic!("Expected Success"),
        };

        // Consume the reservation notification
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed()).await;

        // Simulate a swap that returns exactly the target amount (100 sats)
        let swap_result_leaf = create_test_tree_node("node2", 100);
        fixture
            .store
            .update_reservation(&reservation_id, &[swap_result_leaf], &[])
            .await
            .unwrap();

        // Verify that we still get a notification
        let notification_result =
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed()).await;

        assert!(
            notification_result.is_ok(),
            "Expected notification after swap update with exact amount"
        );
    }

    #[tokio::test]
    async fn test_notification_on_pending_balance_change() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let mut rx = fixture.store.subscribe_balance_changes();

        // Add a single 1000 sat leaf
        let leaves = vec![create_test_tree_node("node1", 1000)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Consume initial notification
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed()).await;

        // Reserve with target 100 - pending=900
        let r1 = fixture
            .store
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Consume reservation notification
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed()).await;

        let reservation_id = match r1 {
            ReserveResult::Success(r) => r.id,
            _ => panic!("Expected Success"),
        };

        // Cancel the reservation - this clears pending from 900 to 0
        fixture
            .store
            .cancel_reservation(&reservation_id)
            .await
            .unwrap();

        // Should get notification because pending balance changed
        let notification_result =
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed()).await;

        assert!(
            notification_result.is_ok(),
            "Expected notification when pending balance changes"
        );
    }

    // ==================== Set Leaves with Reservations ====================

    #[tokio::test]
    async fn test_set_leaves_with_reservations() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve all leaves
        let _reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(600, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Small delay to ensure refresh_start is after leaves were added
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let refresh_start = SystemTime::now();

        // Update leaves with new data (including updated versions of reserved leaves)
        let non_existing_operator_leaf = create_test_tree_node("node7", 1000);
        let mut updated_leaf1 = create_test_tree_node("node1", 150);
        updated_leaf1.status = TreeNodeStatus::TransferLocked;
        let new_leaves = vec![
            updated_leaf1,
            create_test_tree_node("node2", 250),
            create_test_tree_node("node4", 400),
        ];
        fixture
            .store
            .set_leaves(&new_leaves, &[non_existing_operator_leaf], refresh_start)
            .await
            .unwrap();

        // Check main pool
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        // Reserved leaves should be preserved and updated where data exists
        assert_eq!(all_leaves.payment_reserved_balance(), 700); // 150 + 250 + 300 (node3 keeps original)
        assert_eq!(all_leaves.available_balance(), 400);
        assert_eq!(all_leaves.missing_operators_balance(), 1000);
        assert_eq!(all_leaves.balance(), 400 + 1000);
        assert_eq!(all_leaves.available.len(), 1);
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node4")
        );
    }

    #[tokio::test]
    async fn test_set_leaves_preserves_reservations_for_in_flight_swaps() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve leaves (simulating start of a swap)
        let _reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Small delay to ensure refresh_start is after leaves were added
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let refresh_start = SystemTime::now();

        // Set new leaves that don't include the reserved ones
        let new_leaves = vec![create_test_tree_node("node3", 300)];
        fixture
            .store
            .set_leaves(&new_leaves, &[], refresh_start)
            .await
            .unwrap();

        // Reservation should be PRESERVED (not removed)
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        // The reserved leaves keep their original values since they're not updated
        assert_eq!(all_leaves.reserved_for_payment.len(), 2);
        assert!(
            all_leaves
                .reserved_for_payment
                .iter()
                .any(|l| l.id.to_string() == "node1" && l.value == 100)
        );
        assert!(
            all_leaves
                .reserved_for_payment
                .iter()
                .any(|l| l.id.to_string() == "node2" && l.value == 200)
        );
    }

    // ==================== Spent Leaves Cleanup ====================

    #[tokio::test]
    async fn test_spent_leaves_not_restored_by_set_leaves() {
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve node1 for payment
        let reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Finalize the reservation (node1 is now spent)
        fixture
            .store
            .finalize_reservation(&reservation.id, None)
            .await
            .unwrap();

        // Verify node1 is not in the pool
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 1);
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node2")
        );
        assert!(
            !all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1")
        );

        // Simulate a refresh that started BEFORE the finalize completed.
        // Use a timestamp in the past to simulate this race condition.
        // Since spent_at >= refresh_start, the spent marker should be kept.
        let refresh_start = SystemTime::now() - std::time::Duration::from_secs(60);
        let stale_leaves = vec![
            create_test_tree_node("node1", 100), // This was spent!
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300), // New leaf
        ];
        fixture
            .store
            .set_leaves(&stale_leaves, &[], refresh_start)
            .await
            .unwrap();

        // Verify node1 was NOT restored (it's in spent markers, spent_at >= refresh_start)
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 2); // node2 and node3 only
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node2")
        );
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node3")
        );
        assert!(
            !all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1"),
            "Spent leaf node1 should not be restored when refresh started before spend"
        );
    }

    #[tokio::test]
    async fn test_spent_ids_cleaned_up_when_no_longer_in_refresh() {
        // Tests that spent markers are cleaned up based on timestamp:
        // - Kept when spent_at >= refresh_start (recent spend)
        // - Ignored (not used for filtering) when spent_at < refresh_start
        // - Actually deleted after SPENT_MARKER_CLEANUP_THRESHOLD_MS
        let fixture = PostgresTreeStoreTestFixture::new().await;
        let leaves = vec![create_test_tree_node("node1", 100)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve and finalize node1
        let reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
        fixture
            .store
            .finalize_reservation(&reservation.id, None)
            .await
            .unwrap();

        // First refresh with refresh_start BEFORE spent_at (simulating race condition).
        // The spent marker should filter out node1 because spent_at >= refresh_start.
        let refresh_start = SystemTime::now() - std::time::Duration::from_secs(60);
        let stale_leaves = vec![create_test_tree_node("node1", 100)];
        fixture
            .store
            .set_leaves(&stale_leaves, &[], refresh_start)
            .await
            .unwrap();
        assert!(
            fixture
                .store
                .get_leaves()
                .await
                .unwrap()
                .available
                .is_empty(),
            "node1 should be filtered by spent marker (recent spend)"
        );

        // Spent marker should still exist (not deleted yet, within threshold)
        let client = fixture.store.pool.get().await.unwrap();
        let spent_count: i64 = client
            .query_one("SELECT COUNT(*) FROM tree_spent_leaves", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(spent_count, 1, "spent marker should still exist");

        // Second refresh with refresh_start AFTER spent_at (operators had time to process).
        // The spent marker is ignored (not used for filtering) because spent_at < refresh_start.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let refresh_start2 = SystemTime::now();
        let fresh_leaves = vec![create_test_tree_node("node2", 200)];
        fixture
            .store
            .set_leaves(&fresh_leaves, &[], refresh_start2)
            .await
            .unwrap();

        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 1);
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node2")
        );

        // Spent marker still exists (within threshold, not deleted yet)
        // but it's ignored for filtering because spent_at < refresh_start2
        let spent_count: i64 = client
            .query_one("SELECT COUNT(*) FROM tree_spent_leaves", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            spent_count, 1,
            "spent marker still exists but is ignored for filtering"
        );

        // If node1 appears again (e.g., received back via transfer), it should be accepted
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let refresh_start3 = SystemTime::now();
        let new_node1_leaves = vec![
            create_test_tree_node("node1", 150),
            create_test_tree_node("node2", 200),
        ];
        fixture
            .store
            .set_leaves(&new_node1_leaves, &[], refresh_start3)
            .await
            .unwrap();

        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(
            all_leaves.available.len(),
            2,
            "node1 should be accepted after spent marker cleanup"
        );
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1" && l.value == 150)
        );
    }

    // ==================== Race Condition Fix Tests ====================

    #[tokio::test]
    async fn test_add_leaves_not_deleted_by_set_leaves() {
        // Test that leaves added AFTER refresh starts are NOT deleted by set_leaves.
        // This is the key race condition fix.
        let fixture = PostgresTreeStoreTestFixture::new().await;

        // Add initial leaves
        let initial_leaves = vec![create_test_tree_node("node1", 100)];
        fixture.store.add_leaves(&initial_leaves).await.unwrap();

        // Simulate: refresh starts at T1
        let refresh_start = SystemTime::now();

        // Small delay to ensure the new leaf is added AFTER refresh_start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Simulate: while refresh is in progress, a new leaf arrives (e.g., from a payment)
        let new_leaf = create_test_tree_node("node2", 200);
        fixture.store.add_leaves(&[new_leaf]).await.unwrap();

        // Simulate: refresh completes with stale data (doesn't include node2)
        let stale_refresh_data = vec![create_test_tree_node("node1", 100)];
        fixture
            .store
            .set_leaves(&stale_refresh_data, &[], refresh_start)
            .await
            .unwrap();

        // Verify: node2 is PRESERVED (not deleted) because it was added after refresh started
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 2);
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
                .any(|l| l.id.to_string() == "node2"),
            "Leaf added after refresh started should be preserved"
        );
    }

    #[tokio::test]
    async fn test_old_leaves_deleted_by_set_leaves() {
        // Test that leaves added BEFORE refresh starts ARE deleted if not in refresh data.
        let fixture = PostgresTreeStoreTestFixture::new().await;

        // Add initial leaves
        let initial_leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture.store.add_leaves(&initial_leaves).await.unwrap();

        // Use a refresh_start far enough in the future to exceed the grace period.
        // This simulates an "old" refresh that should delete leaves added "now".
        let refresh_start = SystemTime::now()
            + std::time::Duration::from_millis((LEAF_PRESERVATION_GRACE_PERIOD_MS + 1000) as u64);

        // Simulate: refresh completes with data that doesn't include node2
        let refresh_data = vec![create_test_tree_node("node1", 100)];
        fixture
            .store
            .set_leaves(&refresh_data, &[], refresh_start)
            .await
            .unwrap();

        // Verify: node2 is DELETED because it was added before refresh started and not in refresh
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 1);
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1")
        );
        assert!(
            !all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node2"),
            "Leaf added before refresh started should be deleted if not in refresh data"
        );
    }

    #[tokio::test]
    async fn test_change_leaves_from_swap_protected() {
        // Test that change leaves from update_reservation are protected from concurrent refresh.
        let fixture = PostgresTreeStoreTestFixture::new().await;

        // Add initial leaf
        let initial_leaves = vec![create_test_tree_node("node1", 1000)];
        fixture.store.add_leaves(&initial_leaves).await.unwrap();

        // Reserve the leaf
        let reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(1000, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Simulate: refresh starts
        let refresh_start = SystemTime::now();

        // Small delay
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Simulate: swap completes and adds change leaves via update_reservation
        let reserved_leaf = create_test_tree_node("swap_output", 500);
        let change_leaf = create_test_tree_node("change", 500);
        fixture
            .store
            .update_reservation(&reservation.id, &[reserved_leaf], &[change_leaf])
            .await
            .unwrap();

        // Simulate: refresh completes with stale data (doesn't include change leaf)
        let stale_refresh_data = vec![create_test_tree_node("node1", 1000)];
        fixture
            .store
            .set_leaves(&stale_refresh_data, &[], refresh_start)
            .await
            .unwrap();

        // Verify: change leaf is PRESERVED
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "change"),
            "Change leaf from swap should be preserved"
        );
    }

    #[tokio::test]
    async fn test_finalize_with_new_leaves_protected() {
        // Test that new leaves from finalize_reservation are protected from concurrent refresh.
        let fixture = PostgresTreeStoreTestFixture::new().await;

        // Add initial leaf
        let initial_leaves = vec![create_test_tree_node("node1", 1000)];
        fixture.store.add_leaves(&initial_leaves).await.unwrap();

        // Reserve the leaf
        let reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(1000, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Simulate: refresh starts
        let refresh_start = SystemTime::now();

        // Small delay
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Simulate: payment completes and adds change via finalize_reservation
        let change_leaf = create_test_tree_node("change", 900);
        fixture
            .store
            .finalize_reservation(&reservation.id, Some(&[change_leaf]))
            .await
            .unwrap();

        // Simulate: refresh completes with stale data
        let stale_refresh_data = vec![create_test_tree_node("node1", 1000)];
        fixture
            .store
            .set_leaves(&stale_refresh_data, &[], refresh_start)
            .await
            .unwrap();

        // Verify: change leaf is PRESERVED, node1 is NOT restored (it's spent)
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "change"),
            "Change leaf from finalize should be preserved"
        );
        assert!(
            !all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1"),
            "Spent leaf should not be restored"
        );
    }

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

    // ==================== Swap/Refresh Race Condition Fix Tests ====================

    #[tokio::test]
    async fn test_set_leaves_skipped_during_active_swap() {
        // Test that set_leaves is skipped when there's an active swap reservation.
        // This prevents stale refresh data from overwriting swap results.
        let fixture = PostgresTreeStoreTestFixture::new().await;

        // Add initial leaves
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve leaves for a swap (not payment)
        let _reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Swap, // This is a swap, not payment
        )
        .await
        .unwrap();

        // Simulate refresh starting while swap is in progress
        let refresh_start = SystemTime::now();

        // Small delay
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Try to set new leaves (should be skipped due to active swap)
        let new_leaves = vec![create_test_tree_node("node3", 300)];
        fixture
            .store
            .set_leaves(&new_leaves, &[], refresh_start)
            .await
            .unwrap();

        // Verify: since there's an active swap, set_leaves should have been skipped
        // The available pool should still be empty (leaves are reserved for swap)
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(
            all_leaves.available.is_empty(),
            "set_leaves should be skipped during active swap"
        );
        assert_eq!(all_leaves.reserved_for_swap.len(), 2);
    }

    #[tokio::test]
    async fn test_set_leaves_skipped_after_swap_completes_during_refresh() {
        // Test the main race condition fix:
        // 1. Refresh starts (t=0)
        // 2. Swap completes during refresh (t=1)
        // 3. set_leaves called with stale data (t=2)
        // Expected: set_leaves should be skipped because swap completed after refresh started
        let fixture = PostgresTreeStoreTestFixture::new().await;

        // Add initial leaves
        let leaves = vec![create_test_tree_node("node1", 1000)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve leaves for a swap
        let reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(1000, None)),
            false,
            ReservationPurpose::Swap,
        )
        .await
        .unwrap();

        // Simulate refresh starting at T0
        let refresh_start = SystemTime::now();

        // Small delay to ensure swap completes AFTER refresh started
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Swap completes at T1, adding new leaves
        let new_leaves_from_swap = vec![create_test_tree_node("swap_result", 500)];
        fixture
            .store
            .finalize_reservation(&reservation.id, Some(&new_leaves_from_swap))
            .await
            .unwrap();

        // Verify swap result leaves are in the pool
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 1);
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "swap_result")
        );

        // Now at T2, set_leaves is called with stale data from the refresh
        // This data was fetched at T0, before the swap completed
        let stale_refresh_data = vec![create_test_tree_node("node1", 1000)]; // Old leaf
        fixture
            .store
            .set_leaves(&stale_refresh_data, &[], refresh_start)
            .await
            .unwrap();

        // Verify: set_leaves should have been SKIPPED because swap completed during refresh
        // The swap result leaf should still be present
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "swap_result"),
            "Swap result leaf should be preserved after skipped set_leaves"
        );
        // The stale node1 should NOT have been restored
        assert!(
            !all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1"),
            "Stale leaf should not be restored when set_leaves is skipped"
        );
    }

    #[tokio::test]
    async fn test_set_leaves_proceeds_after_swap_when_refresh_starts_later() {
        // Test that set_leaves proceeds normally when refresh starts AFTER swap completed.
        // This is the normal case - swap finishes, then a new refresh starts.
        let fixture = PostgresTreeStoreTestFixture::new().await;

        // Add initial leaves
        let leaves = vec![create_test_tree_node("node1", 1000)];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve leaves for a swap
        let reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(1000, None)),
            false,
            ReservationPurpose::Swap,
        )
        .await
        .unwrap();

        // Swap completes first
        let new_leaves_from_swap = vec![create_test_tree_node("swap_result", 500)];
        fixture
            .store
            .finalize_reservation(&reservation.id, Some(&new_leaves_from_swap))
            .await
            .unwrap();

        // Small delay to ensure refresh starts AFTER swap completed
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Now refresh starts (AFTER swap completed)
        let refresh_start = SystemTime::now();

        // Small delay for grace period
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // set_leaves called with fresh data that includes the swap result
        let fresh_refresh_data = vec![
            create_test_tree_node("swap_result", 500),
            create_test_tree_node("new_deposit", 200),
        ];
        fixture
            .store
            .set_leaves(&fresh_refresh_data, &[], refresh_start)
            .await
            .unwrap();

        // Verify: set_leaves should have proceeded normally
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "swap_result"),
            "swap_result should be present"
        );
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "new_deposit"),
            "new_deposit should be added"
        );
    }

    #[tokio::test]
    async fn test_payment_reservation_does_not_block_set_leaves() {
        // Test that payment reservations (not swap) do NOT block set_leaves.
        // Only swap reservations should block because they modify leaf ownership.
        let fixture = PostgresTreeStoreTestFixture::new().await;

        // Add initial leaves
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        fixture.store.add_leaves(&leaves).await.unwrap();

        // Reserve leaves for PAYMENT (not swap)
        let _reservation = reserve_leaves(
            &fixture.store,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment, // This is payment, should not block
        )
        .await
        .unwrap();

        // Small delay to ensure refresh starts after leaves were added
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let refresh_start = SystemTime::now();

        // set_leaves should proceed (payment reservation doesn't block)
        let new_leaves = vec![
            create_test_tree_node("node1", 150), // Updated value
            create_test_tree_node("node3", 300), // New leaf
        ];
        fixture
            .store
            .set_leaves(&new_leaves, &[], refresh_start)
            .await
            .unwrap();

        // Verify: set_leaves should have proceeded
        // node3 should be in the pool (set_leaves was not skipped)
        let all_leaves = fixture.store.get_leaves().await.unwrap();
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node3"),
            "New leaf should be added when payment reservation is active"
        );
    }
}
