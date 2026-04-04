//! `PostgreSQL`-backed implementation of the `TokenOutputStore` trait.
//!
//! This module provides a persistent token output store backed by `PostgreSQL`,
//! suitable for server-side or multi-instance deployments where
//! in-memory storage is insufficient.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use spark_postgres::deadpool_postgres;
use spark_postgres::tokio_postgres;

use deadpool_postgres::Pool;
use macros::async_trait;
use platform_utils::time::SystemTime;
use spark_wallet::{
    GetTokenOutputsFilter, ReservationTarget, SelectionStrategy, TokenMetadata, TokenOutput,
    TokenOutputServiceError, TokenOutputStore, TokenOutputWithPrevOut, TokenOutputs,
    TokenOutputsPerStatus, TokenOutputsReservation, TokenOutputsReservationId,
    TokenReservationPurpose,
};
use tracing::{trace, warn};
use uuid::Uuid;

use crate::persist::StorageError;

use super::base::run_migrations;
#[cfg(test)]
use super::base::{PostgresStorageConfig, create_pool};

/// Name of the schema migrations table for `PostgresTokenStore`.
const TOKEN_MIGRATIONS_TABLE: &str = "token_schema_migrations";

/// Advisory lock key for serializing token store write operations.
/// This prevents deadlocks by ensuring only one write transaction runs at a time.
/// The lock is automatically released when the transaction commits or rolls back.
const TOKEN_STORE_WRITE_LOCK_KEY: i64 = 0x746F_6B65_6E53_5452; // "toknSTR" as hex

/// Spent markers are kept in the database for this duration to support multiple
/// SDK instances sharing the same postgres database. During `set_tokens_outputs`, spent
/// markers older than `refresh_timestamp` are ignored (treated as deleted).
/// Actual deletion only happens for markers older than this threshold.
const SPENT_MARKER_CLEANUP_THRESHOLD_MS: i64 = 5 * 60 * 1000; // 5 minutes

/// `PostgreSQL`-backed token output store implementation.
pub(crate) struct PostgresTokenStore {
    pool: Pool,
}

#[async_trait]
impl TokenOutputStore for PostgresTokenStore {
    #[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
    async fn set_tokens_outputs(
        &self,
        token_outputs: &[TokenOutputs],
        refresh_started_at: SystemTime,
    ) -> Result<(), TokenOutputServiceError> {
        // Convert SystemTime to chrono for PostgreSQL
        let refresh_timestamp: chrono::DateTime<chrono::Utc> = refresh_started_at.into();

        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        Self::acquire_write_lock(&tx).await?;

        // Skip if swap is active or completed during this refresh
        let (has_active_swap, swap_completed_during_refresh): (bool, bool) = {
            let row = tx
                .query_one(
                    r"
                    SELECT
                        EXISTS(SELECT 1 FROM token_reservations WHERE purpose = 'Swap'),
                        COALESCE((SELECT last_completed_at >= $1 FROM token_swap_status WHERE id = 1), FALSE)
                    ",
                    &[&refresh_timestamp],
                )
                .await
                .map_err(map_err)?;
            (row.get(0), row.get(1))
        };

        if has_active_swap || swap_completed_during_refresh {
            trace!(
                "Skipping set_tokens_outputs: active_swap={}, swap_completed_during_refresh={}",
                has_active_swap, swap_completed_during_refresh
            );
            return Ok(());
        }

        // Clean up old spent markers
        Self::cleanup_spent_markers(&tx, refresh_timestamp).await?;

        // Get recent spent output IDs (spent_at >= refresh_timestamp).
        // Older spent markers are ignored - if the refresh started after the spend,
        // operators had time to process it.
        let spent_ids: HashSet<String> = {
            let rows = tx
                .query(
                    "SELECT output_id FROM token_spent_outputs WHERE spent_at >= $1",
                    &[&refresh_timestamp],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get(0)).collect()
        };

        // Delete non-reserved outputs added BEFORE the refresh started.
        // Outputs added after will be preserved (they were inserted while refresh was in progress).
        tx.execute(
            "DELETE FROM token_outputs WHERE reservation_id IS NULL AND added_at < $1",
            &[&refresh_timestamp],
        )
        .await
        .map_err(map_err)?;

        // Build a set of all incoming output IDs for reconciliation
        let incoming_output_ids: HashSet<String> = token_outputs
            .iter()
            .flat_map(|to| to.outputs.iter().map(|o| o.output.id.clone()))
            .collect();

        // Reconcile reservations: find reserved outputs that no longer exist
        let reserved_rows = tx
            .query(
                r"SELECT r.id, o.id AS output_id
                  FROM token_reservations r
                  JOIN token_outputs o ON o.reservation_id = r.id",
                &[],
            )
            .await
            .map_err(map_err)?;

        // Group reserved outputs by reservation ID
        let mut reservation_outputs: HashMap<String, Vec<String>> = HashMap::new();
        for row in &reserved_rows {
            let reservation_id: String = row.get("id");
            let output_id: String = row.get("output_id");
            reservation_outputs
                .entry(reservation_id)
                .or_default()
                .push(output_id);
        }

        // Find reservations that have no valid outputs after reconciliation
        let mut reservations_to_delete: Vec<String> = Vec::new();
        let mut outputs_to_remove_from_reservation: Vec<String> = Vec::new();
        for (reservation_id, output_ids) in &reservation_outputs {
            let valid_ids: Vec<&String> = output_ids
                .iter()
                .filter(|id| incoming_output_ids.contains(*id))
                .collect();
            if valid_ids.is_empty() {
                reservations_to_delete.push(reservation_id.clone());
            } else {
                // Remove individual outputs that no longer exist
                for id in output_ids {
                    if !incoming_output_ids.contains(id) {
                        outputs_to_remove_from_reservation.push(id.clone());
                    }
                }
            }
        }

        // Delete outputs whose reservations are being removed entirely
        if !reservations_to_delete.is_empty() {
            tx.execute(
                "DELETE FROM token_outputs WHERE reservation_id = ANY($1)",
                &[&reservations_to_delete],
            )
            .await
            .map_err(map_err)?;

            tx.execute(
                "DELETE FROM token_reservations WHERE id = ANY($1)",
                &[&reservations_to_delete],
            )
            .await
            .map_err(map_err)?;
        }

        // Delete individual reserved outputs that no longer exist
        if !outputs_to_remove_from_reservation.is_empty() {
            tx.execute(
                "DELETE FROM token_outputs WHERE id = ANY($1)",
                &[&outputs_to_remove_from_reservation],
            )
            .await
            .map_err(map_err)?;

            // Check if any reservations are now empty after removing individual outputs
            let empty_reservations = tx
                .query(
                    r"SELECT r.id FROM token_reservations r
                      LEFT JOIN token_outputs o ON o.reservation_id = r.id
                      WHERE o.id IS NULL",
                    &[],
                )
                .await
                .map_err(map_err)?;
            let empty_ids: Vec<String> =
                empty_reservations.iter().map(|row| row.get("id")).collect();
            if !empty_ids.is_empty() {
                tx.execute(
                    "DELETE FROM token_reservations WHERE id = ANY($1)",
                    &[&empty_ids],
                )
                .await
                .map_err(map_err)?;
            }
        }

        // Collect IDs of currently reserved outputs (that survived reconciliation)
        let reserved_output_ids: HashSet<String> = {
            let rows = tx
                .query(
                    "SELECT id FROM token_outputs WHERE reservation_id IS NOT NULL",
                    &[],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get("id")).collect()
        };

        // Delete metadata not referenced by any remaining outputs or incoming data
        tx.execute(
            r"DELETE FROM token_metadata
              WHERE identifier NOT IN (
                  SELECT DISTINCT token_identifier FROM token_outputs
              )",
            &[],
        )
        .await
        .map_err(map_err)?;

        // Insert new metadata and outputs, excluding spent and reserved
        for to in token_outputs {
            // Upsert metadata
            Self::upsert_metadata(&tx, &to.metadata).await?;

            // Insert outputs that aren't currently reserved or spent
            for output in &to.outputs {
                if reserved_output_ids.contains(&output.output.id)
                    || spent_ids.contains(&output.output.id)
                {
                    continue;
                }
                Self::insert_single_output(&tx, &to.metadata.identifier, output).await?;
            }
        }

        tx.commit().await.map_err(map_err)?;

        trace!(
            "Updated {} token outputs in PostgreSQL",
            token_outputs.len()
        );
        Ok(())
    }

    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError> {
        let client = self.pool.get().await.map_err(map_err)?;

        let rows = client
            .query(
                r"SELECT m.identifier, m.issuer_public_key, m.name, m.ticker, m.decimals,
                         m.max_supply, m.is_freezable, m.creation_entity_public_key,
                         o.id AS output_id, o.owner_public_key, o.revocation_commitment,
                         o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                         o.token_public_key, o.token_amount,
                         o.prev_tx_hash, o.prev_tx_vout, o.reservation_id,
                         r.purpose
                  FROM token_metadata m
                  LEFT JOIN token_outputs o ON o.token_identifier = m.identifier
                  LEFT JOIN token_reservations r ON o.reservation_id = r.id
                  ORDER BY m.identifier, o.token_amount::NUMERIC ASC",
                &[],
            )
            .await
            .map_err(map_err)?;

        let mut map: HashMap<String, TokenOutputsPerStatus> = HashMap::new();

        for row in rows {
            let identifier: String = row.get("identifier");
            if !map.contains_key(&identifier) {
                let metadata = Self::metadata_from_row(&row)?;
                map.insert(
                    identifier.clone(),
                    TokenOutputsPerStatus {
                        metadata,
                        available: Vec::new(),
                        reserved_for_payment: Vec::new(),
                        reserved_for_swap: Vec::new(),
                    },
                );
            }
            let Some(entry) = map.get_mut(&identifier) else {
                continue;
            };

            let output_id: Option<String> = row.get("output_id");
            if output_id.is_none() {
                continue;
            }

            let output = Self::output_from_row(&row)?;
            let purpose: Option<String> = row.get("purpose");

            match purpose.as_deref() {
                Some("Payment") => entry.reserved_for_payment.push(output),
                Some("Swap") => entry.reserved_for_swap.push(output),
                _ => entry.available.push(output),
            }
        }

        Ok(map.into_values().collect())
    }

    async fn get_token_outputs(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<TokenOutputsPerStatus, TokenOutputServiceError> {
        let client = self.pool.get().await.map_err(map_err)?;

        let (where_clause, param): (&str, String) = match filter {
            GetTokenOutputsFilter::Identifier(id) => ("m.identifier = $1", id.to_string()),
            GetTokenOutputsFilter::IssuerPublicKey(pk) => {
                ("m.issuer_public_key = $1", pk.to_string())
            }
        };

        let query = format!(
            r"SELECT m.identifier, m.issuer_public_key, m.name, m.ticker, m.decimals,
                     m.max_supply, m.is_freezable, m.creation_entity_public_key,
                     o.id AS output_id, o.owner_public_key, o.revocation_commitment,
                     o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                     o.token_public_key, o.token_amount,
                     o.prev_tx_hash, o.prev_tx_vout, o.reservation_id,
                     r.purpose
              FROM token_metadata m
              LEFT JOIN token_outputs o ON o.token_identifier = m.identifier
              LEFT JOIN token_reservations r ON o.reservation_id = r.id
              WHERE {where_clause}
              ORDER BY o.token_amount::NUMERIC ASC"
        );

        let rows = client.query(&query, &[&param]).await.map_err(map_err)?;

        if rows.is_empty() {
            return Err(TokenOutputServiceError::Generic(
                "Token outputs not found".to_string(),
            ));
        }

        let metadata = Self::metadata_from_row(&rows[0])?;
        let mut result = TokenOutputsPerStatus {
            metadata,
            available: Vec::new(),
            reserved_for_payment: Vec::new(),
            reserved_for_swap: Vec::new(),
        };

        for row in &rows {
            let output_id: Option<String> = row.get("output_id");
            if output_id.is_none() {
                continue;
            }

            let output = Self::output_from_row(row)?;
            let purpose: Option<String> = row.get("purpose");

            match purpose.as_deref() {
                Some("Payment") => result.reserved_for_payment.push(output),
                Some("Swap") => result.reserved_for_swap.push(output),
                _ => result.available.push(output),
            }
        }

        Ok(result)
    }

    #[allow(clippy::cast_possible_wrap)]
    async fn insert_token_outputs(
        &self,
        token_outputs: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError> {
        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        // Upsert metadata
        Self::upsert_metadata(&tx, &token_outputs.metadata).await?;

        // Remove inserted output IDs from spent markers (output returned to us)
        let output_ids: Vec<String> = token_outputs
            .outputs
            .iter()
            .map(|o| o.output.id.clone())
            .collect();
        if !output_ids.is_empty() {
            tx.execute(
                "DELETE FROM token_spent_outputs WHERE output_id = ANY($1)",
                &[&output_ids],
            )
            .await
            .map_err(map_err)?;
        }

        // Insert outputs where id not already present
        for output in &token_outputs.outputs {
            Self::insert_single_output(&tx, &token_outputs.metadata.identifier, output).await?;
        }

        tx.commit().await.map_err(map_err)?;

        trace!(
            "Inserted {} token outputs into PostgreSQL",
            token_outputs.outputs.len()
        );
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn reserve_token_outputs(
        &self,
        token_identifier: &str,
        target: ReservationTarget,
        purpose: TokenReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        match target {
            ReservationTarget::MinTotalValue(amount) => {
                if amount == 0 {
                    return Err(TokenOutputServiceError::Generic(
                        "Amount to reserve must be greater than zero".to_string(),
                    ));
                }
            }
            ReservationTarget::MaxOutputCount(count) => {
                if count == 0 {
                    return Err(TokenOutputServiceError::Generic(
                        "Count to reserve must be greater than zero".to_string(),
                    ));
                }
            }
        }

        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        Self::acquire_write_lock(&tx).await?;

        // Get metadata
        let metadata_row = tx
            .query_opt(
                "SELECT * FROM token_metadata WHERE identifier = $1",
                &[&token_identifier],
            )
            .await
            .map_err(map_err)?
            .ok_or_else(|| {
                TokenOutputServiceError::Generic(format!(
                    "Token outputs not found for identifier: {token_identifier}"
                ))
            })?;
        let metadata = Self::metadata_from_row(&metadata_row)?;

        // Get available (non-reserved) outputs
        let rows = tx
            .query(
                r"SELECT o.id AS output_id, o.owner_public_key, o.revocation_commitment,
                         o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                         o.token_public_key, o.token_amount, o.prev_tx_hash, o.prev_tx_vout,
                         o.token_identifier AS identifier
                  FROM token_outputs o
                  WHERE o.token_identifier = $1 AND o.reservation_id IS NULL",
                &[&token_identifier],
            )
            .await
            .map_err(map_err)?;

        let mut outputs: Vec<TokenOutputWithPrevOut> = rows
            .iter()
            .map(Self::output_from_row)
            .collect::<Result<Vec<_>, _>>()?;

        // Filter by preferred if provided
        if let Some(ref preferred) = preferred_outputs {
            let preferred_ids: HashSet<&str> =
                preferred.iter().map(|p| p.output.id.as_str()).collect();
            outputs.retain(|o| preferred_ids.contains(o.output.id.as_str()));
        }

        // Check sufficiency for MinTotalValue
        if let ReservationTarget::MinTotalValue(amount) = target
            && outputs.iter().map(|o| o.output.token_amount).sum::<u128>() < amount
        {
            return Err(TokenOutputServiceError::InsufficientFunds);
        }

        // Select outputs using the same logic as InMemory
        let selected_outputs = if let ReservationTarget::MinTotalValue(amount) = target
            && let Some(output) = outputs.iter().find(|o| o.output.token_amount == amount)
        {
            vec![output.clone()]
        } else {
            match selection_strategy {
                None | Some(SelectionStrategy::SmallestFirst) => {
                    outputs.sort_by_key(|o| o.output.token_amount);
                }
                Some(SelectionStrategy::LargestFirst) => {
                    outputs.sort_by_key(|o| std::cmp::Reverse(o.output.token_amount));
                }
            }

            match target {
                ReservationTarget::MinTotalValue(amount) => {
                    let mut selected = Vec::new();
                    let mut remaining = amount;
                    for output in outputs {
                        if remaining == 0 {
                            break;
                        }
                        selected.push(output.clone());
                        remaining = remaining.saturating_sub(output.output.token_amount);
                    }
                    if remaining > 0 {
                        return Err(TokenOutputServiceError::InsufficientFunds);
                    }
                    selected
                }
                ReservationTarget::MaxOutputCount(count) => {
                    outputs.truncate(count);
                    outputs
                }
            }
        };

        // Create reservation
        let reservation_id = Uuid::now_v7().to_string();
        let purpose_str = match purpose {
            TokenReservationPurpose::Payment => "Payment",
            TokenReservationPurpose::Swap => "Swap",
        };

        tx.execute(
            "INSERT INTO token_reservations (id, purpose) VALUES ($1, $2)",
            &[&reservation_id, &purpose_str],
        )
        .await
        .map_err(map_err)?;

        // Set reservation_id on selected outputs
        let selected_ids: Vec<String> = selected_outputs
            .iter()
            .map(|o| o.output.id.clone())
            .collect();
        tx.execute(
            "UPDATE token_outputs SET reservation_id = $1 WHERE id = ANY($2)",
            &[&reservation_id, &selected_ids],
        )
        .await
        .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;

        let reservation_token_outputs = TokenOutputs {
            metadata,
            outputs: selected_outputs,
        };

        Ok(TokenOutputsReservation::new(
            reservation_id,
            reservation_token_outputs,
        ))
    }

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        Self::acquire_write_lock(&tx).await?;

        // Clear reservation_id from outputs (ON DELETE SET NULL would do this,
        // but we do it explicitly for clarity)
        tx.execute(
            "UPDATE token_outputs SET reservation_id = NULL WHERE reservation_id = $1",
            &[id],
        )
        .await
        .map_err(map_err)?;

        // Delete the reservation
        tx.execute("DELETE FROM token_reservations WHERE id = $1", &[id])
            .await
            .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;

        trace!("Canceled token outputs reservation: {}", id);
        Ok(())
    }

    async fn finalize_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let mut client = self.pool.get().await.map_err(map_err)?;
        let tx = client.transaction().await.map_err(map_err)?;

        Self::acquire_write_lock(&tx).await?;

        // Get reservation purpose and reserved output IDs
        let reservation_row = tx
            .query_opt(
                "SELECT purpose FROM token_reservations WHERE id = $1",
                &[id],
            )
            .await
            .map_err(map_err)?;

        let Some(reservation_row) = reservation_row else {
            warn!("Tried to finalize a non existing reservation");
            return Ok(());
        };

        let is_swap = reservation_row.get::<_, String>("purpose") == "Swap";

        // Get reserved output IDs and mark them as spent
        let reserved_output_ids: Vec<String> = {
            let rows = tx
                .query(
                    "SELECT id FROM token_outputs WHERE reservation_id = $1",
                    &[id],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get(0)).collect()
        };

        // Batch insert spent output markers
        if !reserved_output_ids.is_empty() {
            tx.execute(
                r"INSERT INTO token_spent_outputs (output_id)
                  SELECT * FROM UNNEST($1::text[])
                  ON CONFLICT DO NOTHING",
                &[&reserved_output_ids],
            )
            .await
            .map_err(map_err)?;
        }

        // Delete reserved outputs
        tx.execute("DELETE FROM token_outputs WHERE reservation_id = $1", &[id])
            .await
            .map_err(map_err)?;

        // Delete the reservation
        tx.execute("DELETE FROM token_reservations WHERE id = $1", &[id])
            .await
            .map_err(map_err)?;

        // If this was a swap reservation, update last_completed_at
        if is_swap {
            tx.execute(
                "UPDATE token_swap_status SET last_completed_at = NOW() WHERE id = 1",
                &[],
            )
            .await
            .map_err(map_err)?;
        }

        // Clean up any orphaned metadata
        tx.execute(
            r"DELETE FROM token_metadata
              WHERE identifier NOT IN (
                  SELECT DISTINCT token_identifier FROM token_outputs
              )",
            &[],
        )
        .await
        .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;

        trace!("Finalized token outputs reservation: {}", id);
        Ok(())
    }

    async fn now(&self) -> Result<SystemTime, TokenOutputServiceError> {
        let client = self.pool.get().await.map_err(map_err)?;
        let row = client
            .query_one("SELECT NOW()", &[])
            .await
            .map_err(map_err)?;
        let now: chrono::DateTime<chrono::Utc> = row.get(0);
        Ok(now.into())
    }
}

impl PostgresTokenStore {
    /// Creates a new `PostgresTokenStore`.
    #[cfg(test)]
    pub async fn new(config: PostgresStorageConfig) -> Result<Self, StorageError> {
        let pool = create_pool(&config)?;
        Self::new_with_pool(pool).await
    }

    /// Creates a new `PostgresTokenStore` using an existing connection pool.
    ///
    /// This allows sharing a single pool across multiple store implementations.
    pub async fn new_with_pool(pool: Pool) -> Result<Self, StorageError> {
        let store = Self { pool };
        store.migrate().await?;

        Ok(store)
    }

    /// Runs database migrations for token store tables.
    async fn migrate(&self) -> Result<(), StorageError> {
        run_migrations(&self.pool, TOKEN_MIGRATIONS_TABLE, &Self::migrations()).await
    }

    /// Returns the list of migrations for the token store.
    fn migrations() -> Vec<&'static [&'static str]> {
        vec![
            // Migration 1: Token store tables with race condition protection
            &[
                "CREATE TABLE IF NOT EXISTS token_metadata (
                    identifier TEXT PRIMARY KEY,
                    issuer_public_key TEXT NOT NULL,
                    name TEXT NOT NULL,
                    ticker TEXT NOT NULL,
                    decimals INTEGER NOT NULL,
                    max_supply TEXT NOT NULL,
                    is_freezable BOOLEAN NOT NULL,
                    creation_entity_public_key TEXT
                )",
                "CREATE INDEX IF NOT EXISTS idx_token_metadata_issuer_pk
                    ON token_metadata (issuer_public_key)",
                "CREATE TABLE IF NOT EXISTS token_reservations (
                    id TEXT PRIMARY KEY,
                    purpose TEXT NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )",
                "CREATE TABLE IF NOT EXISTS token_outputs (
                    id TEXT PRIMARY KEY,
                    token_identifier TEXT NOT NULL REFERENCES token_metadata(identifier),
                    owner_public_key TEXT NOT NULL,
                    revocation_commitment TEXT NOT NULL,
                    withdraw_bond_sats BIGINT NOT NULL,
                    withdraw_relative_block_locktime BIGINT NOT NULL,
                    token_public_key TEXT,
                    token_amount TEXT NOT NULL,
                    prev_tx_hash TEXT NOT NULL,
                    prev_tx_vout INTEGER NOT NULL,
                    reservation_id TEXT REFERENCES token_reservations(id) ON DELETE SET NULL,
                    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )",
                "CREATE INDEX IF NOT EXISTS idx_token_outputs_identifier
                    ON token_outputs (token_identifier)",
                "CREATE INDEX IF NOT EXISTS idx_token_outputs_reservation
                    ON token_outputs (reservation_id) WHERE reservation_id IS NOT NULL",
                "CREATE TABLE IF NOT EXISTS token_spent_outputs (
                    output_id TEXT PRIMARY KEY,
                    spent_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )",
                "CREATE TABLE IF NOT EXISTS token_swap_status (
                    id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
                    last_completed_at TIMESTAMPTZ
                )",
                "INSERT INTO token_swap_status (id) VALUES (1) ON CONFLICT DO NOTHING",
            ],
        ]
    }

    /// Acquires an exclusive advisory lock for write operations.
    async fn acquire_write_lock(
        tx: &tokio_postgres::Transaction<'_>,
    ) -> Result<(), TokenOutputServiceError> {
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&TOKEN_STORE_WRITE_LOCK_KEY],
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }

    /// Inserts a single output into the database.
    #[allow(clippy::cast_possible_wrap)]
    async fn insert_single_output(
        tx: &tokio_postgres::Transaction<'_>,
        token_identifier: &str,
        output: &TokenOutputWithPrevOut,
    ) -> Result<(), TokenOutputServiceError> {
        tx.execute(
            r"INSERT INTO token_outputs
                (id, token_identifier, owner_public_key, revocation_commitment,
                 withdraw_bond_sats, withdraw_relative_block_locktime,
                 token_public_key, token_amount, prev_tx_hash, prev_tx_vout, added_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
              ON CONFLICT (id) DO NOTHING",
            &[
                &output.output.id,
                &token_identifier,
                &output.output.owner_public_key.to_string(),
                &output.output.revocation_commitment,
                &(output.output.withdraw_bond_sats as i64),
                &(output.output.withdraw_relative_block_locktime as i64),
                &output.output.token_public_key.map(|pk| pk.to_string()),
                &output.output.token_amount.to_string(),
                &output.prev_tx_hash,
                &(output.prev_tx_vout as i32),
            ],
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }

    /// Upserts token metadata.
    #[allow(clippy::cast_possible_wrap)]
    async fn upsert_metadata(
        tx: &tokio_postgres::Transaction<'_>,
        metadata: &TokenMetadata,
    ) -> Result<(), TokenOutputServiceError> {
        tx.execute(
            r"INSERT INTO token_metadata
                (identifier, issuer_public_key, name, ticker, decimals, max_supply,
                 is_freezable, creation_entity_public_key)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
              ON CONFLICT (identifier) DO UPDATE SET
                issuer_public_key = EXCLUDED.issuer_public_key,
                name = EXCLUDED.name,
                ticker = EXCLUDED.ticker,
                decimals = EXCLUDED.decimals,
                max_supply = EXCLUDED.max_supply,
                is_freezable = EXCLUDED.is_freezable,
                creation_entity_public_key = EXCLUDED.creation_entity_public_key",
            &[
                &metadata.identifier,
                &metadata.issuer_public_key.to_string(),
                &metadata.name,
                &metadata.ticker,
                &(metadata.decimals as i32),
                &metadata.max_supply.to_string(),
                &metadata.is_freezable,
                &metadata.creation_entity_public_key.map(|pk| pk.to_string()),
            ],
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }

    /// Cleans up spent markers older than the cleanup threshold relative to refresh timestamp.
    async fn cleanup_spent_markers(
        tx: &tokio_postgres::Transaction<'_>,
        refresh_timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), TokenOutputServiceError> {
        let threshold = chrono::Duration::milliseconds(SPENT_MARKER_CLEANUP_THRESHOLD_MS);
        let cleanup_cutoff = refresh_timestamp
            .checked_sub_signed(threshold)
            .unwrap_or(refresh_timestamp);

        tx.execute(
            "DELETE FROM token_spent_outputs WHERE spent_at < $1",
            &[&cleanup_cutoff],
        )
        .await
        .map_err(map_err)?;

        Ok(())
    }

    /// Parses a `TokenMetadata` from a database row.
    #[allow(clippy::cast_sign_loss)]
    fn metadata_from_row(
        row: &tokio_postgres::Row,
    ) -> Result<TokenMetadata, TokenOutputServiceError> {
        let identifier: String = row.get("identifier");
        let issuer_pk_str: String = row.get("issuer_public_key");
        let name: String = row.get("name");
        let ticker: String = row.get("ticker");
        let decimals: i32 = row.get("decimals");
        let max_supply_str: String = row.get("max_supply");
        let is_freezable: bool = row.get("is_freezable");
        let creation_entity_pk_str: Option<String> = row.get("creation_entity_public_key");

        Ok(TokenMetadata {
            identifier,
            issuer_public_key: issuer_pk_str.parse().map_err(map_err)?,
            name,
            ticker,
            decimals: decimals as u32,
            max_supply: max_supply_str.parse().map_err(map_err)?,
            is_freezable,
            creation_entity_public_key: creation_entity_pk_str
                .map(|s| s.parse().map_err(map_err))
                .transpose()?,
        })
    }

    /// Parses a `TokenOutputWithPrevOut` from a database row.
    #[allow(clippy::cast_sign_loss)]
    fn output_from_row(
        row: &tokio_postgres::Row,
    ) -> Result<TokenOutputWithPrevOut, TokenOutputServiceError> {
        let output_id: String = row.get("output_id");
        let owner_pk_str: String = row.get("owner_public_key");
        let revocation_commitment: String = row.get("revocation_commitment");
        let withdraw_bond_sats: i64 = row.get("withdraw_bond_sats");
        let withdraw_relative_block_locktime: i64 = row.get("withdraw_relative_block_locktime");
        let token_pk_str: Option<String> = row.get("token_public_key");
        let token_amount_str: String = row.get("token_amount");
        let prev_tx_hash: String = row.get("prev_tx_hash");
        let prev_tx_vout: i32 = row.get("prev_tx_vout");

        // Get token_identifier from the row if available, otherwise fall back
        let token_identifier: String = row
            .try_get("token_identifier")
            .unwrap_or_else(|_| row.get("identifier"));

        Ok(TokenOutputWithPrevOut {
            output: TokenOutput {
                id: output_id,
                owner_public_key: owner_pk_str.parse().map_err(map_err)?,
                revocation_commitment,
                withdraw_bond_sats: withdraw_bond_sats as u64,
                withdraw_relative_block_locktime: withdraw_relative_block_locktime as u64,
                token_public_key: token_pk_str
                    .map(|s| s.parse().map_err(map_err))
                    .transpose()?,
                token_identifier,
                token_amount: token_amount_str.parse().map_err(map_err)?,
            },
            prev_tx_hash,
            prev_tx_vout: prev_tx_vout as u32,
        })
    }
}

/// Maps any error to `TokenOutputServiceError`.
fn map_err<E: std::fmt::Display>(e: E) -> TokenOutputServiceError {
    TokenOutputServiceError::Generic(e.to_string())
}

/// Creates a `PostgresTokenStore` instance for use with the SDK, using an existing pool.
pub async fn create_postgres_token_store(
    pool: Pool,
) -> Result<Arc<dyn TokenOutputStore>, StorageError> {
    Ok(Arc::new(PostgresTokenStore::new_with_pool(pool).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::token_store_tests as shared_tests;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::postgres::Postgres;

    /// Helper struct that holds the container and store together.
    /// The container must be kept alive for the duration of the test.
    struct PostgresTokenStoreTestFixture {
        store: PostgresTokenStore,
        #[allow(dead_code)]
        container: ContainerAsync<Postgres>,
    }

    impl PostgresTokenStoreTestFixture {
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
                PostgresTokenStore::new(PostgresStorageConfig::with_defaults(connection_string))
                    .await
                    .expect("Failed to create PostgresTokenStore");

            Self { store, container }
        }
    }

    // ==================== Shared tests ====================

    #[tokio::test]
    async fn test_set_tokens_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_set_tokens_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_token_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_get_token_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_tokens_outputs_with_update() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_set_tokens_outputs_with_update(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_insert_token_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_insert_token_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_and_cancel() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_and_cancel(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_and_finalize() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_and_finalize(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_and_set_add_output() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_and_set_add_output(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_and_set_remove_reserved_output() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_and_set_remove_reserved_output(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_multiple_parallel_reservations() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_multiple_parallel_reservations(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_with_preferred_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_with_preferred_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_insufficient_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_insufficient_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_nonexistent_token() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_nonexistent_token(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_exact_amount_match() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_exact_amount_match(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_multiple_outputs_combination() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_multiple_outputs_combination(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_all_available_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_all_available_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_with_preferred_outputs_insufficient() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_with_preferred_outputs_insufficient(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_zero_amount() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_zero_amount(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_reservation() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_cancel_nonexistent_reservation(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_nonexistent_reservation() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_finalize_nonexistent_reservation(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_removes_all_tokens() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_set_removes_all_tokens(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_single_large_output() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_single_large_output(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_token_outputs_none_found() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_get_token_outputs_none_found(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_reconciles_reservation_with_empty_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_set_reconciles_reservation_with_empty_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_selection_strategy_smallest_first() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_selection_strategy_smallest_first(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_selection_strategy_largest_first() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_selection_strategy_largest_first(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_reserve_max_output_count_smallest_first() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_max_output_count_smallest_first(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_max_output_count_largest_first() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_max_output_count_largest_first(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_max_output_count_more_than_available() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_max_output_count_more_than_available(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_max_output_count_zero_rejected() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_max_output_count_zero_rejected(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_for_payment_affects_balance() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_for_payment_affects_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_for_swap_does_not_affect_balance() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_for_swap_does_not_affect_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_mixed_reservation_purposes_balance() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_mixed_reservation_purposes_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_tokens_outputs_skipped_during_active_swap() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_set_tokens_outputs_skipped_during_active_swap(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_tokens_outputs_skipped_after_swap_completes_during_refresh() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_set_tokens_outputs_skipped_after_swap_completes_during_refresh(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn test_insert_outputs_preserved_by_set_tokens_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_insert_outputs_preserved_by_set_tokens_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_spent_outputs_not_restored_by_set_tokens_outputs() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_spent_outputs_not_restored_by_set_tokens_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_swap_marks_spent_and_tracks_completion() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_finalize_swap_marks_spent_and_tracks_completion(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_insert_outputs_clears_spent_status() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_insert_outputs_clears_spent_status(&fixture.store).await;
    }
}
