//! `MySQL`-backed implementation of the `TokenOutputStore` trait.
//!
//! Direct port of `crates/spark-postgres/src/token_store.rs`. See `tree_store.rs`
//! for the SQL translation rules used here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, NaiveDateTime, Utc};
use macros::async_trait;
use mysql_async::prelude::*;
use mysql_async::{Conn, Params, Pool, Row, TxOpts, Value};
use platform_utils::time::SystemTime;
use spark_wallet::{
    GetTokenOutputsFilter, ReservationTarget, SelectionStrategy, TokenMetadata, TokenOutput,
    TokenOutputServiceError, TokenOutputStore, TokenOutputWithPrevOut, TokenOutputs,
    TokenOutputsPerStatus, TokenOutputsReservation, TokenOutputsReservationId,
    TokenReservationPurpose,
};
use tracing::{trace, warn};
use uuid::Uuid;

use crate::config::MysqlStorageConfig;
use crate::error::MysqlError;
use crate::migrations::run_migrations;
use crate::pool::create_pool;

const TOKEN_MIGRATIONS_TABLE: &str = "token_schema_migrations";

const TOKEN_STORE_WRITE_LOCK_NAME: &str = "token_store_write_lock";
const WRITE_LOCK_TIMEOUT_SECS: i64 = 30;

const SPENT_MARKER_CLEANUP_THRESHOLD_MS: i64 = 5 * 60 * 1000;
const RESERVATION_TIMEOUT_SECS: i64 = 300;

/// `MySQL`-backed token output store implementation.
pub struct MysqlTokenStore {
    pool: Pool,
}

#[async_trait]
impl TokenOutputStore for MysqlTokenStore {
    #[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
    async fn set_tokens_outputs(
        &self,
        token_outputs: &[TokenOutputs],
        refresh_started_at: SystemTime,
    ) -> Result<(), TokenOutputServiceError> {
        let refresh_timestamp: DateTime<Utc> = refresh_started_at.into();

        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        Self::acquire_write_lock(&mut conn).await?;
        let result =
            Self::set_tokens_outputs_inner(&mut conn, token_outputs, refresh_timestamp).await;
        Self::release_write_lock_quiet(&mut conn).await;
        result
    }

    async fn get_token_balances(
        &self,
    ) -> Result<Vec<(TokenMetadata, u128)>, TokenOutputServiceError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        // Server-side aggregate: spendable (available + swap-reserved) per
        // token. Matches the in-memory default impl which returns all tokens
        // that have at least one output (including zero spendable balance).
        // `token_amount` is stored as VARCHAR — cast to DECIMAL(65,0) so the
        // SUM works across full u128 range, then return as TEXT for parsing.
        let rows: Vec<Row> = conn
            .query(
                r"SELECT m.identifier, m.issuer_public_key, m.name, m.ticker, m.decimals,
                         m.max_supply, m.is_freezable, m.creation_entity_public_key,
                         CAST(COALESCE(SUM(
                            CASE
                              WHEN o.reservation_id IS NULL THEN CAST(o.token_amount AS DECIMAL(65,0))
                              WHEN r.purpose = 'Swap' THEN CAST(o.token_amount AS DECIMAL(65,0))
                              ELSE 0
                            END
                         ), 0) AS CHAR) AS balance
                  FROM token_metadata m
                  JOIN token_outputs o ON o.token_identifier = m.identifier
                  LEFT JOIN token_reservations r ON o.reservation_id = r.id
                  GROUP BY m.identifier, m.issuer_public_key, m.name, m.ticker,
                           m.decimals, m.max_supply, m.is_freezable, m.creation_entity_public_key",
            )
            .await
            .map_err(map_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let metadata = Self::metadata_from_row(&row)?;
            let balance_str: String = row.get("balance").ok_or_else(missing_col)?;
            let balance: u128 = balance_str.parse().map_err(map_err)?;
            out.push((metadata, balance));
        }
        Ok(out)
    }

    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;

        let rows: Vec<Row> = conn
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
                  ORDER BY m.identifier, CAST(o.token_amount AS DECIMAL(65,0)) ASC",
            )
            .await
            .map_err(map_err)?;

        let mut map: HashMap<String, TokenOutputsPerStatus> = HashMap::new();

        for row in rows {
            let identifier: String = get_str_required(&row, "identifier")?;
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

            // `Option<Option<String>>`: outer = column missing, inner = NULL.
            // Both flatten to "no output for this row" (LEFT JOIN miss).
            let output_id: Option<String> =
                row.get::<Option<String>, _>("output_id").and_then(|v| v);
            if output_id.is_none() {
                continue;
            }

            let output = Self::output_from_row(&row)?;
            let purpose: Option<String> = row.get::<Option<String>, _>("purpose").and_then(|v| v);

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
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;

        let (where_clause, param): (&str, String) = match filter {
            GetTokenOutputsFilter::Identifier(id) => ("m.identifier = ?", id.to_string()),
            GetTokenOutputsFilter::IssuerPublicKey(pk) => {
                ("m.issuer_public_key = ?", pk.to_string())
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
              ORDER BY CAST(o.token_amount AS DECIMAL(65,0)) ASC"
        );

        let rows: Vec<Row> = conn.exec(&query, (param,)).await.map_err(map_err)?;

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
            let output_id: Option<String> =
                row.get::<Option<String>, _>("output_id").and_then(|v| v);
            if output_id.is_none() {
                continue;
            }

            let output = Self::output_from_row(row)?;
            let purpose: Option<String> = row.get::<Option<String>, _>("purpose").and_then(|v| v);

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
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        Self::upsert_metadata(&mut tx, &token_outputs.metadata).await?;

        let output_ids: Vec<String> = token_outputs
            .outputs
            .iter()
            .map(|o| o.output.id.clone())
            .collect();
        if !output_ids.is_empty() {
            let placeholders = build_placeholders(output_ids.len());
            let sql =
                format!("DELETE FROM token_spent_outputs WHERE output_id IN ({placeholders})");
            let params: Vec<Value> = output_ids.iter().cloned().map(Value::from).collect();
            tx.exec_drop(&sql, Params::Positional(params))
                .await
                .map_err(map_err)?;
        }

        for output in &token_outputs.outputs {
            Self::insert_single_output(&mut tx, &token_outputs.metadata.identifier, output).await?;
        }

        tx.commit().await.map_err(map_err)?;

        trace!(
            "Inserted {} token outputs into MySQL",
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

        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        Self::acquire_write_lock(&mut conn).await?;
        let result = Self::reserve_token_outputs_inner(
            &mut conn,
            token_identifier,
            target,
            purpose,
            preferred_outputs,
            selection_strategy,
        )
        .await;
        Self::release_write_lock_quiet(&mut conn).await;
        result
    }

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        // Scoped to a single `reservation_id`; row-level FK + MVCC suffice.
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        Self::cancel_reservation_inner(&mut conn, id).await?;
        trace!("Canceled token outputs reservation: {}", id);
        Ok(())
    }

    async fn finalize_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        // Serialize against `set_tokens_outputs` so its `token_spent_outputs`
        // snapshot and the upsert that consumes it cannot interleave with this
        // transaction's spent-marker write.
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        Self::acquire_write_lock(&mut conn).await?;
        let result = Self::finalize_reservation_inner(&mut conn, id).await;
        Self::release_write_lock_quiet(&mut conn).await;
        result?;
        trace!("Finalized token outputs reservation: {}", id);
        Ok(())
    }

    async fn now(&self) -> Result<SystemTime, TokenOutputServiceError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        let row: Option<NaiveDateTime> =
            conn.query_first("SELECT NOW(6)").await.map_err(map_err)?;
        let now =
            row.ok_or_else(|| TokenOutputServiceError::Generic("NOW() returned no row".into()))?;
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(now, Utc);
        Ok(dt.into())
    }
}

impl MysqlTokenStore {
    pub async fn from_config(config: MysqlStorageConfig) -> Result<Self, MysqlError> {
        let pool = create_pool(&config)?;
        Self::init(pool).await
    }

    pub async fn from_pool(pool: Pool) -> Result<Self, MysqlError> {
        Self::init(pool).await
    }

    async fn init(pool: Pool) -> Result<Self, MysqlError> {
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), MysqlError> {
        run_migrations(&self.pool, TOKEN_MIGRATIONS_TABLE, &Self::migrations()).await
    }

    fn migrations() -> Vec<&'static [&'static str]> {
        vec![&[
            "CREATE TABLE IF NOT EXISTS token_metadata (
                identifier VARCHAR(255) NOT NULL PRIMARY KEY,
                issuer_public_key VARCHAR(255) NOT NULL,
                name VARCHAR(255) NOT NULL,
                ticker VARCHAR(64) NOT NULL,
                decimals INT NOT NULL,
                max_supply VARCHAR(128) NOT NULL,
                is_freezable TINYINT(1) NOT NULL,
                creation_entity_public_key VARCHAR(255) NULL
            )",
            "CREATE INDEX idx_token_metadata_issuer_pk ON token_metadata (issuer_public_key)",
            "CREATE TABLE IF NOT EXISTS token_reservations (
                id VARCHAR(255) NOT NULL PRIMARY KEY,
                purpose VARCHAR(64) NOT NULL,
                created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
            )",
            "CREATE TABLE IF NOT EXISTS token_outputs (
                id VARCHAR(255) NOT NULL PRIMARY KEY,
                token_identifier VARCHAR(255) NOT NULL,
                owner_public_key VARCHAR(255) NOT NULL,
                revocation_commitment VARCHAR(255) NOT NULL,
                withdraw_bond_sats BIGINT NOT NULL,
                withdraw_relative_block_locktime BIGINT NOT NULL,
                token_public_key VARCHAR(255) NULL,
                token_amount VARCHAR(128) NOT NULL,
                prev_tx_hash VARCHAR(255) NOT NULL,
                prev_tx_vout INT NOT NULL,
                reservation_id VARCHAR(255) NULL,
                added_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
                CONSTRAINT fk_token_outputs_metadata FOREIGN KEY (token_identifier)
                    REFERENCES token_metadata(identifier),
                CONSTRAINT fk_token_outputs_reservation FOREIGN KEY (reservation_id)
                    REFERENCES token_reservations(id) ON DELETE SET NULL
            )",
            "CREATE INDEX idx_token_outputs_identifier ON token_outputs (token_identifier)",
            "CREATE INDEX idx_token_outputs_reservation ON token_outputs (reservation_id)",
            "CREATE TABLE IF NOT EXISTS token_spent_outputs (
                output_id VARCHAR(255) NOT NULL PRIMARY KEY,
                spent_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
            )",
            "CREATE TABLE IF NOT EXISTS token_swap_status (
                id INT NOT NULL PRIMARY KEY DEFAULT 1,
                last_completed_at DATETIME(6) NULL,
                CHECK (id = 1)
            )",
            "INSERT IGNORE INTO token_swap_status (id) VALUES (1)",
        ]]
    }

    async fn acquire_write_lock(conn: &mut Conn) -> Result<(), TokenOutputServiceError> {
        let acquired: Option<i64> = conn
            .exec_first(
                "SELECT GET_LOCK(?, ?)",
                (TOKEN_STORE_WRITE_LOCK_NAME, WRITE_LOCK_TIMEOUT_SECS),
            )
            .await
            .map_err(map_err)?;
        if acquired != Some(1) {
            return Err(TokenOutputServiceError::Generic(format!(
                "Failed to acquire token store write lock within {WRITE_LOCK_TIMEOUT_SECS}s"
            )));
        }
        Ok(())
    }

    async fn release_write_lock_quiet(conn: &mut Conn) {
        let _ = conn
            .exec_drop("SELECT RELEASE_LOCK(?)", (TOKEN_STORE_WRITE_LOCK_NAME,))
            .await;
    }

    #[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
    async fn set_tokens_outputs_inner(
        conn: &mut Conn,
        token_outputs: &[TokenOutputs],
        refresh_timestamp: DateTime<Utc>,
    ) -> Result<(), TokenOutputServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        Self::cleanup_stale_reservations(&mut tx).await?;

        let row: Option<(i64, i64)> = tx
            .exec_first(
                r"SELECT
                    (SELECT EXISTS(SELECT 1 FROM token_reservations WHERE purpose = 'Swap')) AS has_active_swap,
                    COALESCE(
                        (SELECT (last_completed_at >= ?) FROM token_swap_status WHERE id = 1),
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
            trace!(
                "Skipping set_tokens_outputs: active_swap={}, swap_completed_during_refresh={}",
                has_active_swap, swap_completed_during_refresh
            );
            tx.commit().await.map_err(map_err)?;
            return Ok(());
        }

        Self::cleanup_spent_markers(&mut tx, refresh_timestamp).await?;

        let spent_rows: Vec<String> = tx
            .exec(
                "SELECT output_id FROM token_spent_outputs WHERE spent_at >= ?",
                (refresh_timestamp.naive_utc(),),
            )
            .await
            .map_err(map_err)?;
        let spent_ids: HashSet<String> = spent_rows.into_iter().collect();

        tx.exec_drop(
            "DELETE FROM token_outputs WHERE reservation_id IS NULL AND added_at < ?",
            (refresh_timestamp.naive_utc(),),
        )
        .await
        .map_err(map_err)?;

        let incoming_output_ids: HashSet<String> = token_outputs
            .iter()
            .flat_map(|to| to.outputs.iter().map(|o| o.output.id.clone()))
            .collect();

        let reserved_pairs: Vec<(String, String)> = tx
            .query(
                r"SELECT r.id, o.id
                  FROM token_reservations r
                  JOIN token_outputs o ON o.reservation_id = r.id",
            )
            .await
            .map_err(map_err)?;

        let mut reservation_outputs: HashMap<String, Vec<String>> = HashMap::new();
        for (reservation_id, output_id) in reserved_pairs {
            reservation_outputs
                .entry(reservation_id)
                .or_default()
                .push(output_id);
        }

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
                for id in output_ids {
                    if !incoming_output_ids.contains(id) {
                        outputs_to_remove_from_reservation.push(id.clone());
                    }
                }
            }
        }

        if !reservations_to_delete.is_empty() {
            let placeholders = build_placeholders(reservations_to_delete.len());
            let outputs_sql =
                format!("DELETE FROM token_outputs WHERE reservation_id IN ({placeholders})");
            let outputs_params: Vec<Value> = reservations_to_delete
                .iter()
                .cloned()
                .map(Value::from)
                .collect();
            tx.exec_drop(&outputs_sql, Params::Positional(outputs_params))
                .await
                .map_err(map_err)?;

            let res_sql = format!("DELETE FROM token_reservations WHERE id IN ({placeholders})");
            let res_params: Vec<Value> = reservations_to_delete
                .iter()
                .cloned()
                .map(Value::from)
                .collect();
            tx.exec_drop(&res_sql, Params::Positional(res_params))
                .await
                .map_err(map_err)?;
        }

        if !outputs_to_remove_from_reservation.is_empty() {
            let placeholders = build_placeholders(outputs_to_remove_from_reservation.len());
            let sql = format!("DELETE FROM token_outputs WHERE id IN ({placeholders})");
            let params: Vec<Value> = outputs_to_remove_from_reservation
                .iter()
                .cloned()
                .map(Value::from)
                .collect();
            tx.exec_drop(&sql, Params::Positional(params))
                .await
                .map_err(map_err)?;

            let empty_ids: Vec<String> = tx
                .query(
                    r"SELECT r.id FROM token_reservations r
                      LEFT JOIN token_outputs o ON o.reservation_id = r.id
                      WHERE o.id IS NULL",
                )
                .await
                .map_err(map_err)?;
            if !empty_ids.is_empty() {
                let placeholders = build_placeholders(empty_ids.len());
                let sql = format!("DELETE FROM token_reservations WHERE id IN ({placeholders})");
                let params: Vec<Value> = empty_ids.iter().cloned().map(Value::from).collect();
                tx.exec_drop(&sql, Params::Positional(params))
                    .await
                    .map_err(map_err)?;
            }
        }

        let reserved_output_ids: HashSet<String> = tx
            .query::<String, _>("SELECT id FROM token_outputs WHERE reservation_id IS NOT NULL")
            .await
            .map_err(map_err)?
            .into_iter()
            .collect();

        tx.query_drop(
            r"DELETE FROM token_metadata
              WHERE identifier NOT IN (
                  SELECT DISTINCT token_identifier FROM token_outputs
              )",
        )
        .await
        .map_err(map_err)?;

        for to in token_outputs {
            Self::upsert_metadata(&mut tx, &to.metadata).await?;

            for output in &to.outputs {
                if reserved_output_ids.contains(&output.output.id)
                    || spent_ids.contains(&output.output.id)
                {
                    continue;
                }
                Self::insert_single_output(&mut tx, &to.metadata.identifier, output).await?;
            }
        }

        tx.commit().await.map_err(map_err)?;

        trace!("Updated {} token outputs in MySQL", token_outputs.len());
        Ok(())
    }

    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    async fn reserve_token_outputs_inner(
        conn: &mut Conn,
        token_identifier: &str,
        target: ReservationTarget,
        purpose: TokenReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let metadata_row: Option<Row> = tx
            .exec_first(
                "SELECT * FROM token_metadata WHERE identifier = ?",
                (token_identifier,),
            )
            .await
            .map_err(map_err)?;
        let metadata_row = metadata_row.ok_or_else(|| {
            TokenOutputServiceError::Generic(format!(
                "Token outputs not found for identifier: {token_identifier}"
            ))
        })?;
        let metadata = Self::metadata_from_row(&metadata_row)?;

        let rows: Vec<Row> = tx
            .exec(
                r"SELECT o.id AS output_id, o.owner_public_key, o.revocation_commitment,
                         o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                         o.token_public_key, o.token_amount, o.prev_tx_hash, o.prev_tx_vout,
                         o.token_identifier
                  FROM token_outputs o
                  WHERE o.token_identifier = ? AND o.reservation_id IS NULL",
                (token_identifier,),
            )
            .await
            .map_err(map_err)?;

        let mut outputs: Vec<TokenOutputWithPrevOut> = rows
            .iter()
            .map(Self::output_from_row)
            .collect::<Result<Vec<_>, _>>()?;

        if let Some(ref preferred) = preferred_outputs {
            let preferred_ids: HashSet<&str> =
                preferred.iter().map(|p| p.output.id.as_str()).collect();
            outputs.retain(|o| preferred_ids.contains(o.output.id.as_str()));
        }

        if let ReservationTarget::MinTotalValue(amount) = target
            && outputs.iter().map(|o| o.output.token_amount).sum::<u128>() < amount
        {
            return Err(TokenOutputServiceError::InsufficientFunds);
        }

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

        let reservation_id = Uuid::now_v7().to_string();
        let purpose_str = match purpose {
            TokenReservationPurpose::Payment => "Payment",
            TokenReservationPurpose::Swap => "Swap",
        };

        tx.exec_drop(
            "INSERT INTO token_reservations (id, purpose) VALUES (?, ?)",
            (&reservation_id, purpose_str),
        )
        .await
        .map_err(map_err)?;

        let selected_ids: Vec<String> = selected_outputs
            .iter()
            .map(|o| o.output.id.clone())
            .collect();
        if !selected_ids.is_empty() {
            let placeholders = build_placeholders(selected_ids.len());
            let sql =
                format!("UPDATE token_outputs SET reservation_id = ? WHERE id IN ({placeholders})");
            let mut params: Vec<Value> = Vec::with_capacity(selected_ids.len() + 1);
            params.push(Value::from(reservation_id.clone()));
            for id in &selected_ids {
                params.push(Value::from(id.clone()));
            }
            tx.exec_drop(&sql, Params::Positional(params))
                .await
                .map_err(map_err)?;
        }

        tx.commit().await.map_err(map_err)?;

        Ok(TokenOutputsReservation::new(
            reservation_id,
            TokenOutputs {
                metadata,
                outputs: selected_outputs,
            },
        ))
    }

    async fn cancel_reservation_inner(
        conn: &mut Conn,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        tx.exec_drop(
            "UPDATE token_outputs SET reservation_id = NULL WHERE reservation_id = ?",
            (id,),
        )
        .await
        .map_err(map_err)?;

        tx.exec_drop("DELETE FROM token_reservations WHERE id = ?", (id,))
            .await
            .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn finalize_reservation_inner(
        conn: &mut Conn,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let purpose: Option<String> = tx
            .exec_first("SELECT purpose FROM token_reservations WHERE id = ?", (id,))
            .await
            .map_err(map_err)?;

        let Some(purpose) = purpose else {
            warn!("Tried to finalize a non existing reservation");
            tx.commit().await.map_err(map_err)?;
            return Ok(());
        };

        let is_swap = purpose == "Swap";

        let reserved_output_ids: Vec<String> = tx
            .exec(
                "SELECT id FROM token_outputs WHERE reservation_id = ?",
                (id,),
            )
            .await
            .map_err(map_err)?;

        if !reserved_output_ids.is_empty() {
            let mut sql =
                String::from("INSERT IGNORE INTO token_spent_outputs (output_id) VALUES ");
            let mut params: Vec<Value> = Vec::with_capacity(reserved_output_ids.len());
            for (i, oid) in reserved_output_ids.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push_str("(?)");
                params.push(Value::from(oid.clone()));
            }
            tx.exec_drop(&sql, Params::Positional(params))
                .await
                .map_err(map_err)?;
        }

        tx.exec_drop("DELETE FROM token_outputs WHERE reservation_id = ?", (id,))
            .await
            .map_err(map_err)?;

        tx.exec_drop("DELETE FROM token_reservations WHERE id = ?", (id,))
            .await
            .map_err(map_err)?;

        if is_swap {
            tx.query_drop("UPDATE token_swap_status SET last_completed_at = NOW(6) WHERE id = 1")
                .await
                .map_err(map_err)?;
        }

        tx.query_drop(
            r"DELETE FROM token_metadata
              WHERE identifier NOT IN (
                  SELECT DISTINCT token_identifier FROM token_outputs
              )",
        )
        .await
        .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    #[allow(clippy::cast_possible_wrap)]
    async fn insert_single_output(
        tx: &mut mysql_async::Transaction<'_>,
        token_identifier: &str,
        output: &TokenOutputWithPrevOut,
    ) -> Result<(), TokenOutputServiceError> {
        tx.exec_drop(
            r"INSERT IGNORE INTO token_outputs
                (id, token_identifier, owner_public_key, revocation_commitment,
                 withdraw_bond_sats, withdraw_relative_block_locktime,
                 token_public_key, token_amount, prev_tx_hash, prev_tx_vout, added_at)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NOW(6))",
            (
                &output.output.id,
                token_identifier,
                output.output.owner_public_key.to_string(),
                &output.output.revocation_commitment,
                output.output.withdraw_bond_sats as i64,
                output.output.withdraw_relative_block_locktime as i64,
                output.output.token_public_key.map(|pk| pk.to_string()),
                output.output.token_amount.to_string(),
                &output.prev_tx_hash,
                output.prev_tx_vout as i32,
            ),
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }

    #[allow(clippy::cast_possible_wrap)]
    async fn upsert_metadata(
        tx: &mut mysql_async::Transaction<'_>,
        metadata: &TokenMetadata,
    ) -> Result<(), TokenOutputServiceError> {
        tx.exec_drop(
            r"INSERT INTO token_metadata
                (identifier, issuer_public_key, name, ticker, decimals, max_supply,
                 is_freezable, creation_entity_public_key)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?)
              ON DUPLICATE KEY UPDATE
                issuer_public_key = VALUES(issuer_public_key),
                name = VALUES(name),
                ticker = VALUES(ticker),
                decimals = VALUES(decimals),
                max_supply = VALUES(max_supply),
                is_freezable = VALUES(is_freezable),
                creation_entity_public_key = VALUES(creation_entity_public_key)",
            (
                &metadata.identifier,
                metadata.issuer_public_key.to_string(),
                &metadata.name,
                &metadata.ticker,
                metadata.decimals as i32,
                metadata.max_supply.to_string(),
                metadata.is_freezable,
                metadata.creation_entity_public_key.map(|pk| pk.to_string()),
            ),
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn cleanup_stale_reservations(
        tx: &mut mysql_async::Transaction<'_>,
    ) -> Result<u64, TokenOutputServiceError> {
        let mut result = tx
            .exec_iter(
                "DELETE FROM token_reservations
                 WHERE created_at < DATE_SUB(NOW(6), INTERVAL ? SECOND)",
                (RESERVATION_TIMEOUT_SECS,),
            )
            .await
            .map_err(map_err)?;
        let affected = result.affected_rows();
        let _: Vec<mysql_async::Row> = result.collect().await.map_err(map_err)?;

        if affected > 0 {
            trace!("Cleaned up {} stale token reservations", affected);
        }
        Ok(affected)
    }

    async fn cleanup_spent_markers(
        tx: &mut mysql_async::Transaction<'_>,
        refresh_timestamp: DateTime<Utc>,
    ) -> Result<(), TokenOutputServiceError> {
        let threshold = chrono::Duration::milliseconds(SPENT_MARKER_CLEANUP_THRESHOLD_MS);
        let cleanup_cutoff = refresh_timestamp
            .checked_sub_signed(threshold)
            .unwrap_or(refresh_timestamp);

        tx.exec_drop(
            "DELETE FROM token_spent_outputs WHERE spent_at < ?",
            (cleanup_cutoff.naive_utc(),),
        )
        .await
        .map_err(map_err)?;

        Ok(())
    }

    #[allow(clippy::cast_sign_loss)]
    fn metadata_from_row(row: &Row) -> Result<TokenMetadata, TokenOutputServiceError> {
        // Use Option<T> for every read to avoid panics on NULL — `row.get::<T, _>`
        // with non-Option `T` panics on NULL during FromValue conversion. NOT NULL
        // schema constraints already enforce this is rare, but a `(`Null`)` panic
        // crashes the whole connection's listening loop instead of returning a
        // typed error to the caller.
        let identifier: String = get_str_required(row, "identifier")?;
        let issuer_pk_str: String = get_str_required(row, "issuer_public_key")?;
        let name: String = get_str_required(row, "name")?;
        let ticker: String = get_str_required(row, "ticker")?;
        let decimals: i32 = row
            .get::<Option<i32>, _>("decimals")
            .ok_or_else(missing_col)?
            .ok_or_else(|| {
                TokenOutputServiceError::Generic("decimals column is NULL".to_string())
            })?;
        let max_supply_str: String = get_str_required(row, "max_supply")?;
        let is_freezable: bool = row
            .get::<Option<bool>, _>("is_freezable")
            .ok_or_else(missing_col)?
            .ok_or_else(|| {
                TokenOutputServiceError::Generic("is_freezable column is NULL".to_string())
            })?;
        let creation_entity_pk_str: Option<String> = row
            .get::<Option<String>, _>("creation_entity_public_key")
            .unwrap_or(None);

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

    #[allow(clippy::cast_sign_loss)]
    fn output_from_row(row: &Row) -> Result<TokenOutputWithPrevOut, TokenOutputServiceError> {
        // See `metadata_from_row` for why every column is read via `Option<T>`
        // first — `mysql_async` panics on NULL for non-Option `T`.
        let output_id: String = get_str_required(row, "output_id")?;
        let owner_pk_str: String = get_str_required(row, "owner_public_key")?;
        let revocation_commitment: String = get_str_required(row, "revocation_commitment")?;
        let withdraw_bond_sats: i64 = row
            .get::<Option<i64>, _>("withdraw_bond_sats")
            .ok_or_else(missing_col)?
            .ok_or_else(|| {
                TokenOutputServiceError::Generic("withdraw_bond_sats column is NULL".to_string())
            })?;
        let withdraw_relative_block_locktime: i64 = row
            .get::<Option<i64>, _>("withdraw_relative_block_locktime")
            .ok_or_else(missing_col)?
            .ok_or_else(|| {
                TokenOutputServiceError::Generic(
                    "withdraw_relative_block_locktime column is NULL".to_string(),
                )
            })?;
        let token_pk_str: Option<String> = row
            .get::<Option<String>, _>("token_public_key")
            .unwrap_or(None);
        let token_amount_str: String = get_str_required(row, "token_amount")?;
        let prev_tx_hash: String = get_str_required(row, "prev_tx_hash")?;
        let prev_tx_vout: i32 = row
            .get::<Option<i32>, _>("prev_tx_vout")
            .ok_or_else(missing_col)?
            .ok_or_else(|| {
                TokenOutputServiceError::Generic("prev_tx_vout column is NULL".to_string())
            })?;

        let token_identifier: String = row
            .get::<Option<String>, _>("token_identifier")
            .and_then(|v| v) // Some(Some(s)) | Some(None) → Option<String>
            .or_else(|| row.get::<Option<String>, _>("identifier").and_then(|v| v))
            .ok_or_else(missing_col)?;

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

/// Reads a column that the schema declares NOT NULL as an `Option<String>`
/// first to avoid `mysql_async`'s panic-on-NULL behavior in `FromValue` for
/// non-`Option` types, then surfaces both "column missing" and "column NULL"
/// as `TokenOutputServiceError::Generic`. Use this for any `String` column
/// in row-helper code, even when the schema says NOT NULL — a buggy
/// migration or a CTE that exposes the same column name on multiple sides
/// of a JOIN can otherwise crash the connection.
fn get_str_required(row: &Row, col: &str) -> Result<String, TokenOutputServiceError> {
    row.get::<Option<String>, _>(col)
        .ok_or_else(missing_col)?
        .ok_or_else(|| TokenOutputServiceError::Generic(format!("{col} column is NULL")))
}

fn missing_col() -> TokenOutputServiceError {
    TokenOutputServiceError::Generic("missing column in query result".to_string())
}

fn map_err<E: std::fmt::Display>(e: E) -> TokenOutputServiceError {
    TokenOutputServiceError::Generic(e.to_string())
}

pub async fn create_mysql_token_store(
    config: MysqlStorageConfig,
) -> Result<Arc<dyn TokenOutputStore>, MysqlError> {
    Ok(Arc::new(MysqlTokenStore::from_config(config).await?))
}

pub async fn create_mysql_token_store_from_pool(
    pool: Pool,
) -> Result<Arc<dyn TokenOutputStore>, MysqlError> {
    Ok(Arc::new(MysqlTokenStore::from_pool(pool).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::token_store_tests as shared_tests;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::mysql::Mysql;

    struct MysqlTokenStoreTestFixture {
        store: MysqlTokenStore,
        #[allow(dead_code)]
        container: ContainerAsync<Mysql>,
    }

    impl MysqlTokenStoreTestFixture {
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

            let store =
                MysqlTokenStore::from_config(MysqlStorageConfig::with_defaults(connection_string))
                    .await
                    .expect("Failed to create MysqlTokenStore");

            Self { store, container }
        }
    }

    #[tokio::test]
    async fn test_set_tokens_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_set_tokens_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_token_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_get_token_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_insert_token_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_insert_token_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_and_finalize() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_and_finalize(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_swap_marks_spent_and_tracks_completion() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_finalize_swap_marks_spent_and_tracks_completion(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_token_balances_includes_zero_spendable() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_get_token_balances_includes_zero_spendable(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_reservation_blocked_by_write_lock() {
        // Regression: `finalize_reservation` must acquire the same named lock
        // as `set_tokens_outputs` so they serialize. Otherwise a concurrent
        // set_tokens_outputs could read the spent_outputs snapshot before our
        // marker commits and re-insert the just-spent output as Available.
        let fixture = MysqlTokenStoreTestFixture::new().await;

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

        // Hold the named lock on a separate connection.
        let mut holder = fixture.store.pool.get_conn().await.unwrap();
        let acquired: Option<i64> = holder
            .exec_first(
                "SELECT GET_LOCK(?, ?)",
                (TOKEN_STORE_WRITE_LOCK_NAME, WRITE_LOCK_TIMEOUT_SECS),
            )
            .await
            .unwrap();
        assert_eq!(acquired, Some(1), "holder failed to acquire the lock");

        let store = Arc::new(fixture.store);
        let store_for_task = store.clone();
        let res_id = reservation.id.clone();
        let finalize_task =
            tokio::spawn(async move { store_for_task.finalize_reservation(&res_id).await });

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            !finalize_task.is_finished(),
            "finalize_reservation completed while named lock was held — \
             the lock is not being acquired"
        );

        holder
            .exec_drop("SELECT RELEASE_LOCK(?)", (TOKEN_STORE_WRITE_LOCK_NAME,))
            .await
            .unwrap();
        drop(holder);

        tokio::time::timeout(std::time::Duration::from_secs(5), finalize_task)
            .await
            .expect("finalize_reservation did not complete after lock released")
            .unwrap()
            .unwrap();
    }
}
