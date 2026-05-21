//! Generic `MySQL` migration runner with version tracking and concurrency control.
//!
//! Uses `GET_LOCK`/`RELEASE_LOCK` (`MySQL` named locks) to serialize migrations
//! across concurrent connections. Named locks are session-scoped (not
//! transaction-scoped like Postgres `pg_advisory_xact_lock`), so the lock is
//! acquired before the transaction and explicitly released after commit /
//! rollback. The connection is held for the entire migration sequence so the
//! session — and therefore the lock — stays bound to a single client.
//!
//! ## Why migrations are expressed as a [`Migration`] enum and not raw SQL
//!
//! `MySQL` DDL (`CREATE INDEX`, `ALTER TABLE ADD/DROP COLUMN`, `CREATE TABLE`,
//! etc.) implicitly commits the surrounding transaction. If a migration that
//! chains multiple DDL statements crashes partway, the earlier statements
//! remain applied while the migration's version row never gets inserted —
//! restart re-runs the whole migration and fails on the partially applied
//! DDL (`ER_DUP_FIELDNAME` for the column, `ER_DUP_KEYNAME` for the index).
//!
//! Vanilla `MySQL` 8.x only supports `IF [NOT] EXISTS` on `CREATE TABLE`,
//! `DROP TABLE`, `DROP INDEX`, and a few others — not on `CREATE INDEX`,
//! `ADD COLUMN`, or `DROP COLUMN`. Rather than ask migration authors to
//! hand-roll `information_schema` guards (verbose) or rely on the runner
//! catching duplicate-object error codes (silently masks accidental dup
//! definitions), we model the non-idempotent operations as enum variants:
//!
//! ```rust,ignore
//! Migration::AddColumn { table: "tree_leaves", column: "value",
//!     definition: "BIGINT NOT NULL DEFAULT 0" }
//! Migration::CreateIndex { name: "idx_tree_leaves_slim", table: "tree_leaves",
//!     columns: "(status, is_missing_from_operators, reservation_id, value)" }
//! Migration::DropColumn { table: "lnurl_receive_metadata", column: "preimage" }
//! Migration::AddForeignKey { name: "fk_tree_leaves_reservation", table: "tree_leaves",
//!     definition: "FOREIGN KEY (reservation_id) REFERENCES tree_reservations(id) ON DELETE SET NULL" }
//! ```
//!
//! The runner emits guarded SQL for each variant: a pre-flight check against
//! `information_schema` followed by the DDL only when the object isn't
//! already in the desired state. Re-running a partially applied migration
//! becomes a no-op for the already-applied statements without swallowing
//! genuine errors (parse errors, missing tables, permission denials, etc.).
//!
//! [`Migration::Sql`] is the escape hatch for already-idempotent statements
//! (`CREATE TABLE IF NOT EXISTS`, `INSERT … ON DUPLICATE KEY UPDATE id = id`,
//! plain DML) and any DDL that doesn't fit one of the structured variants —
//! those run as-is and their errors propagate normally. Avoid `INSERT IGNORE`
//! for idempotent inserts: it silently swallows non-PK errors (FK / NOT NULL
//! / type errors). Use the explicit `ON DUPLICATE KEY UPDATE` form so only
//! genuine duplicate-key collisions are no-op'd.

use mysql_async::Pool;
use mysql_async::prelude::*;

use crate::error::MysqlError;
use crate::pool::map_db_error;

/// Timeout (seconds) when waiting for the migration `GET_LOCK`.
const MIGRATION_LOCK_TIMEOUT_SECS: i64 = 60;

/// A single migration step.
///
/// Use [`Migration::Sql`] for statements that are already idempotent (e.g.
/// `CREATE TABLE IF NOT EXISTS`, `INSERT … ON DUPLICATE KEY UPDATE id = id`,
/// plain DML) or that don't fit one of the structured variants. Use the
/// structured variants for the non-idempotent DDL — `MySQL` doesn't support
/// `IF NOT EXISTS` on add/drop column or create index, so the runner emits
/// an `information_schema` guard before each statement.
///
/// `Sql` carries an owned `String` so callers can build statements at runtime
/// (e.g. inlining a tenant identity into a backfill); the structured variants
/// keep `&'static str` because their identifiers are always known at compile
/// time.
#[derive(Clone, Debug)]
pub enum Migration {
    /// Run the SQL statement as-is. Errors propagate.
    Sql(String),

    /// `ALTER TABLE <table> ADD COLUMN <column> <definition>`, guarded by an
    /// `information_schema.columns` lookup so re-running an already applied
    /// migration is a no-op.
    AddColumn {
        table: &'static str,
        column: &'static str,
        /// Full column definition, e.g. `"BIGINT NOT NULL DEFAULT 0"`.
        definition: &'static str,
    },

    /// `ALTER TABLE <table> DROP COLUMN <column>`, guarded so it no-ops if
    /// the column has already been dropped.
    DropColumn {
        table: &'static str,
        column: &'static str,
    },

    /// `CREATE INDEX <name> ON <table><columns>`, guarded by an
    /// `information_schema.statistics` lookup so re-running a partially
    /// applied migration is a no-op.
    CreateIndex {
        name: &'static str,
        table: &'static str,
        /// Parenthesised column list, e.g. `"(status, value)"`.
        columns: &'static str,
    },

    /// `DROP INDEX <name> ON <table>`, guarded so it no-ops if the index has
    /// already been dropped. (`MySQL` 8.0.16+ supports `DROP INDEX IF EXISTS`
    /// natively, but we handle it here for parity and older versions.)
    DropIndex {
        name: &'static str,
        table: &'static str,
    },

    /// Adds a foreign-key constraint on `table` named `name`, guarded by an
    /// `information_schema.table_constraints` lookup so re-running an already
    /// applied migration is a no-op. `MySQL` doesn't support
    /// `ADD CONSTRAINT … IF NOT EXISTS`, so this variant emulates it.
    AddForeignKey {
        name: &'static str,
        table: &'static str,
        /// Body after `ADD CONSTRAINT <name>`, e.g.
        /// `"FOREIGN KEY (col) REFERENCES other(col) ON DELETE SET NULL"`.
        definition: &'static str,
    },

    /// Drops a foreign-key constraint on `table` named `name`, guarded by an
    /// `information_schema.table_constraints` lookup so re-running an already
    /// applied migration is a no-op. Needed when rewriting parent PKs to lead
    /// with `user_id`: dependent FKs must be dropped first.
    DropForeignKey {
        name: &'static str,
        table: &'static str,
    },

    /// `ALTER TABLE <table> DROP PRIMARY KEY`, guarded by an
    /// `information_schema.table_constraints` lookup so re-running an already
    /// applied migration is a no-op. Needed when rewriting PKs to lead with
    /// `user_id`: a partial-apply replay would otherwise fail with
    /// `ER_CANT_DROP_FIELD_OR_KEY`.
    DropPrimaryKey { table: &'static str },
}

impl Migration {
    /// Convenience constructor for the common case of a `&'static str` literal.
    pub fn sql(s: &str) -> Self {
        Self::Sql(s.to_string())
    }
}

/// `MySQL` FK rename. `MySQL` has no `RENAME CONSTRAINT` for foreign keys, so
/// [`run_migrations`] drops the old FK and re-adds it under `new_name`
/// using `definition`. `table` is the new (post-rename) table name.
pub struct FkRename<'a> {
    pub table: &'a str,
    pub old_name: &'a str,
    pub new_name: &'a str,
    /// Body after `ADD CONSTRAINT <new_name>`, e.g.
    /// `"FOREIGN KEY (col) REFERENCES other(col) ON DELETE SET NULL"`.
    pub definition: &'a str,
}

/// One-shot pre-prefix rename map for [`run_migrations`]. Lists every
/// owned table / index / FK to move, plus the migrations tracker itself.
/// `old_migrations_table` is the canary — if missing, the rename is
/// skipped (DB is fresh or already upgraded).
///
/// `MySQL` PKs are always named `PRIMARY` (table-scoped), so PKs travel
/// with their tables automatically.
pub struct SchemaRenames<'a> {
    /// Old name of the per-store schema migrations table; used as the
    /// canary check.
    pub old_migrations_table: &'a str,
    /// New name of the per-store schema migrations table. Must match the
    /// `migrations_table` argument to [`run_migrations`].
    pub new_migrations_table: &'a str,
    /// `(old_table, new_table)` pairs.
    pub tables: &'a [(&'a str, &'a str)],
    /// `(parent_table_new_name, old_index, new_index)` triples — `MySQL`
    /// indexes are renamed via `ALTER TABLE <table> RENAME INDEX <old> TO <new>`.
    pub indexes: &'a [(&'a str, &'a str, &'a str)],
    /// Foreign keys to drop-and-recreate under the new name.
    pub foreign_keys: &'a [FkRename<'a>],
}

/// Applies a one-shot schema rename on the caller's connection. Returns
/// early if the canary `renames.old_migrations_table` is absent.
///
/// `MySQL` DDL auto-commits, so renames aren't transactional — the migrations
/// tracking table is renamed **last** so a crash mid-sequence leaves the
/// canary pointing at the old name. Each step is `information_schema`-guarded
/// so the replay is idempotent.
async fn apply_renames(
    conn: &mut mysql_async::Conn,
    renames: &SchemaRenames<'_>,
) -> Result<(), MysqlError> {
    // Canary: if the old migrations table doesn't exist, nothing to do.
    if !table_exists(conn, renames.old_migrations_table).await? {
        return Ok(());
    }

    // Each step is information_schema-guarded for replay safety after a
    // crash mid-sequence (DDL auto-commits in `MySQL`).
    for (old, new) in renames.tables {
        if !table_exists(conn, old).await? || table_exists(conn, new).await? {
            continue;
        }
        let sql = format!("RENAME TABLE `{old}` TO `{new}`");
        conn.query_drop(&sql).await.map_err(|e| {
            MysqlError::Database(format!("Failed to rename table {old} -> {new}: {e}"))
        })?;
    }

    for (table, old, new) in renames.indexes {
        if !index_exists(conn, table, old).await? || index_exists(conn, table, new).await? {
            continue;
        }
        let sql = format!("ALTER TABLE `{table}` RENAME INDEX `{old}` TO `{new}`");
        conn.query_drop(&sql).await.map_err(|e| {
            MysqlError::Database(format!(
                "Failed to rename index {old} -> {new} on {table}: {e}"
            ))
        })?;
    }

    for fk in renames.foreign_keys {
        if foreign_key_exists(conn, fk.table, fk.new_name).await? {
            continue;
        }
        if !foreign_key_exists(conn, fk.table, fk.old_name).await? {
            continue;
        }
        let alter_sql = format!(
            "ALTER TABLE `{}` DROP FOREIGN KEY `{}`, ADD CONSTRAINT `{}` {}",
            fk.table, fk.old_name, fk.new_name, fk.definition
        );
        conn.query_drop(&alter_sql).await.map_err(|e| {
            MysqlError::Database(format!(
                "Failed to rename FK {} -> {} on {}: {e}",
                fk.old_name, fk.new_name, fk.table
            ))
        })?;
    }

    // Rename the migrations tracking table last; it doubles as the canary
    // that the rename completed.
    if table_exists(conn, renames.old_migrations_table).await?
        && !table_exists(conn, renames.new_migrations_table).await?
    {
        let sql = format!(
            "RENAME TABLE `{}` TO `{}`",
            renames.old_migrations_table, renames.new_migrations_table
        );
        conn.query_drop(&sql).await.map_err(map_db_error)?;
    }

    Ok(())
}

async fn table_exists(conn: &mut mysql_async::Conn, table: &str) -> Result<bool, MysqlError> {
    let count: Option<i64> = conn
        .exec_first(
            "SELECT COUNT(*) FROM information_schema.tables
             WHERE table_schema = DATABASE() AND table_name = ?",
            (table,),
        )
        .await
        .map_err(map_db_error)?;
    Ok(count.unwrap_or(0) > 0)
}

/// Runs database migrations with version tracking and concurrency control.
///
/// This function:
/// - Acquires a named lock (derived from the migrations table name) to prevent concurrent migrations
/// - Optionally applies a one-shot schema rename first (see [`SchemaRenames`])
/// - Creates the migrations tracking table if it doesn't exist
/// - Applies only new migrations (based on version number)
/// - Releases the lock before returning
///
/// Rename + migrations share one `GET_LOCK` on the session connection.
///
/// # Arguments
/// * `pool` - The connection pool to use
/// * `migrations_table` - Name of the table to track migration versions (e.g., `schema_migrations`)
/// * `migrations` - List of migrations, where each migration is a list of [`Migration`] steps
/// * `renames` - When `Some`, applied before migrations; canary-gated so
///   fresh / already-upgraded DBs pay only one probe.
#[allow(clippy::arithmetic_side_effects)]
pub async fn run_migrations(
    pool: &Pool,
    migrations_table: &str,
    migrations: &[Vec<Migration>],
    renames: Option<&SchemaRenames<'_>>,
) -> Result<(), MysqlError> {
    let mut conn = pool
        .get_conn()
        .await
        .map_err(|e| MysqlError::Connection(e.to_string()))?;

    let lock_name = format!("migration_lock_{migrations_table}");

    // Acquire GET_LOCK on the session connection. Returns 1 if granted, 0 on
    // timeout, NULL on error. We hold this for the full migration sequence
    // (including the optional schema rename) and release it explicitly below.
    let acquired: Option<i64> = conn
        .exec_first(
            "SELECT GET_LOCK(?, ?)",
            (lock_name.clone(), MIGRATION_LOCK_TIMEOUT_SECS),
        )
        .await
        .map_err(|e| {
            MysqlError::Initialization(format!("Failed to acquire migration lock: {e}"))
        })?;
    if acquired != Some(1) {
        return Err(MysqlError::Initialization(format!(
            "Failed to acquire migration lock '{lock_name}' within {MIGRATION_LOCK_TIMEOUT_SECS}s"
        )));
    }

    let result = run_migrations_inner(&mut conn, migrations_table, migrations, renames).await;

    // Always release the lock, even on failure. Ignore release errors so we
    // don't mask the underlying migration error.
    let _ = conn.exec_drop("SELECT RELEASE_LOCK(?)", (lock_name,)).await;

    result
}

#[allow(clippy::arithmetic_side_effects)] // `i + 1` for migration version, bounded by Vec length
async fn run_migrations_inner(
    conn: &mut mysql_async::Conn,
    migrations_table: &str,
    migrations: &[Vec<Migration>],
    renames: Option<&SchemaRenames<'_>>,
) -> Result<(), MysqlError> {
    if let Some(renames) = renames {
        apply_renames(conn, renames).await?;
    }

    // Begin transaction. Migration table creation lives inside the transaction
    // for parity with the postgres impl, but note that DDL implicitly commits
    // in `MySQL` — this transaction only protects DML statements between DDL
    // boundaries. Idempotency is ensured by the structured `Migration`
    // variants, not by transactional rollback.
    conn.query_drop("START TRANSACTION")
        .await
        .map_err(map_db_error)?;

    // Create migrations table if it doesn't exist. `applied_at` is pinned to
    // UTC (rather than session-local) so the migration audit log is consistent
    // regardless of MySQL session TZ.
    let create_table_sql = format!(
        "CREATE TABLE IF NOT EXISTS `{migrations_table}` (
            version INT PRIMARY KEY,
            applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))
        )"
    );
    conn.query_drop(&create_table_sql)
        .await
        .map_err(map_db_error)?;

    // Get current version.
    let current_version: i32 = conn
        .query_first(format!(
            "SELECT COALESCE(MAX(version), 0) FROM `{migrations_table}`"
        ))
        .await
        .map_err(map_db_error)?
        .unwrap_or(0);

    for (i, migration) in migrations.iter().enumerate() {
        let version = i32::try_from(i + 1).unwrap_or(i32::MAX);
        if version > current_version {
            for step in migration {
                run_step(conn, version, step).await?;
            }
            let insert_sql = format!(
                "INSERT INTO `{migrations_table}` (version, applied_at) VALUES (?, UTC_TIMESTAMP(6))"
            );
            conn.exec_drop(&insert_sql, (version,))
                .await
                .map_err(map_db_error)?;
        }
    }

    conn.query_drop("COMMIT").await.map_err(map_db_error)?;

    Ok(())
}

/// Runs a single [`Migration`] step. Structured variants check
/// `information_schema` first and only emit the DDL when needed; the SQL
/// variant runs as-is.
async fn run_step(
    conn: &mut mysql_async::Conn,
    version: i32,
    step: &Migration,
) -> Result<(), MysqlError> {
    match step {
        Migration::Sql(sql) => conn
            .query_drop(sql.as_str())
            .await
            .map_err(|e| MysqlError::Database(format!("Migration {version} failed: {e}"))),

        Migration::AddColumn {
            table,
            column,
            definition,
        } => {
            if column_exists(conn, table, column).await? {
                return Ok(());
            }
            let sql = format!("ALTER TABLE `{table}` ADD COLUMN `{column}` {definition}");
            conn.query_drop(&sql).await.map_err(|e| {
                MysqlError::Database(format!(
                    "Migration {version} ADD COLUMN {table}.{column} failed: {e}"
                ))
            })
        }

        Migration::DropColumn { table, column } => {
            if !column_exists(conn, table, column).await? {
                return Ok(());
            }
            let sql = format!("ALTER TABLE `{table}` DROP COLUMN `{column}`");
            conn.query_drop(&sql).await.map_err(|e| {
                MysqlError::Database(format!(
                    "Migration {version} DROP COLUMN {table}.{column} failed: {e}"
                ))
            })
        }

        Migration::CreateIndex {
            name,
            table,
            columns,
        } => {
            if index_exists(conn, table, name).await? {
                return Ok(());
            }
            let sql = format!("CREATE INDEX `{name}` ON `{table}` {columns}");
            conn.query_drop(&sql).await.map_err(|e| {
                MysqlError::Database(format!(
                    "Migration {version} CREATE INDEX {name} on {table} failed: {e}"
                ))
            })
        }

        Migration::DropIndex { name, table } => {
            if !index_exists(conn, table, name).await? {
                return Ok(());
            }
            let sql = format!("DROP INDEX `{name}` ON `{table}`");
            conn.query_drop(&sql).await.map_err(|e| {
                MysqlError::Database(format!(
                    "Migration {version} DROP INDEX {name} on {table} failed: {e}"
                ))
            })
        }

        Migration::AddForeignKey {
            name,
            table,
            definition,
        } => {
            if foreign_key_exists(conn, table, name).await? {
                return Ok(());
            }
            let sql = format!("ALTER TABLE `{table}` ADD CONSTRAINT `{name}` {definition}");
            conn.query_drop(&sql).await.map_err(|e| {
                MysqlError::Database(format!(
                    "Migration {version} ADD CONSTRAINT {name} on {table} failed: {e}"
                ))
            })
        }

        Migration::DropForeignKey { name, table } => {
            if !foreign_key_exists(conn, table, name).await? {
                return Ok(());
            }
            let sql = format!("ALTER TABLE `{table}` DROP FOREIGN KEY `{name}`");
            conn.query_drop(&sql).await.map_err(|e| {
                MysqlError::Database(format!(
                    "Migration {version} DROP FOREIGN KEY {name} on {table} failed: {e}"
                ))
            })
        }

        Migration::DropPrimaryKey { table } => {
            if !primary_key_exists(conn, table).await? {
                return Ok(());
            }
            let sql = format!("ALTER TABLE `{table}` DROP PRIMARY KEY");
            conn.query_drop(&sql).await.map_err(|e| {
                MysqlError::Database(format!(
                    "Migration {version} DROP PRIMARY KEY on {table} failed: {e}"
                ))
            })
        }
    }
}

/// Checks `information_schema.columns` for a given column on the current schema.
async fn column_exists(
    conn: &mut mysql_async::Conn,
    table: &str,
    column: &str,
) -> Result<bool, MysqlError> {
    let count: Option<i64> = conn
        .exec_first(
            "SELECT COUNT(*) FROM information_schema.columns
             WHERE table_schema = DATABASE()
               AND table_name = ?
               AND column_name = ?",
            (table, column),
        )
        .await
        .map_err(map_db_error)?;
    Ok(count.unwrap_or(0) > 0)
}

/// Checks `information_schema.statistics` for a given index on the current schema.
async fn index_exists(
    conn: &mut mysql_async::Conn,
    table: &str,
    index_name: &str,
) -> Result<bool, MysqlError> {
    let count: Option<i64> = conn
        .exec_first(
            "SELECT COUNT(*) FROM information_schema.statistics
             WHERE table_schema = DATABASE()
               AND table_name = ?
               AND index_name = ?",
            (table, index_name),
        )
        .await
        .map_err(map_db_error)?;
    Ok(count.unwrap_or(0) > 0)
}

/// Checks `information_schema.table_constraints` for a given foreign-key
/// constraint on the current schema.
async fn foreign_key_exists(
    conn: &mut mysql_async::Conn,
    table: &str,
    fk_name: &str,
) -> Result<bool, MysqlError> {
    let count: Option<i64> = conn
        .exec_first(
            "SELECT COUNT(*) FROM information_schema.table_constraints
             WHERE table_schema = DATABASE()
               AND table_name = ?
               AND constraint_name = ?
               AND constraint_type = 'FOREIGN KEY'",
            (table, fk_name),
        )
        .await
        .map_err(map_db_error)?;
    Ok(count.unwrap_or(0) > 0)
}

/// Checks `information_schema.table_constraints` for a primary-key constraint
/// on the current schema. A PRIMARY KEY constraint is always named `PRIMARY`
/// in `MySQL`.
async fn primary_key_exists(conn: &mut mysql_async::Conn, table: &str) -> Result<bool, MysqlError> {
    let count: Option<i64> = conn
        .exec_first(
            "SELECT COUNT(*) FROM information_schema.table_constraints
             WHERE table_schema = DATABASE()
               AND table_name = ?
               AND constraint_type = 'PRIMARY KEY'",
            (table,),
        )
        .await
        .map_err(map_db_error)?;
    Ok(count.unwrap_or(0) > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MysqlStorageConfig;
    use crate::pool::create_pool;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::mysql::Mysql;

    /// Simulates a partially-applied migration: the DDL gets executed once but
    /// the version row never gets recorded (e.g. a crash between the DDL and
    /// the `INSERT INTO migrations`). On re-run, the structured variants must
    /// detect the existing state and no-op rather than abort.
    #[tokio::test]
    async fn test_partial_migration_replay_is_idempotent() {
        // Container must outlive the test — drop binds the lifetime to the
        // function scope while clippy is happy with a non-underscore name.
        let container: ContainerAsync<Mysql> = Mysql::default()
            .start()
            .await
            .expect("Failed to start `MySQL` container");
        let host_port = container
            .get_host_port_ipv4(3306)
            .await
            .expect("Failed to get host port");
        let pool = create_pool(&MysqlStorageConfig::with_defaults(format!(
            "mysql://root@127.0.0.1:{host_port}/test"
        )))
        .expect("Failed to create pool");

        // Pre-create the schema state that a previous half-finished migration
        // would have left behind: a base table plus an extra column / index.
        let mut conn = pool.get_conn().await.expect("get_conn");
        conn.query_drop("CREATE TABLE example (id VARCHAR(64) PRIMARY KEY, data JSON NOT NULL)")
            .await
            .expect("create base table");
        conn.query_drop("ALTER TABLE example ADD COLUMN value BIGINT NOT NULL DEFAULT 0")
            .await
            .expect("pre-add column");
        conn.query_drop("CREATE INDEX idx_example_value ON example(value)")
            .await
            .expect("pre-create index");
        drop(conn);

        // Now run the "full" migration set as if we are starting fresh: the
        // ADD COLUMN and CREATE INDEX statements must succeed despite the
        // objects already existing. The DROP at the end must also succeed
        // even though we never created `dropme`.
        let migrations: Vec<Vec<Migration>> = vec![
            vec![Migration::sql(
                "CREATE TABLE IF NOT EXISTS example (id VARCHAR(64) PRIMARY KEY, data JSON NOT NULL)",
            )],
            vec![
                Migration::AddColumn {
                    table: "example",
                    column: "value",
                    definition: "BIGINT NOT NULL DEFAULT 0",
                },
                Migration::CreateIndex {
                    name: "idx_example_value",
                    table: "example",
                    columns: "(value)",
                },
                Migration::DropColumn {
                    table: "example",
                    column: "dropme",
                },
            ],
        ];

        run_migrations(&pool, "test_schema_migrations", &migrations, None)
            .await
            .expect("re-run should be idempotent");

        // And re-running again must also succeed (no version-row regression).
        run_migrations(&pool, "test_schema_migrations", &migrations, None)
            .await
            .expect("second re-run should be a no-op");
    }

    /// End-to-end rename: tables, indexes, FK (drop+recreate), and
    /// migrations tracker all move from legacy to `brz_*` with seed data
    /// preserved.
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn test_bootstrap_rename_upgrades_legacy_schema() {
        let container: ContainerAsync<Mysql> = Mysql::default()
            .start()
            .await
            .expect("Failed to start `MySQL` container");
        let host_port = container
            .get_host_port_ipv4(3306)
            .await
            .expect("Failed to get host port");
        let pool = create_pool(&MysqlStorageConfig::with_defaults(format!(
            "mysql://root@127.0.0.1:{host_port}/test"
        )))
        .expect("Failed to create pool");

        // Pre-populate as a pre-prefix deployment would: legacy parent +
        // child with an explicit FK, an index on the child, and a
        // migrations tracker carrying a version row. Seed a data row that
        // must survive the rename.
        let mut conn = pool.get_conn().await.expect("get_conn");
        conn.query_drop(
            "CREATE TABLE widget_parents (
                id VARCHAR(64) NOT NULL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .await
        .expect("create parent");
        conn.query_drop(
            "CREATE TABLE widgets (
                id VARCHAR(64) NOT NULL PRIMARY KEY,
                parent_id VARCHAR(64) NOT NULL,
                CONSTRAINT fk_widgets_parent FOREIGN KEY (parent_id)
                    REFERENCES widget_parents(id)
            )",
        )
        .await
        .expect("create child");
        conn.query_drop("CREATE INDEX idx_widgets_parent ON widgets(parent_id)")
            .await
            .expect("create index");
        conn.query_drop("INSERT INTO widget_parents (id, name) VALUES ('p1', 'alpha')")
            .await
            .expect("seed parent");
        conn.query_drop("INSERT INTO widgets (id, parent_id) VALUES ('w1', 'p1')")
            .await
            .expect("seed child");
        conn.query_drop(
            "CREATE TABLE legacy_widget_migrations (
                version INT PRIMARY KEY,
                applied_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
            )",
        )
        .await
        .expect("create legacy migrations");
        conn.query_drop("INSERT INTO legacy_widget_migrations (version) VALUES (1)")
            .await
            .expect("seed version row");
        drop(conn);

        let renames = SchemaRenames {
            old_migrations_table: "legacy_widget_migrations",
            new_migrations_table: "brz_widget_migrations",
            tables: &[
                ("widget_parents", "brz_widget_parents"),
                ("widgets", "brz_widgets"),
            ],
            indexes: &[(
                "brz_widgets",
                "idx_widgets_parent",
                "brz_idx_widgets_parent",
            )],
            foreign_keys: &[FkRename {
                table: "brz_widgets",
                old_name: "fk_widgets_parent",
                new_name: "brz_fk_widgets_parent",
                definition: "FOREIGN KEY (parent_id) REFERENCES `brz_widget_parents`(id)",
            }],
        };

        // run_migrations with an empty migration list — we're only exercising
        // the rename block.
        run_migrations(&pool, "brz_widget_migrations", &[], Some(&renames))
            .await
            .expect("rename should succeed");

        let mut conn = pool.get_conn().await.expect("get_conn");

        let new_table_count: Option<i64> = conn
            .exec_first(
                "SELECT COUNT(*) FROM information_schema.tables
                 WHERE table_schema = DATABASE() AND table_name = 'brz_widgets'",
                (),
            )
            .await
            .expect("probe brz_widgets");
        assert_eq!(new_table_count, Some(1), "brz_widgets must exist");

        let old_table_count: Option<i64> = conn
            .exec_first(
                "SELECT COUNT(*) FROM information_schema.tables
                 WHERE table_schema = DATABASE() AND table_name = 'widgets'",
                (),
            )
            .await
            .expect("probe widgets");
        assert_eq!(old_table_count, Some(0), "legacy widgets must be gone");

        let row: Option<(String, String)> = conn
            .exec_first("SELECT id, parent_id FROM brz_widgets WHERE id = 'w1'", ())
            .await
            .expect("seed row preserved");
        assert_eq!(
            row,
            Some(("w1".to_string(), "p1".to_string())),
            "seed data must survive the rename"
        );

        let new_index_count: Option<i64> = conn
            .exec_first(
                "SELECT COUNT(*) FROM information_schema.statistics
                 WHERE table_schema = DATABASE()
                   AND table_name = 'brz_widgets'
                   AND index_name = 'brz_idx_widgets_parent'",
                (),
            )
            .await
            .expect("probe brz index");
        assert_eq!(new_index_count, Some(1), "index must be renamed");

        let new_fk_count: Option<i64> = conn
            .exec_first(
                "SELECT COUNT(*) FROM information_schema.table_constraints
                 WHERE table_schema = DATABASE()
                   AND table_name = 'brz_widgets'
                   AND constraint_type = 'FOREIGN KEY'
                   AND constraint_name = 'brz_fk_widgets_parent'",
                (),
            )
            .await
            .expect("probe brz fk");
        assert_eq!(new_fk_count, Some(1), "FK must be renamed");

        // The renamed FK must still functionally enforce referential
        // integrity — inserting a row whose parent_id has no matching
        // parent row should fail. Catches the case where the rename
        // produced a constraint by name but lost its referent (e.g. wrong
        // column or wrong parent in the combined ALTER).
        let violation = conn
            .query_drop(
                "INSERT INTO brz_widgets (id, parent_id) VALUES ('w_orphan', 'missing_parent')",
            )
            .await;
        assert!(
            violation.is_err(),
            "renamed FK must still reject orphan parent_id"
        );

        let version: Option<i32> = conn
            .exec_first("SELECT MAX(version) FROM brz_widget_migrations", ())
            .await
            .expect("probe canary table");
        assert_eq!(version, Some(1), "version row must survive");

        drop(conn);

        // Re-running must be a no-op (canary is gone now, returns early).
        run_migrations(&pool, "brz_widget_migrations", &[], Some(&renames))
            .await
            .expect("re-run should be idempotent");
    }

    /// Rename must skip listed objects that don't exist (partial pre-prefix
    /// schema, e.g. a customer at an older migration version).
    #[tokio::test]
    async fn test_bootstrap_rename_tolerates_missing_objects() {
        let container: ContainerAsync<Mysql> = Mysql::default()
            .start()
            .await
            .expect("Failed to start MySQL container");
        let host_port = container
            .get_host_port_ipv4(3306)
            .await
            .expect("Failed to get host port");
        let pool = create_pool(&MysqlStorageConfig::with_defaults(format!(
            "mysql://root@127.0.0.1:{host_port}/test"
        )))
        .expect("Failed to create pool");

        // Pre-populate the canary + a bare table — no index, no FK. The
        // rename map below lists objects that don't exist.
        let mut conn = pool.get_conn().await.expect("get_conn");
        conn.query_drop(
            "CREATE TABLE widgets (
                id VARCHAR(64) NOT NULL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .await
        .expect("create table");
        conn.query_drop("INSERT INTO widgets (id, name) VALUES ('w1', 'alpha')")
            .await
            .expect("seed row");
        conn.query_drop(
            "CREATE TABLE legacy_widget_migrations (
                version INT PRIMARY KEY,
                applied_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
            )",
        )
        .await
        .expect("create canary");
        conn.query_drop("INSERT INTO legacy_widget_migrations (version) VALUES (1)")
            .await
            .expect("seed version row");
        drop(conn);

        let renames = SchemaRenames {
            old_migrations_table: "legacy_widget_migrations",
            new_migrations_table: "brz_widget_migrations",
            tables: &[("widgets", "brz_widgets")],
            indexes: &[(
                "brz_widgets",
                "idx_widgets_missing",
                "brz_idx_widgets_missing",
            )],
            foreign_keys: &[FkRename {
                table: "brz_widgets",
                old_name: "fk_widgets_missing",
                new_name: "brz_fk_widgets_missing",
                definition: "FOREIGN KEY (id) REFERENCES brz_widgets(id)",
            }],
        };

        run_migrations(&pool, "brz_widget_migrations", &[], Some(&renames))
            .await
            .expect("rename must skip missing objects without erroring");

        let mut conn = pool.get_conn().await.expect("get_conn");
        let row: Option<(String, String)> = conn
            .exec_first("SELECT id, name FROM brz_widgets WHERE id = 'w1'", ())
            .await
            .expect("probe brz_widgets");
        assert_eq!(
            row,
            Some(("w1".to_string(), "alpha".to_string())),
            "table + data preserved despite missing index/FK"
        );
    }
}
