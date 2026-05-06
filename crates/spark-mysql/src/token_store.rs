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

use crate::advisory_lock::identity_lock_name;
use crate::config::MysqlStorageConfig;
use crate::error::MysqlError;
use crate::migrations::{Migration, run_migrations};
use crate::pool::create_pool;

const TOKEN_MIGRATIONS_TABLE: &str = "token_schema_migrations";

/// Domain prefix mixed into the per-tenant `GET_LOCK` name so the token store's
/// locks never collide with the tree store's, even when two tenants share a
/// database.
const TOKEN_STORE_LOCK_PREFIX: &str = "breez-spark-sdk:token:";
const WRITE_LOCK_TIMEOUT_SECS: i64 = 30;

const SPENT_MARKER_CLEANUP_THRESHOLD_MS: i64 = 5 * 60 * 1000;
const RESERVATION_TIMEOUT_SECS: i64 = 300;

/// `MySQL`-backed token output store implementation.
///
/// Each instance is scoped to a single tenant identity so multiple tenants
/// can share one `MySQL` database without cross-pollinating token state.
pub struct MysqlTokenStore {
    pool: Pool,
    /// 33-byte secp256k1 compressed pubkey identifying this tenant. All reads
    /// and writes are filtered by `user_id = self.identity`.
    identity: Vec<u8>,
    /// Stable per-tenant `GET_LOCK` name derived from `identity`.
    lock_name: String,
}

/// Builds the multi-tenant scoping migration for the token store. Adds
/// `user_id VARBINARY(33)` to every per-user table (including
/// `token_metadata` — metadata is per-tenant to avoid 0-balance leakage for
/// tokens a tenant never owned), backfills with the connecting tenant, and
/// rewrites primary keys / FKs to lead with `user_id`.
#[allow(clippy::too_many_lines)]
fn token_store_multi_tenant_migration(identity: &[u8]) -> Vec<Migration> {
    let id_hex = hex::encode(identity);
    let id_lit = format!("UNHEX('{id_hex}')");

    let mut stmts: Vec<Migration> = Vec::new();

    // Drop the existing FKs FIRST so we can rewrite the parent PKs they
    // reference. Both FKs were defined with explicit `CONSTRAINT` clauses.
    stmts.push(Migration::DropForeignKey {
        name: "fk_token_outputs_metadata",
        table: "token_outputs",
    });
    stmts.push(Migration::DropForeignKey {
        name: "fk_token_outputs_reservation",
        table: "token_outputs",
    });

    // token_metadata: scope per-tenant. Required even though metadata never
    // structurally collides — leaking a token's mere existence (e.g. a
    // 0-balance entry for a token a tenant never held) would be a privacy
    // regression.
    stmts.push(Migration::AddColumn {
        table: "token_metadata",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE token_metadata SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE token_metadata MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE token_metadata DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, identifier)",
    ));
    stmts.push(Migration::DropIndex {
        name: "idx_token_metadata_issuer_pk",
        table: "token_metadata",
    });
    stmts.push(Migration::CreateIndex {
        name: "idx_token_metadata_user_issuer_pk",
        table: "token_metadata",
        columns: "(user_id, issuer_public_key)",
    });

    // token_reservations: scope by user_id.
    stmts.push(Migration::AddColumn {
        table: "token_reservations",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE token_reservations SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE token_reservations MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE token_reservations DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)",
    ));

    // token_outputs: scope by user_id, rekey, re-add composite FKs.
    stmts.push(Migration::AddColumn {
        table: "token_outputs",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE token_outputs SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE token_outputs MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE token_outputs DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)",
    ));
    // Re-add the FKs as composite, scoped by user_id. The reservation FK uses
    // NO ACTION (the default) instead of the previous `ON DELETE SET NULL`:
    // a whole-row SET NULL would null `user_id` (NOT NULL).
    stmts.push(Migration::sql(
        "ALTER TABLE token_outputs \
         ADD CONSTRAINT fk_token_outputs_metadata_user \
         FOREIGN KEY (user_id, token_identifier) \
         REFERENCES token_metadata(user_id, identifier)",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE token_outputs \
         ADD CONSTRAINT fk_token_outputs_reservation_user \
         FOREIGN KEY (user_id, reservation_id) \
         REFERENCES token_reservations(user_id, id)",
    ));
    stmts.push(Migration::DropIndex {
        name: "idx_token_outputs_identifier",
        table: "token_outputs",
    });
    stmts.push(Migration::DropIndex {
        name: "idx_token_outputs_reservation",
        table: "token_outputs",
    });
    stmts.push(Migration::CreateIndex {
        name: "idx_token_outputs_user_identifier",
        table: "token_outputs",
        columns: "(user_id, token_identifier)",
    });
    stmts.push(Migration::CreateIndex {
        name: "idx_token_outputs_user_reservation",
        table: "token_outputs",
        columns: "(user_id, reservation_id)",
    });

    // token_spent_outputs: scope by user_id.
    stmts.push(Migration::AddColumn {
        table: "token_spent_outputs",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE token_spent_outputs SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE token_spent_outputs MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE token_spent_outputs DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, output_id)",
    ));

    // token_swap_status was a singleton (PK id=1, CHECK id=1). Drop the PK
    // and the id column, then re-key by user_id.
    stmts.push(Migration::DropPrimaryKey {
        table: "token_swap_status",
    });
    stmts.push(Migration::DropColumn {
        table: "token_swap_status",
        column: "id",
    });
    stmts.push(Migration::AddColumn {
        table: "token_swap_status",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE token_swap_status SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE token_swap_status MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE token_swap_status ADD PRIMARY KEY (user_id)",
    ));

    stmts
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
        self.acquire_write_lock(&mut conn).await?;
        let result = self
            .set_tokens_outputs_inner(&mut conn, token_outputs, refresh_timestamp)
            .await;
        self.release_write_lock_quiet(&mut conn).await;
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
            .exec(
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
                  JOIN token_outputs o
                    ON o.token_identifier = m.identifier AND o.user_id = m.user_id
                  LEFT JOIN token_reservations r
                    ON o.reservation_id = r.id AND o.user_id = r.user_id
                  WHERE m.user_id = ?
                  GROUP BY m.identifier, m.issuer_public_key, m.name, m.ticker,
                           m.decimals, m.max_supply, m.is_freezable, m.creation_entity_public_key",
                (self.identity.clone(),),
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
            .exec(
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
                  WHERE m.user_id = ?
                  ORDER BY m.identifier, CAST(o.token_amount AS DECIMAL(65,0)) ASC",
                (self.identity.clone(),),
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
              LEFT JOIN token_outputs o
                ON o.token_identifier = m.identifier AND o.user_id = m.user_id
              LEFT JOIN token_reservations r
                ON o.reservation_id = r.id AND o.user_id = r.user_id
              WHERE m.user_id = ? AND {where_clause}
              ORDER BY CAST(o.token_amount AS DECIMAL(65,0)) ASC"
        );

        let rows: Vec<Row> = conn
            .exec(&query, (self.identity.clone(), param))
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

        self.upsert_metadata(&mut tx, &token_outputs.metadata)
            .await?;

        let output_ids: Vec<String> = token_outputs
            .outputs
            .iter()
            .map(|o| o.output.id.clone())
            .collect();
        if !output_ids.is_empty() {
            let placeholders = build_placeholders(output_ids.len());
            let sql = format!(
                "DELETE FROM token_spent_outputs WHERE user_id = ? AND output_id IN ({placeholders})"
            );
            let mut params: Vec<Value> = Vec::with_capacity(output_ids.len().saturating_add(1));
            params.push(Value::from(self.identity.clone()));
            params.extend(output_ids.iter().cloned().map(Value::from));
            tx.exec_drop(&sql, Params::Positional(params))
                .await
                .map_err(map_err)?;
        }

        for output in &token_outputs.outputs {
            self.insert_single_output(&mut tx, &token_outputs.metadata.identifier, output)
                .await?;
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
        self.acquire_write_lock(&mut conn).await?;
        let result = self
            .reserve_token_outputs_inner(
                &mut conn,
                token_identifier,
                target,
                purpose,
                preferred_outputs,
                selection_strategy,
            )
            .await;
        self.release_write_lock_quiet(&mut conn).await;
        result
    }

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        // Scoped to a single `reservation_id`; row-level FK + MVCC suffice.
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        self.cancel_reservation_inner(&mut conn, id).await?;
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
        self.acquire_write_lock(&mut conn).await?;
        let result = self.finalize_reservation_inner(&mut conn, id).await;
        self.release_write_lock_quiet(&mut conn).await;
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
    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_config(
        config: MysqlStorageConfig,
        identity: &[u8],
    ) -> Result<Self, MysqlError> {
        let pool = create_pool(&config)?;
        Self::init(pool, identity).await
    }

    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_pool(pool: Pool, identity: &[u8]) -> Result<Self, MysqlError> {
        Self::init(pool, identity).await
    }

    async fn init(pool: Pool, identity: &[u8]) -> Result<Self, MysqlError> {
        let store = Self {
            pool,
            identity: identity.to_vec(),
            lock_name: identity_lock_name(TOKEN_STORE_LOCK_PREFIX, identity),
        };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), MysqlError> {
        run_migrations(
            &self.pool,
            TOKEN_MIGRATIONS_TABLE,
            &Self::migrations(&self.identity),
        )
        .await
    }

    fn migrations(identity: &[u8]) -> Vec<Vec<Migration>> {
        vec![
            vec![
                Migration::sql(
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
                ),
                Migration::CreateIndex {
                    name: "idx_token_metadata_issuer_pk",
                    table: "token_metadata",
                    columns: "(issuer_public_key)",
                },
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS token_reservations (
                        id VARCHAR(255) NOT NULL PRIMARY KEY,
                        purpose VARCHAR(64) NOT NULL,
                        created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
                    )",
                ),
                Migration::sql(
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
                ),
                Migration::CreateIndex {
                    name: "idx_token_outputs_identifier",
                    table: "token_outputs",
                    columns: "(token_identifier)",
                },
                Migration::CreateIndex {
                    name: "idx_token_outputs_reservation",
                    table: "token_outputs",
                    columns: "(reservation_id)",
                },
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS token_spent_outputs (
                        output_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        spent_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
                    )",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS token_swap_status (
                        id INT NOT NULL PRIMARY KEY DEFAULT 1,
                        last_completed_at DATETIME(6) NULL,
                        CHECK (id = 1)
                    )",
                ),
                Migration::sql(
                    "INSERT INTO token_swap_status (id) VALUES (1)
                     ON DUPLICATE KEY UPDATE id = id",
                ),
            ],
            // Migration 2: Multi-tenant scoping.
            token_store_multi_tenant_migration(identity),
        ]
    }

    async fn acquire_write_lock(&self, conn: &mut Conn) -> Result<(), TokenOutputServiceError> {
        let acquired: Option<i64> = conn
            .exec_first(
                "SELECT GET_LOCK(?, ?)",
                (self.lock_name.as_str(), WRITE_LOCK_TIMEOUT_SECS),
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

    async fn release_write_lock_quiet(&self, conn: &mut Conn) {
        let _ = conn
            .exec_drop("SELECT RELEASE_LOCK(?)", (self.lock_name.as_str(),))
            .await;
    }

    #[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
    async fn set_tokens_outputs_inner(
        &self,
        conn: &mut Conn,
        token_outputs: &[TokenOutputs],
        refresh_timestamp: DateTime<Utc>,
    ) -> Result<(), TokenOutputServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        self.cleanup_stale_reservations(&mut tx).await?;

        let row: Option<(i64, i64)> = tx
            .exec_first(
                r"SELECT
                    (SELECT EXISTS(SELECT 1 FROM token_reservations WHERE user_id = ? AND purpose = 'Swap')) AS has_active_swap,
                    COALESCE(
                        (SELECT (last_completed_at >= ?) FROM token_swap_status WHERE user_id = ?),
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
            trace!(
                "Skipping set_tokens_outputs: active_swap={}, swap_completed_during_refresh={}",
                has_active_swap, swap_completed_during_refresh
            );
            return Ok(());
        }

        self.cleanup_spent_markers(&mut tx, refresh_timestamp)
            .await?;

        let spent_rows: Vec<String> = tx
            .exec(
                "SELECT output_id FROM token_spent_outputs WHERE user_id = ? AND spent_at >= ?",
                (self.identity.clone(), refresh_timestamp.naive_utc()),
            )
            .await
            .map_err(map_err)?;
        let spent_ids: HashSet<String> = spent_rows.into_iter().collect();

        tx.exec_drop(
            "DELETE FROM token_outputs WHERE user_id = ? AND reservation_id IS NULL AND added_at < ?",
            (self.identity.clone(), refresh_timestamp.naive_utc()),
        )
        .await
        .map_err(map_err)?;

        let incoming_output_ids: HashSet<String> = token_outputs
            .iter()
            .flat_map(|to| to.outputs.iter().map(|o| o.output.id.clone()))
            .collect();

        let reserved_pairs: Vec<(String, String)> = tx
            .exec(
                r"SELECT r.id, o.id
                  FROM token_reservations r
                  JOIN token_outputs o
                    ON o.reservation_id = r.id AND o.user_id = r.user_id
                  WHERE r.user_id = ?",
                (self.identity.clone(),),
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
            let outputs_sql = format!(
                "DELETE FROM token_outputs WHERE user_id = ? AND reservation_id IN ({placeholders})"
            );
            let mut outputs_params: Vec<Value> =
                Vec::with_capacity(reservations_to_delete.len().saturating_add(1));
            outputs_params.push(Value::from(self.identity.clone()));
            outputs_params.extend(reservations_to_delete.iter().cloned().map(Value::from));
            tx.exec_drop(&outputs_sql, Params::Positional(outputs_params))
                .await
                .map_err(map_err)?;

            let res_sql = format!(
                "DELETE FROM token_reservations WHERE user_id = ? AND id IN ({placeholders})"
            );
            let mut res_params: Vec<Value> =
                Vec::with_capacity(reservations_to_delete.len().saturating_add(1));
            res_params.push(Value::from(self.identity.clone()));
            res_params.extend(reservations_to_delete.iter().cloned().map(Value::from));
            tx.exec_drop(&res_sql, Params::Positional(res_params))
                .await
                .map_err(map_err)?;
        }

        if !outputs_to_remove_from_reservation.is_empty() {
            let placeholders = build_placeholders(outputs_to_remove_from_reservation.len());
            let sql =
                format!("DELETE FROM token_outputs WHERE user_id = ? AND id IN ({placeholders})");
            let mut params: Vec<Value> =
                Vec::with_capacity(outputs_to_remove_from_reservation.len().saturating_add(1));
            params.push(Value::from(self.identity.clone()));
            params.extend(
                outputs_to_remove_from_reservation
                    .iter()
                    .cloned()
                    .map(Value::from),
            );
            tx.exec_drop(&sql, Params::Positional(params))
                .await
                .map_err(map_err)?;

            let empty_ids: Vec<String> = tx
                .exec(
                    r"SELECT r.id FROM token_reservations r
                      LEFT JOIN token_outputs o
                        ON o.reservation_id = r.id AND o.user_id = r.user_id
                      WHERE r.user_id = ? AND o.id IS NULL",
                    (self.identity.clone(),),
                )
                .await
                .map_err(map_err)?;
            if !empty_ids.is_empty() {
                let placeholders = build_placeholders(empty_ids.len());
                let sql = format!(
                    "DELETE FROM token_reservations WHERE user_id = ? AND id IN ({placeholders})"
                );
                let mut params: Vec<Value> = Vec::with_capacity(empty_ids.len().saturating_add(1));
                params.push(Value::from(self.identity.clone()));
                params.extend(empty_ids.iter().cloned().map(Value::from));
                tx.exec_drop(&sql, Params::Positional(params))
                    .await
                    .map_err(map_err)?;
            }
        }

        let reserved_output_ids: HashSet<String> = tx
            .exec::<String, _, _>(
                "SELECT id FROM token_outputs WHERE user_id = ? AND reservation_id IS NOT NULL",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_err)?
            .into_iter()
            .collect();

        // Drop tenant-scoped metadata rows that no longer have any outputs.
        tx.exec_drop(
            r"DELETE FROM token_metadata
              WHERE user_id = ?
                AND identifier NOT IN (
                  SELECT DISTINCT token_identifier FROM token_outputs WHERE user_id = ?
              )",
            (self.identity.clone(), self.identity.clone()),
        )
        .await
        .map_err(map_err)?;

        for to in token_outputs {
            self.upsert_metadata(&mut tx, &to.metadata).await?;

            for output in &to.outputs {
                if reserved_output_ids.contains(&output.output.id)
                    || spent_ids.contains(&output.output.id)
                {
                    continue;
                }
                self.insert_single_output(&mut tx, &to.metadata.identifier, output)
                    .await?;
            }
        }

        tx.commit().await.map_err(map_err)?;

        trace!("Updated {} token outputs in MySQL", token_outputs.len());
        Ok(())
    }

    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    async fn reserve_token_outputs_inner(
        &self,
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
                "SELECT * FROM token_metadata WHERE user_id = ? AND identifier = ?",
                (self.identity.clone(), token_identifier),
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
                  WHERE o.user_id = ? AND o.token_identifier = ? AND o.reservation_id IS NULL",
                (self.identity.clone(), token_identifier),
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
            "INSERT INTO token_reservations (user_id, id, purpose) VALUES (?, ?, ?)",
            (self.identity.clone(), &reservation_id, purpose_str),
        )
        .await
        .map_err(map_err)?;

        let selected_ids: Vec<String> = selected_outputs
            .iter()
            .map(|o| o.output.id.clone())
            .collect();
        if !selected_ids.is_empty() {
            let placeholders = build_placeholders(selected_ids.len());
            let sql = format!(
                "UPDATE token_outputs SET reservation_id = ? WHERE user_id = ? AND id IN ({placeholders})"
            );
            let mut params: Vec<Value> = Vec::with_capacity(selected_ids.len() + 2);
            params.push(Value::from(reservation_id.clone()));
            params.push(Value::from(self.identity.clone()));
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
        &self,
        conn: &mut Conn,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        tx.exec_drop(
            "UPDATE token_outputs SET reservation_id = NULL WHERE user_id = ? AND reservation_id = ?",
            (self.identity.clone(), id),
        )
        .await
        .map_err(map_err)?;

        tx.exec_drop(
            "DELETE FROM token_reservations WHERE user_id = ? AND id = ?",
            (self.identity.clone(), id),
        )
        .await
        .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn finalize_reservation_inner(
        &self,
        conn: &mut Conn,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_err)?;

        let purpose: Option<String> = tx
            .exec_first(
                "SELECT purpose FROM token_reservations WHERE user_id = ? AND id = ?",
                (self.identity.clone(), id),
            )
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
                "SELECT id FROM token_outputs WHERE user_id = ? AND reservation_id = ?",
                (self.identity.clone(), id),
            )
            .await
            .map_err(map_err)?;

        if !reserved_output_ids.is_empty() {
            let mut sql =
                String::from("INSERT INTO token_spent_outputs (user_id, output_id) VALUES ");
            let mut params: Vec<Value> =
                Vec::with_capacity(reserved_output_ids.len().saturating_mul(2));
            for (i, oid) in reserved_output_ids.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push_str("(?, ?)");
                params.push(Value::from(self.identity.clone()));
                params.push(Value::from(oid.clone()));
            }
            // Suppress duplicate-PK errors only.
            sql.push_str(" ON DUPLICATE KEY UPDATE output_id = output_id");
            tx.exec_drop(&sql, Params::Positional(params))
                .await
                .map_err(map_err)?;
        }

        tx.exec_drop(
            "DELETE FROM token_outputs WHERE user_id = ? AND reservation_id = ?",
            (self.identity.clone(), id),
        )
        .await
        .map_err(map_err)?;

        tx.exec_drop(
            "DELETE FROM token_reservations WHERE user_id = ? AND id = ?",
            (self.identity.clone(), id),
        )
        .await
        .map_err(map_err)?;

        // UPSERT the per-tenant swap-status row so a tenant that joined after
        // the multi-tenant migration (and thus has no row) gets one created
        // lazily.
        if is_swap {
            tx.exec_drop(
                "INSERT INTO token_swap_status (user_id, last_completed_at) VALUES (?, NOW(6))
                 ON DUPLICATE KEY UPDATE last_completed_at = VALUES(last_completed_at)",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_err)?;
        }

        tx.exec_drop(
            r"DELETE FROM token_metadata
              WHERE user_id = ?
                AND identifier NOT IN (
                  SELECT DISTINCT token_identifier FROM token_outputs WHERE user_id = ?
              )",
            (self.identity.clone(), self.identity.clone()),
        )
        .await
        .map_err(map_err)?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    #[allow(clippy::cast_possible_wrap)]
    async fn insert_single_output(
        &self,
        tx: &mut mysql_async::Transaction<'_>,
        token_identifier: &str,
        output: &TokenOutputWithPrevOut,
    ) -> Result<(), TokenOutputServiceError> {
        tx.exec_drop(
            // ON DUPLICATE KEY UPDATE id = id no-ops on the (user_id, id)
            // primary key conflict only — unlike INSERT IGNORE, FK / NOT NULL
            // / type errors still propagate.
            r"INSERT INTO token_outputs
                (user_id, id, token_identifier, owner_public_key, revocation_commitment,
                 withdraw_bond_sats, withdraw_relative_block_locktime,
                 token_public_key, token_amount, prev_tx_hash, prev_tx_vout, added_at)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NOW(6))
              ON DUPLICATE KEY UPDATE id = id",
            (
                self.identity.clone(),
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
        &self,
        tx: &mut mysql_async::Transaction<'_>,
        metadata: &TokenMetadata,
    ) -> Result<(), TokenOutputServiceError> {
        tx.exec_drop(
            r"INSERT INTO token_metadata
                (user_id, identifier, issuer_public_key, name, ticker, decimals, max_supply,
                 is_freezable, creation_entity_public_key)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
              ON DUPLICATE KEY UPDATE
                issuer_public_key = VALUES(issuer_public_key),
                name = VALUES(name),
                ticker = VALUES(ticker),
                decimals = VALUES(decimals),
                max_supply = VALUES(max_supply),
                is_freezable = VALUES(is_freezable),
                creation_entity_public_key = VALUES(creation_entity_public_key)",
            (
                self.identity.clone(),
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

    /// Cleans up stale reservations for THIS tenant. Releases dependent
    /// outputs by clearing `reservation_id` first (the composite FK uses NO
    /// ACTION because column-list SET NULL would null `user_id`).
    async fn cleanup_stale_reservations(
        &self,
        tx: &mut mysql_async::Transaction<'_>,
    ) -> Result<u64, TokenOutputServiceError> {
        tx.exec_drop(
            r"UPDATE token_outputs SET reservation_id = NULL
              WHERE user_id = ?
                AND reservation_id IN (
                    SELECT id FROM (
                        SELECT id FROM token_reservations
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
                "DELETE FROM token_reservations
                 WHERE user_id = ? AND created_at < DATE_SUB(NOW(6), INTERVAL ? SECOND)",
                (self.identity.clone(), RESERVATION_TIMEOUT_SECS),
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
        &self,
        tx: &mut mysql_async::Transaction<'_>,
        refresh_timestamp: DateTime<Utc>,
    ) -> Result<(), TokenOutputServiceError> {
        let threshold = chrono::Duration::milliseconds(SPENT_MARKER_CLEANUP_THRESHOLD_MS);
        let cleanup_cutoff = refresh_timestamp
            .checked_sub_signed(threshold)
            .unwrap_or(refresh_timestamp);

        tx.exec_drop(
            "DELETE FROM token_spent_outputs WHERE user_id = ? AND spent_at < ?",
            (self.identity.clone(), cleanup_cutoff.naive_utc()),
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

/// Creates a `MysqlTokenStore` from a configuration.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_mysql_token_store(
    config: MysqlStorageConfig,
    identity: &[u8],
) -> Result<Arc<dyn TokenOutputStore>, MysqlError> {
    Ok(Arc::new(
        MysqlTokenStore::from_config(config, identity).await?,
    ))
}

/// Creates a `MysqlTokenStore` from an existing connection pool.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_mysql_token_store_from_pool(
    pool: Pool,
    identity: &[u8],
) -> Result<Arc<dyn TokenOutputStore>, MysqlError> {
    Ok(Arc::new(MysqlTokenStore::from_pool(pool, identity).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::token_store_tests as shared_tests;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::mysql::Mysql;

    /// Fixed 33-byte test identity. Tests run in their own ephemeral container,
    /// so a single shared identity is fine — the schema still gets exercised.
    pub(super) const TEST_IDENTITY: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];

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

            let store = MysqlTokenStore::from_config(
                MysqlStorageConfig::with_defaults(connection_string),
                &TEST_IDENTITY,
            )
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

    // ---- newly wired shared tests, parity with spark-postgres ----

    #[tokio::test]
    async fn test_cancel_nonexistent_reservation() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_cancel_nonexistent_reservation(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_finalize_nonexistent_reservation() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_finalize_nonexistent_reservation(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_token_outputs_none_found() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_get_token_outputs_none_found(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_insert_outputs_clears_spent_status() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_insert_outputs_clears_spent_status(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_insert_outputs_preserved_by_set_tokens_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_insert_outputs_preserved_by_set_tokens_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_mixed_reservation_purposes_balance() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_mixed_reservation_purposes_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_multiple_parallel_reservations() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_multiple_parallel_reservations(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_all_available_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_all_available_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_exact_amount_match() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_exact_amount_match(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_for_payment_affects_balance() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_for_payment_affects_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_for_swap_does_not_affect_balance() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_for_swap_does_not_affect_balance(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_insufficient_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_insufficient_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_max_output_count_largest_first() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_max_output_count_largest_first(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_max_output_count_more_than_available() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_max_output_count_more_than_available(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_max_output_count_smallest_first() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_max_output_count_smallest_first(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_max_output_count_zero_rejected() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_max_output_count_zero_rejected(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_multiple_outputs_combination() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_multiple_outputs_combination(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_nonexistent_token() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_nonexistent_token(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_single_large_output() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_single_large_output(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_and_cancel() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_and_cancel(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_and_set_add_output() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_and_set_add_output(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_and_set_remove_reserved_output() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_and_set_remove_reserved_output(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_selection_strategy_largest_first() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_selection_strategy_largest_first(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_reserve_token_outputs_selection_strategy_smallest_first() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_token_outputs_selection_strategy_smallest_first(&fixture.store)
            .await;
    }

    #[tokio::test]
    async fn test_reserve_with_preferred_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_with_preferred_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_with_preferred_outputs_insufficient() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_with_preferred_outputs_insufficient(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_reserve_zero_amount() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_reserve_zero_amount(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_reconciles_reservation_with_empty_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_set_reconciles_reservation_with_empty_outputs(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_removes_all_tokens() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_set_removes_all_tokens(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_tokens_outputs_skipped_after_swap_completes_during_refresh() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_set_tokens_outputs_skipped_after_swap_completes_during_refresh(
            &fixture.store,
        )
        .await;
    }

    #[tokio::test]
    async fn test_set_tokens_outputs_skipped_during_active_swap() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_set_tokens_outputs_skipped_during_active_swap(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_tokens_outputs_with_update() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_set_tokens_outputs_with_update(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_spent_outputs_not_restored_by_set_tokens_outputs() {
        let fixture = MysqlTokenStoreTestFixture::new().await;
        shared_tests::test_spent_outputs_not_restored_by_set_tokens_outputs(&fixture.store).await;
    }

    // ==================== MySQL-Specific Tests ====================

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

        // Hold the per-tenant named lock on a separate connection.
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
            tokio::spawn(async move { store_for_task.finalize_reservation(&res_id).await });

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            !finalize_task.is_finished(),
            "finalize_reservation completed while named lock was held — \
             the lock is not being acquired"
        );

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
    }

    #[tokio::test]
    async fn test_stale_swap_reservation_does_not_block_set_tokens_outputs() {
        // Regression test mirroring the tree store fix: a stale Swap reservation
        // must be cleaned up before has_active_swap is evaluated, otherwise the
        // reservation pins itself in place and the local token-output set freezes.
        let fixture = MysqlTokenStoreTestFixture::new().await;
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
        let mut conn = fixture.store.pool.get_conn().await.unwrap();
        conn.exec_drop(
            "UPDATE token_reservations SET created_at = DATE_SUB(NOW(6), INTERVAL 10 MINUTE) WHERE id = ?",
            (&reservation.id,),
        )
        .await
        .unwrap();
        drop(conn);

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

    // ==================== Multi-tenant isolation ====================

    /// A second 33-byte test identity (must differ from `TEST_IDENTITY`).
    const TEST_IDENTITY_B: [u8; 33] = [
        0x03, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae,
        0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd,
        0xbe, 0xbf, 0xc0,
    ];

    /// Two `MysqlTokenStore` instances with distinct identities sharing one
    /// connection pool / DB. The container must be kept alive for the test.
    struct TwoTenantTokenFixture {
        a: MysqlTokenStore,
        b: MysqlTokenStore,
        #[allow(dead_code)]
        container: ContainerAsync<Mysql>,
    }

    impl TwoTenantTokenFixture {
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

            let a = MysqlTokenStore::from_pool(pool.clone(), &TEST_IDENTITY)
                .await
                .expect("Failed to create tenant A");
            let b = MysqlTokenStore::from_pool(pool, &TEST_IDENTITY_B)
                .await
                .expect("Failed to create tenant B");

            Self { a, b, container }
        }
    }

    /// End-to-end isolation: every `TokenOutputStore` method must keep tenants
    /// A and B from observing each other's data. Critically, `token_metadata`
    /// is per-tenant — both tenants seeding the same `identifier` ("token-1")
    /// must coexist without collision and without leaking each other's
    /// balances.
    #[tokio::test]
    #[allow(clippy::too_many_lines, clippy::similar_names)]
    async fn test_two_tenant_isolation() {
        let fx = TwoTenantTokenFixture::new().await;

        let a_token1 = shared_tests::create_token_outputs(1, vec![100, 200]);
        let b_token1 = shared_tests::create_token_outputs(1, vec![500, 1_000, 2_000]);
        let b_only_token2 = shared_tests::create_token_outputs(2, vec![777]);

        fx.a.set_tokens_outputs(
            std::slice::from_ref(&a_token1),
            shared_tests::future_refresh_start(),
        )
        .await
        .unwrap();
        fx.b.set_tokens_outputs(
            &[b_token1, b_only_token2],
            shared_tests::future_refresh_start(),
        )
        .await
        .unwrap();

        let listed_a = fx.a.list_tokens_outputs().await.unwrap();
        let listed_b = fx.b.list_tokens_outputs().await.unwrap();
        assert_eq!(listed_a.len(), 1, "A only sees its own token");
        assert_eq!(listed_b.len(), 2, "B sees its two tokens");
        assert!(
            !listed_a.iter().any(|t| t.metadata.identifier == "token-2"),
            "A must not see B's token-2 (no zero-balance leakage)"
        );
        assert_eq!(listed_a[0].available.len(), 2, "A has 2 outputs of token-1");
        let listed_b_t1 = listed_b
            .iter()
            .find(|t| t.metadata.identifier == "token-1")
            .unwrap();
        assert_eq!(listed_b_t1.available.len(), 3, "B has 3 outputs of token-1");

        let bal_a = fx.a.get_token_balances().await.unwrap();
        let bal_b = fx.b.get_token_balances().await.unwrap();
        assert_eq!(bal_a.len(), 1);
        assert_eq!(bal_a[0].0.identifier, "token-1");
        assert_eq!(bal_a[0].1, 300, "A's token-1 balance is just A's outputs");
        let bal_b_map: HashMap<String, u128> =
            bal_b.into_iter().map(|(m, v)| (m.identifier, v)).collect();
        assert_eq!(bal_b_map.get("token-1"), Some(&3_500));
        assert_eq!(bal_b_map.get("token-2"), Some(&777));

        let got_a =
            fx.a.get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
                .await
                .unwrap();
        assert_eq!(got_a.available.len(), 2);

        let got_a_t2 =
            fx.a.get_token_outputs(GetTokenOutputsFilter::Identifier("token-2"))
                .await;
        assert!(
            matches!(got_a_t2, Err(TokenOutputServiceError::Generic(_))),
            "A must not be able to read B's token-2 metadata"
        );

        let res_a =
            fx.a.reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(100),
                TokenReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();
        assert!(!res_a.token_outputs.outputs.is_empty());

        let view_b = fx.b.list_tokens_outputs().await.unwrap();
        let view_b_t1 = view_b
            .iter()
            .find(|t| t.metadata.identifier == "token-1")
            .unwrap();
        assert!(
            view_b_t1.reserved_for_payment.is_empty(),
            "B must not see A's reservation"
        );
        assert_eq!(view_b_t1.available.len(), 3);

        let res_a_t2 =
            fx.a.reserve_token_outputs(
                "token-2",
                ReservationTarget::MinTotalValue(100),
                TokenReservationPurpose::Payment,
                None,
                None,
            )
            .await;
        assert!(
            res_a_t2.is_err(),
            "A must not be able to reserve from B's token-2"
        );

        fx.a.finalize_reservation(&res_a.id).await.unwrap();
        let view_b = fx.b.list_tokens_outputs().await.unwrap();
        let view_b_t1 = view_b
            .iter()
            .find(|t| t.metadata.identifier == "token-1")
            .unwrap();
        assert_eq!(view_b_t1.available.len(), 3, "B's outputs untouched");
        assert_eq!(fx.b.get_token_balances().await.unwrap().len(), 2);

        // swap-status row is per tenant.
        let res_b_swap =
            fx.b.reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(500),
                TokenReservationPurpose::Swap,
                None,
                None,
            )
            .await
            .unwrap();
        let listed_a = fx.a.list_tokens_outputs().await.unwrap();
        assert!(
            listed_a.iter().all(|t| t.reserved_for_swap.is_empty()),
            "A must not see B's swap reservation"
        );
        fx.b.finalize_reservation(&res_b_swap.id).await.unwrap();

        // insert_token_outputs on A only inserts into A's namespace. Use
        // identifier_no=2 so the metadata identifier ("token-2") collides
        // with B's existing entry — both tenants must end up with their own
        // row.
        let a_token2 = shared_tests::create_token_outputs(2, vec![999]);
        fx.a.insert_token_outputs(&a_token2).await.unwrap();

        let bal_a = fx.a.get_token_balances().await.unwrap();
        let bal_a_t2 = bal_a
            .iter()
            .find(|(m, _)| m.identifier == "token-2")
            .expect("A should now have its own token-2");
        assert_eq!(bal_a_t2.1, 999, "A's token-2 balance is A's output only");

        let bal_b = fx.b.get_token_balances().await.unwrap();
        let bal_b_t2 = bal_b
            .iter()
            .find(|(m, _)| m.identifier == "token-2")
            .expect("B's token-2 still present");
        assert_eq!(
            bal_b_t2.1, 777,
            "B's token-2 balance unchanged by A's insert"
        );
    }
}
