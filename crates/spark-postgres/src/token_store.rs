//! `PostgreSQL`-backed implementation of the `TokenOutputStore` trait.
//!
//! This module provides a persistent token output store backed by `PostgreSQL`,
//! suitable for server-side or multi-instance deployments where
//! in-memory storage is insufficient.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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

use crate::config::PostgresStorageConfig;
use crate::error::PostgresError;
use crate::migrations::run_migrations;
use crate::pool::create_pool;

/// Name of the schema migrations table for `PostgresTokenStore`.
const TOKEN_MIGRATIONS_TABLE: &str = "token_schema_migrations";

/// Advisory-lock classid for the token store. Combined with a per-tenant
/// `objid` (derived from the identity pubkey) so two tenants don't serialize
/// on each other's writes — only same-tenant writes share the same lock.
const TOKEN_STORE_LOCK_CLASSID: i32 = 0x746F_6B6E; // "tokn" as hex

/// Spent markers are kept in the database for this duration to support multiple
/// SDK instances sharing the same postgres database. During `set_tokens_outputs`, spent
/// markers older than `refresh_timestamp` are ignored (treated as deleted).
/// Actual deletion only happens for markers older than this threshold.
const SPENT_MARKER_CLEANUP_THRESHOLD_MS: i64 = 5 * 60 * 1000; // 5 minutes

/// Reservations whose `created_at` is older than this are considered stale and are
/// dropped at the start of `set_tokens_outputs`. Matches the tree store's timeout.
const RESERVATION_TIMEOUT_SECS: f64 = 300.0; // 5 minutes

/// `PostgreSQL`-backed token output store implementation.
///
/// Each instance is scoped to a single tenant identity so multiple tenants
/// can share one Postgres database without cross-pollinating token state.
pub struct PostgresTokenStore {
    pool: Pool,
    /// 33-byte secp256k1 compressed pubkey identifying this tenant. All reads
    /// and writes are filtered by `user_id = self.identity`.
    identity: Vec<u8>,
    /// Stable per-tenant lock objid derived from `identity`. Used as the
    /// second argument to `pg_advisory_xact_lock(classid, objid)`.
    lock_objid: i32,
}

/// Derive a stable 32-bit lock objid from a 33-byte secp256k1 pubkey.
/// Reuses the same scheme as the tree store so behavior is consistent.
fn identity_lock_objid(identity: &[u8]) -> i32 {
    let buf: [u8; 4] = identity
        .rchunks(4)
        .next()
        .and_then(|tail| tail.try_into().ok())
        .unwrap_or([0u8; 4]);
    i32::from_be_bytes(buf)
}

/// Builds the multi-tenant scoping migration for the token store. Adds
/// `user_id BYTEA` to every per-user table (including `token_metadata` —
/// metadata is per-tenant to avoid 0-balance leakage for tokens a tenant
/// never owned), backfills with the connecting tenant, and rewrites primary
/// keys / FKs to lead with `user_id`. The composite FK from `token_outputs`
/// to `token_reservations` uses NO ACTION (the default) — column-list SET
/// NULL is PG15+ and a whole-row SET NULL would null `user_id` (NOT NULL).
fn token_store_multi_tenant_migration(identity: &[u8]) -> Vec<String> {
    let id_hex = hex::encode(identity);
    let id_lit = format!("'\\x{id_hex}'::bytea");

    vec![
        // Drop dependent FKs FIRST so we can rebuild the parent PKs they
        // reference. Inline `REFERENCES` clauses get auto-named
        // `<table>_<column>_fkey` so we drop those exact names if present.
        "ALTER TABLE token_outputs DROP CONSTRAINT IF EXISTS token_outputs_reservation_id_fkey"
            .to_string(),
        "ALTER TABLE token_outputs DROP CONSTRAINT IF EXISTS token_outputs_token_identifier_fkey"
            .to_string(),
        // token_metadata: scope per-tenant. Required even though metadata
        // never structurally collides — leaking a token's mere existence
        // (e.g. a 0-balance entry for a token a tenant never held) would be
        // a privacy regression.
        "ALTER TABLE token_metadata ADD COLUMN user_id BYTEA".to_string(),
        format!("UPDATE token_metadata SET user_id = {id_lit}"),
        "ALTER TABLE token_metadata \
         ALTER COLUMN user_id SET NOT NULL, \
         DROP CONSTRAINT IF EXISTS token_metadata_pkey, \
         ADD PRIMARY KEY (user_id, identifier)"
            .to_string(),
        "DROP INDEX IF EXISTS idx_token_metadata_issuer_pk".to_string(),
        "CREATE INDEX idx_token_metadata_user_issuer_pk \
         ON token_metadata (user_id, issuer_public_key)"
            .to_string(),
        // token_reservations: scope by user_id.
        "ALTER TABLE token_reservations ADD COLUMN user_id BYTEA".to_string(),
        format!("UPDATE token_reservations SET user_id = {id_lit}"),
        "ALTER TABLE token_reservations \
         ALTER COLUMN user_id SET NOT NULL, \
         DROP CONSTRAINT IF EXISTS token_reservations_pkey, \
         ADD PRIMARY KEY (user_id, id)"
            .to_string(),
        // token_outputs: scope by user_id, rekey, re-add composite FKs.
        "ALTER TABLE token_outputs ADD COLUMN user_id BYTEA".to_string(),
        format!("UPDATE token_outputs SET user_id = {id_lit}"),
        "ALTER TABLE token_outputs \
         ALTER COLUMN user_id SET NOT NULL, \
         DROP CONSTRAINT IF EXISTS token_outputs_pkey, \
         ADD PRIMARY KEY (user_id, id), \
         ADD FOREIGN KEY (user_id, token_identifier) \
            REFERENCES token_metadata(user_id, identifier), \
         ADD FOREIGN KEY (user_id, reservation_id) \
            REFERENCES token_reservations(user_id, id)"
            .to_string(),
        "DROP INDEX IF EXISTS idx_token_outputs_identifier".to_string(),
        "DROP INDEX IF EXISTS idx_token_outputs_reservation".to_string(),
        "CREATE INDEX idx_token_outputs_user_identifier \
         ON token_outputs (user_id, token_identifier)"
            .to_string(),
        "CREATE INDEX idx_token_outputs_user_reservation \
         ON token_outputs (user_id, reservation_id) \
         WHERE reservation_id IS NOT NULL"
            .to_string(),
        // token_spent_outputs: scope by user_id.
        "ALTER TABLE token_spent_outputs ADD COLUMN user_id BYTEA".to_string(),
        format!("UPDATE token_spent_outputs SET user_id = {id_lit}"),
        "ALTER TABLE token_spent_outputs \
         ALTER COLUMN user_id SET NOT NULL, \
         DROP CONSTRAINT IF EXISTS token_spent_outputs_pkey, \
         ADD PRIMARY KEY (user_id, output_id)"
            .to_string(),
        // token_swap_status was a singleton (PK id=1, CHECK id=1). Drop the
        // id column (CASCADE removes both PK and CHECK), then re-key by
        // user_id so each tenant has its own swap-status row.
        "ALTER TABLE token_swap_status DROP COLUMN id CASCADE".to_string(),
        "ALTER TABLE token_swap_status ADD COLUMN user_id BYTEA".to_string(),
        format!("UPDATE token_swap_status SET user_id = {id_lit}"),
        "ALTER TABLE token_swap_status \
         ALTER COLUMN user_id SET NOT NULL, \
         ADD PRIMARY KEY (user_id)"
            .to_string(),
    ]
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

        self.acquire_write_lock(&tx).await?;

        // Drop expired reservations BEFORE evaluating has_active_swap, otherwise a stale
        // Swap reservation (from a crashed client or a swap whose finalize/cancel never
        // ran) keeps has_active_swap true forever, which makes set_tokens_outputs
        // early-return and never reach any subsequent reconciliation. The reservation
        // pins itself in place and the local token-output set freezes.
        self.cleanup_stale_reservations(&tx).await?;

        // Skip if swap is active or completed during this refresh
        let (has_active_swap, swap_completed_during_refresh): (bool, bool) = {
            let row = tx
                .query_one(
                    r"
                    SELECT
                        EXISTS(
                            SELECT 1 FROM token_reservations
                            WHERE user_id = $1 AND purpose = 'Swap'
                        ),
                        COALESCE(
                            (SELECT last_completed_at >= $2
                             FROM token_swap_status WHERE user_id = $1),
                            FALSE
                        )
                    ",
                    &[&self.identity, &refresh_timestamp],
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
        self.cleanup_spent_markers(&tx, refresh_timestamp).await?;

        // Get recent spent output IDs (spent_at >= refresh_timestamp).
        // Older spent markers are ignored - if the refresh started after the spend,
        // operators had time to process it.
        let spent_ids: HashSet<String> = {
            let rows = tx
                .query(
                    "SELECT output_id FROM token_spent_outputs \
                     WHERE user_id = $1 AND spent_at >= $2",
                    &[&self.identity, &refresh_timestamp],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get(0)).collect()
        };

        // Delete non-reserved outputs added BEFORE the refresh started.
        // Outputs added after will be preserved (they were inserted while refresh was in progress).
        tx.execute(
            "DELETE FROM token_outputs \
             WHERE user_id = $1 AND reservation_id IS NULL AND added_at < $2",
            &[&self.identity, &refresh_timestamp],
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
                  JOIN token_outputs o
                    ON o.reservation_id = r.id AND o.user_id = r.user_id
                  WHERE r.user_id = $1",
                &[&self.identity],
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
                "DELETE FROM token_outputs WHERE user_id = $1 AND reservation_id = ANY($2)",
                &[&self.identity, &reservations_to_delete],
            )
            .await
            .map_err(map_err)?;

            tx.execute(
                "DELETE FROM token_reservations WHERE user_id = $1 AND id = ANY($2)",
                &[&self.identity, &reservations_to_delete],
            )
            .await
            .map_err(map_err)?;
        }

        // Delete individual reserved outputs that no longer exist
        if !outputs_to_remove_from_reservation.is_empty() {
            tx.execute(
                "DELETE FROM token_outputs WHERE user_id = $1 AND id = ANY($2)",
                &[&self.identity, &outputs_to_remove_from_reservation],
            )
            .await
            .map_err(map_err)?;

            // Check if any reservations are now empty after removing individual outputs
            let empty_reservations = tx
                .query(
                    r"SELECT r.id FROM token_reservations r
                      LEFT JOIN token_outputs o
                        ON o.reservation_id = r.id AND o.user_id = r.user_id
                      WHERE r.user_id = $1 AND o.id IS NULL",
                    &[&self.identity],
                )
                .await
                .map_err(map_err)?;
            let empty_ids: Vec<String> =
                empty_reservations.iter().map(|row| row.get("id")).collect();
            if !empty_ids.is_empty() {
                tx.execute(
                    "DELETE FROM token_reservations WHERE user_id = $1 AND id = ANY($2)",
                    &[&self.identity, &empty_ids],
                )
                .await
                .map_err(map_err)?;
            }
        }

        // Collect IDs of currently reserved outputs (that survived reconciliation)
        let reserved_output_ids: HashSet<String> = {
            let rows = tx
                .query(
                    "SELECT id FROM token_outputs \
                     WHERE user_id = $1 AND reservation_id IS NOT NULL",
                    &[&self.identity],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get("id")).collect()
        };

        // Delete metadata not referenced by any remaining outputs (per-tenant).
        tx.execute(
            r"DELETE FROM token_metadata
              WHERE user_id = $1
                AND identifier NOT IN (
                    SELECT DISTINCT token_identifier
                    FROM token_outputs WHERE user_id = $1
                )",
            &[&self.identity],
        )
        .await
        .map_err(map_err)?;

        // Insert new metadata and outputs, excluding spent and reserved
        for to in token_outputs {
            // Upsert metadata
            self.upsert_metadata(&tx, &to.metadata).await?;

            // Insert outputs that aren't currently reserved or spent
            for output in &to.outputs {
                if reserved_output_ids.contains(&output.output.id)
                    || spent_ids.contains(&output.output.id)
                {
                    continue;
                }
                self.insert_single_output(&tx, &to.metadata.identifier, output)
                    .await?;
            }
        }

        tx.commit().await.map_err(map_err)?;

        trace!(
            "Updated {} token outputs in PostgreSQL",
            token_outputs.len()
        );
        Ok(())
    }

    async fn get_token_balances(
        &self,
    ) -> Result<Vec<(TokenMetadata, u128)>, TokenOutputServiceError> {
        let client = self.pool.get().await.map_err(map_err)?;
        let rows = client
            .query(
                r"SELECT m.identifier, m.issuer_public_key, m.name, m.ticker, m.decimals,
                         m.max_supply, m.is_freezable, m.creation_entity_public_key,
                         COALESCE(SUM(
                            CASE
                              WHEN o.reservation_id IS NULL THEN o.token_amount::numeric
                              WHEN r.purpose = 'Swap' THEN o.token_amount::numeric
                              ELSE 0
                            END
                         ), 0)::text AS balance
                  FROM token_metadata m
                  JOIN token_outputs o
                    ON o.token_identifier = m.identifier AND o.user_id = m.user_id
                  LEFT JOIN token_reservations r
                    ON o.reservation_id = r.id AND o.user_id = r.user_id
                  WHERE m.user_id = $1
                  GROUP BY m.identifier, m.issuer_public_key, m.name, m.ticker,
                           m.decimals, m.max_supply, m.is_freezable, m.creation_entity_public_key",
                &[&self.identity],
            )
            .await
            .map_err(map_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let metadata = Self::metadata_from_row(&row)?;
            let balance_str: String = row.get("balance");
            let balance: u128 = balance_str.parse().map_err(map_err)?;
            out.push((metadata, balance));
        }
        Ok(out)
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
                  LEFT JOIN token_outputs o
                    ON o.token_identifier = m.identifier AND o.user_id = m.user_id
                  LEFT JOIN token_reservations r
                    ON o.reservation_id = r.id AND o.user_id = r.user_id
                  WHERE m.user_id = $1
                  ORDER BY m.identifier, o.token_amount::NUMERIC ASC",
                &[&self.identity],
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
              LEFT JOIN token_outputs o
                ON o.token_identifier = m.identifier AND o.user_id = m.user_id
              LEFT JOIN token_reservations r
                ON o.reservation_id = r.id AND o.user_id = r.user_id
              WHERE m.user_id = $2 AND {where_clause}
              ORDER BY o.token_amount::NUMERIC ASC"
        );

        let rows = client
            .query(&query, &[&param, &self.identity])
            .await
            .map_err(map_err)?;

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
        self.upsert_metadata(&tx, &token_outputs.metadata).await?;

        // Remove inserted output IDs from spent markers (output returned to us)
        let output_ids: Vec<String> = token_outputs
            .outputs
            .iter()
            .map(|o| o.output.id.clone())
            .collect();
        if !output_ids.is_empty() {
            tx.execute(
                "DELETE FROM token_spent_outputs WHERE user_id = $1 AND output_id = ANY($2)",
                &[&self.identity, &output_ids],
            )
            .await
            .map_err(map_err)?;
        }

        // Insert outputs where id not already present
        for output in &token_outputs.outputs {
            self.insert_single_output(&tx, &token_outputs.metadata.identifier, output)
                .await?;
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

        self.acquire_write_lock(&tx).await?;

        // Get metadata
        let metadata_row = tx
            .query_opt(
                "SELECT * FROM token_metadata WHERE user_id = $1 AND identifier = $2",
                &[&self.identity, &token_identifier],
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
                  WHERE o.user_id = $1
                    AND o.token_identifier = $2
                    AND o.reservation_id IS NULL",
                &[&self.identity, &token_identifier],
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
            "INSERT INTO token_reservations (user_id, id, purpose) VALUES ($1, $2, $3)",
            &[&self.identity, &reservation_id, &purpose_str],
        )
        .await
        .map_err(map_err)?;

        // Set reservation_id on selected outputs
        let selected_ids: Vec<String> = selected_outputs
            .iter()
            .map(|o| o.output.id.clone())
            .collect();
        tx.execute(
            "UPDATE token_outputs SET reservation_id = $1 \
             WHERE user_id = $3 AND id = ANY($2)",
            &[&reservation_id, &selected_ids, &self.identity],
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

        // Clear reservation_id from outputs first — the composite FK uses NO
        // ACTION (column-list SET NULL is PG15+ and a whole-row SET NULL would
        // null user_id, which is NOT NULL).
        tx.execute(
            "UPDATE token_outputs SET reservation_id = NULL \
             WHERE user_id = $1 AND reservation_id = $2",
            &[&self.identity, id],
        )
        .await
        .map_err(map_err)?;

        // Delete the reservation
        tx.execute(
            "DELETE FROM token_reservations WHERE user_id = $1 AND id = $2",
            &[&self.identity, id],
        )
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

        // Serialize against `set_tokens_outputs` so its `token_spent_outputs`
        // snapshot and the upsert that consumes it cannot interleave with this
        // transaction's spent-marker write.
        self.acquire_write_lock(&tx).await?;

        // Get reservation purpose and reserved output IDs
        let reservation_row = tx
            .query_opt(
                "SELECT purpose FROM token_reservations WHERE user_id = $1 AND id = $2",
                &[&self.identity, id],
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
                    "SELECT id FROM token_outputs WHERE user_id = $1 AND reservation_id = $2",
                    &[&self.identity, id],
                )
                .await
                .map_err(map_err)?;
            rows.iter().map(|r| r.get(0)).collect()
        };

        // Batch insert spent output markers
        if !reserved_output_ids.is_empty() {
            tx.execute(
                r"INSERT INTO token_spent_outputs (user_id, output_id)
                  SELECT $2, output_id FROM UNNEST($1::text[]) AS t(output_id)
                  ON CONFLICT DO NOTHING",
                &[&reserved_output_ids, &self.identity],
            )
            .await
            .map_err(map_err)?;
        }

        // Delete reserved outputs
        tx.execute(
            "DELETE FROM token_outputs WHERE user_id = $1 AND reservation_id = $2",
            &[&self.identity, id],
        )
        .await
        .map_err(map_err)?;

        // Delete the reservation
        tx.execute(
            "DELETE FROM token_reservations WHERE user_id = $1 AND id = $2",
            &[&self.identity, id],
        )
        .await
        .map_err(map_err)?;

        // If this was a swap reservation, update last_completed_at. UPSERT so a
        // tenant that joined after migration 2 (and thus has no row) gets one.
        if is_swap {
            tx.execute(
                "INSERT INTO token_swap_status (user_id, last_completed_at) \
                 VALUES ($1, NOW()) \
                 ON CONFLICT (user_id) DO UPDATE SET last_completed_at = EXCLUDED.last_completed_at",
                &[&self.identity],
            )
            .await
            .map_err(map_err)?;
        }

        // Clean up any orphaned metadata (per-tenant).
        tx.execute(
            r"DELETE FROM token_metadata
              WHERE user_id = $1
                AND identifier NOT IN (
                    SELECT DISTINCT token_identifier
                    FROM token_outputs WHERE user_id = $1
                )",
            &[&self.identity],
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
    /// Creates a new `PostgresTokenStore` from a configuration.
    ///
    /// This creates its own connection pool and runs token store migrations.
    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_config(
        config: PostgresStorageConfig,
        identity: &[u8],
    ) -> Result<Self, PostgresError> {
        let pool = create_pool(&config)?;
        Self::init(pool, identity).await
    }

    /// Creates a new `PostgresTokenStore` from an existing connection pool.
    ///
    /// This reuses the provided pool and runs token store migrations.
    /// Useful when sharing a pool with other components (e.g., `PostgresStorage`).
    pub async fn from_pool(pool: Pool, identity: &[u8]) -> Result<Self, PostgresError> {
        Self::init(pool, identity).await
    }

    /// Shared initialization logic for both constructors.
    async fn init(pool: Pool, identity: &[u8]) -> Result<Self, PostgresError> {
        let store = Self {
            pool,
            identity: identity.to_vec(),
            lock_objid: identity_lock_objid(identity),
        };
        store.migrate().await?;
        Ok(store)
    }

    /// Runs database migrations for token store tables.
    async fn migrate(&self) -> Result<(), PostgresError> {
        run_migrations(
            &self.pool,
            TOKEN_MIGRATIONS_TABLE,
            &Self::migrations(&self.identity),
        )
        .await
    }

    /// Returns the list of migrations for the token store.
    fn migrations(identity: &[u8]) -> Vec<Vec<String>> {
        vec![
            // Migration 1: Token store tables with race condition protection
            vec![
                "CREATE TABLE IF NOT EXISTS token_metadata (
                    identifier TEXT PRIMARY KEY,
                    issuer_public_key TEXT NOT NULL,
                    name TEXT NOT NULL,
                    ticker TEXT NOT NULL,
                    decimals INTEGER NOT NULL,
                    max_supply TEXT NOT NULL,
                    is_freezable BOOLEAN NOT NULL,
                    creation_entity_public_key TEXT
                )"
                .to_string(),
                "CREATE INDEX IF NOT EXISTS idx_token_metadata_issuer_pk
                    ON token_metadata (issuer_public_key)"
                    .to_string(),
                "CREATE TABLE IF NOT EXISTS token_reservations (
                    id TEXT PRIMARY KEY,
                    purpose TEXT NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )"
                .to_string(),
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
                )"
                .to_string(),
                "CREATE INDEX IF NOT EXISTS idx_token_outputs_identifier
                    ON token_outputs (token_identifier)"
                    .to_string(),
                "CREATE INDEX IF NOT EXISTS idx_token_outputs_reservation
                    ON token_outputs (reservation_id) WHERE reservation_id IS NOT NULL"
                    .to_string(),
                "CREATE TABLE IF NOT EXISTS token_spent_outputs (
                    output_id TEXT PRIMARY KEY,
                    spent_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )"
                .to_string(),
                "CREATE TABLE IF NOT EXISTS token_swap_status (
                    id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
                    last_completed_at TIMESTAMPTZ
                )"
                .to_string(),
                "INSERT INTO token_swap_status (id) VALUES (1) ON CONFLICT DO NOTHING".to_string(),
            ],
            // Migration 2: Multi-tenant scoping. Adds user_id to every token-store
            // table (including `token_metadata` — per-tenant to avoid 0-balance
            // leakage), backfills with the connecting tenant, and rewrites primary
            // keys / FKs / indexes to lead with user_id.
            token_store_multi_tenant_migration(identity),
        ]
    }

    /// Acquires an exclusive advisory lock for write operations.
    /// Per-tenant: combines `TOKEN_STORE_LOCK_CLASSID` with the tenant-specific
    /// `objid` so concurrent writes from different tenants do not block each
    /// other. Same-tenant writes still serialize on the same lock.
    async fn acquire_write_lock(
        &self,
        tx: &tokio_postgres::Transaction<'_>,
    ) -> Result<(), TokenOutputServiceError> {
        tx.execute(
            "SELECT pg_advisory_xact_lock($1, $2)",
            &[&TOKEN_STORE_LOCK_CLASSID, &self.lock_objid],
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }

    /// Inserts a single output into the database.
    #[allow(clippy::cast_possible_wrap)]
    async fn insert_single_output(
        &self,
        tx: &tokio_postgres::Transaction<'_>,
        token_identifier: &str,
        output: &TokenOutputWithPrevOut,
    ) -> Result<(), TokenOutputServiceError> {
        tx.execute(
            r"INSERT INTO token_outputs
                (user_id, id, token_identifier, owner_public_key, revocation_commitment,
                 withdraw_bond_sats, withdraw_relative_block_locktime,
                 token_public_key, token_amount, prev_tx_hash, prev_tx_vout, added_at)
              VALUES ($11, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
              ON CONFLICT (user_id, id) DO NOTHING",
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
                &self.identity,
            ],
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }

    /// Upserts token metadata.
    #[allow(clippy::cast_possible_wrap)]
    async fn upsert_metadata(
        &self,
        tx: &tokio_postgres::Transaction<'_>,
        metadata: &TokenMetadata,
    ) -> Result<(), TokenOutputServiceError> {
        tx.execute(
            r"INSERT INTO token_metadata
                (user_id, identifier, issuer_public_key, name, ticker, decimals, max_supply,
                 is_freezable, creation_entity_public_key)
              VALUES ($9, $1, $2, $3, $4, $5, $6, $7, $8)
              ON CONFLICT (user_id, identifier) DO UPDATE SET
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
                &self.identity,
            ],
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }

    /// Deletes reservations that have exceeded the timeout.
    /// Called during `set_tokens_outputs` to clean up stale reservations from
    /// crashed clients. Releases the outputs by clearing their `reservation_id`
    /// first, then deletes the parent reservations — the composite FK uses NO
    /// ACTION because column-list SET NULL is PG15+ and a whole-row SET NULL
    /// would null `user_id` (NOT NULL).
    async fn cleanup_stale_reservations(
        &self,
        tx: &tokio_postgres::Transaction<'_>,
    ) -> Result<u64, TokenOutputServiceError> {
        // Release outputs still pointing at any soon-to-be-deleted reservation.
        tx.execute(
            r"UPDATE token_outputs SET reservation_id = NULL
              WHERE user_id = $2
                AND reservation_id IN (
                    SELECT id FROM token_reservations
                    WHERE user_id = $2
                      AND created_at < NOW() - make_interval(secs => $1)
                )",
            &[&RESERVATION_TIMEOUT_SECS, &self.identity],
        )
        .await
        .map_err(map_err)?;

        let result = tx
            .execute(
                r"DELETE FROM token_reservations
                  WHERE user_id = $2
                    AND created_at < NOW() - make_interval(secs => $1)",
                &[&RESERVATION_TIMEOUT_SECS, &self.identity],
            )
            .await
            .map_err(map_err)?;

        if result > 0 {
            trace!("Cleaned up {} stale token reservations", result);
        }

        Ok(result)
    }

    /// Cleans up spent markers older than the cleanup threshold relative to refresh timestamp.
    async fn cleanup_spent_markers(
        &self,
        tx: &tokio_postgres::Transaction<'_>,
        refresh_timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), TokenOutputServiceError> {
        let threshold = chrono::Duration::milliseconds(SPENT_MARKER_CLEANUP_THRESHOLD_MS);
        let cleanup_cutoff = refresh_timestamp
            .checked_sub_signed(threshold)
            .unwrap_or(refresh_timestamp);

        tx.execute(
            "DELETE FROM token_spent_outputs WHERE user_id = $2 AND spent_at < $1",
            &[&cleanup_cutoff, &self.identity],
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

/// Creates a `PostgresTokenStore` instance from a configuration.
///
/// Creates its own connection pool. For sharing a pool, use
/// [`create_postgres_token_store_from_pool`] instead.
///
/// # Arguments
///
/// * `config` - Configuration for the `PostgreSQL` connection pool
/// * `identity` - 33-byte secp256k1 pubkey scoping all reads and writes
pub async fn create_postgres_token_store(
    config: PostgresStorageConfig,
    identity: &[u8],
) -> Result<Arc<dyn TokenOutputStore>, PostgresError> {
    Ok(Arc::new(
        PostgresTokenStore::from_config(config, identity).await?,
    ))
}

/// Creates a `PostgresTokenStore` instance from an existing connection pool.
///
/// Useful when sharing a pool with other components.
///
/// # Arguments
///
/// * `pool` - An existing deadpool-postgres connection pool
/// * `identity` - 33-byte secp256k1 pubkey scoping all reads and writes
pub async fn create_postgres_token_store_from_pool(
    pool: Pool,
    identity: &[u8],
) -> Result<Arc<dyn TokenOutputStore>, PostgresError> {
    Ok(Arc::new(
        PostgresTokenStore::from_pool(pool, identity).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::token_store_tests as shared_tests;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::postgres::Postgres;

    /// Fixed 33-byte test identity. Tests run in their own ephemeral container,
    /// so a single shared identity is fine — the schema still gets exercised.
    const TEST_IDENTITY: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];

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

            let store = PostgresTokenStore::from_config(
                PostgresStorageConfig::with_defaults(connection_string),
                &TEST_IDENTITY,
            )
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
    async fn test_get_token_balances_includes_zero_spendable() {
        let fixture = PostgresTokenStoreTestFixture::new().await;
        shared_tests::test_get_token_balances_includes_zero_spendable(&fixture.store).await;
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

    #[tokio::test]
    async fn test_stale_swap_reservation_does_not_block_set_tokens_outputs() {
        // Regression test mirroring the tree store fix: a stale Swap reservation
        // must be cleaned up before has_active_swap is evaluated, otherwise the
        // reservation pins itself in place and the local token-output set freezes.
        let fixture = PostgresTokenStoreTestFixture::new().await;
        let token1 = shared_tests::create_token_outputs(1, vec![100, 200]);
        fixture
            .store
            .set_tokens_outputs(
                std::slice::from_ref(&token1),
                shared_tests::future_refresh_start(),
            )
            .await
            .unwrap();

        let reservation = fixture
            .store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(100),
                TokenReservationPurpose::Swap,
                None,
                None,
            )
            .await
            .unwrap();

        let stored = fixture
            .store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert!(!stored.reserved_for_swap.is_empty());

        // Backdate the swap reservation past the 5-minute timeout.
        let client = fixture.store.pool.get().await.unwrap();
        client
            .execute(
                "UPDATE token_reservations SET created_at = NOW() - INTERVAL '10 minutes' WHERE id = $1",
                &[&reservation.id],
            )
            .await
            .unwrap();

        // set_tokens_outputs brings fresh data including a new output. Pre-fix:
        // skipped on has_active_swap, the new output is dropped and the reservation
        // lingers forever. Post-fix: cleanup runs first, the stale reservation is
        // dropped, has_active_swap is false, set_tokens_outputs applies.
        let token1_refresh = shared_tests::create_token_outputs(1, vec![100, 200, 300]);
        fixture
            .store
            .set_tokens_outputs(
                std::slice::from_ref(&token1_refresh),
                shared_tests::future_refresh_start(),
            )
            .await
            .unwrap();

        let stored = fixture
            .store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert!(
            stored.reserved_for_swap.is_empty(),
            "Stale swap reservation should be cleaned up"
        );
        assert_eq!(
            stored.available.len(),
            3,
            "set_tokens_outputs should have proceeded and applied the operator's view (3 outputs)"
        );
        assert!(
            stored
                .available
                .iter()
                .any(|o| o.output.token_amount == 300),
            "the 300-amount output from the refresh should be present"
        );
    }

    #[tokio::test]
    async fn test_finalize_reservation_blocked_by_write_lock() {
        // Regression: `finalize_reservation` must acquire the same advisory
        // lock as `set_tokens_outputs` so they serialize. Otherwise a
        // concurrent set_tokens_outputs could read the spent_outputs snapshot
        // before our marker commits and re-insert the just-spent output as
        // Available.
        let fixture = PostgresTokenStoreTestFixture::new().await;

        let token_outputs = shared_tests::create_token_outputs(1, vec![100, 200]);
        fixture
            .store
            .set_tokens_outputs(&[token_outputs], shared_tests::future_refresh_start())
            .await
            .unwrap();
        let reservation = fixture
            .store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(100),
                TokenReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Hold the token-store write lock on a separate connection. Must use
        // the same (classid, objid) pair as `acquire_write_lock`.
        let lock_objid = fixture.store.lock_objid;
        let mut holder = fixture.store.pool.get().await.unwrap();
        let holder_tx = holder.transaction().await.unwrap();
        holder_tx
            .execute(
                "SELECT pg_advisory_xact_lock($1, $2)",
                &[&TOKEN_STORE_LOCK_CLASSID, &lock_objid],
            )
            .await
            .unwrap();

        let store = Arc::new(fixture.store);
        let store_for_task = store.clone();
        let res_id = reservation.id.clone();
        let finalize_task =
            tokio::spawn(async move { store_for_task.finalize_reservation(&res_id).await });

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            !finalize_task.is_finished(),
            "finalize_reservation completed while advisory lock was held — \
             the lock is not being acquired"
        );

        holder_tx.commit().await.unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), finalize_task)
            .await
            .expect("finalize_reservation did not complete after lock released")
            .unwrap()
            .unwrap();
    }
}
