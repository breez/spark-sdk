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
#[derive(Clone, Copy, Debug)]
pub enum Migration {
    /// Run the SQL statement as-is. Errors propagate.
    Sql(&'static str),

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
}

/// Runs database migrations with version tracking and concurrency control.
///
/// This function:
/// - Acquires a named lock (derived from the migrations table name) to prevent concurrent migrations
/// - Creates a migrations tracking table if it doesn't exist
/// - Applies only new migrations (based on version number)
/// - Releases the lock before returning
///
/// # Arguments
/// * `pool` - The connection pool to use
/// * `migrations_table` - Name of the table to track migration versions (e.g., `schema_migrations`)
/// * `migrations` - List of migrations, where each migration is a list of [`Migration`] steps
#[allow(clippy::arithmetic_side_effects)]
pub async fn run_migrations(
    pool: &Pool,
    migrations_table: &str,
    migrations: &[&[Migration]],
) -> Result<(), MysqlError> {
    let mut conn = pool
        .get_conn()
        .await
        .map_err(|e| MysqlError::Connection(e.to_string()))?;

    let lock_name = format!("migration_lock_{migrations_table}");

    // Acquire GET_LOCK on the session connection. Returns 1 if granted, 0 on
    // timeout, NULL on error. We hold this for the full migration sequence and
    // release it explicitly below.
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

    let result = run_migrations_inner(&mut conn, migrations_table, migrations).await;

    // Always release the lock, even on failure. Ignore release errors so we
    // don't mask the underlying migration error.
    let _ = conn.exec_drop("SELECT RELEASE_LOCK(?)", (lock_name,)).await;

    result
}

#[allow(clippy::arithmetic_side_effects)] // `i + 1` for migration version, bounded by Vec length
async fn run_migrations_inner(
    conn: &mut mysql_async::Conn,
    migrations_table: &str,
    migrations: &[&[Migration]],
) -> Result<(), MysqlError> {
    // Begin transaction. Migration table creation lives inside the transaction
    // for parity with the postgres impl, but note that DDL implicitly commits
    // in MySQL — this transaction only protects DML statements between DDL
    // boundaries. Idempotency is ensured by the structured `Migration`
    // variants, not by transactional rollback.
    conn.query_drop("START TRANSACTION")
        .await
        .map_err(map_db_error)?;

    // Create migrations table if it doesn't exist.
    let create_table_sql = format!(
        "CREATE TABLE IF NOT EXISTS `{migrations_table}` (
            version INT PRIMARY KEY,
            applied_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
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
            for step in *migration {
                run_step(conn, version, *step).await?;
            }
            let insert_sql = format!("INSERT INTO `{migrations_table}` (version) VALUES (?)");
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
    step: Migration,
) -> Result<(), MysqlError> {
    match step {
        Migration::Sql(sql) => conn
            .query_drop(sql)
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
            .expect("Failed to start MySQL container");
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
        let migrations: Vec<&[Migration]> = vec![
            &[Migration::Sql(
                "CREATE TABLE IF NOT EXISTS example (id VARCHAR(64) PRIMARY KEY, data JSON NOT NULL)",
            )],
            &[
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

        run_migrations(&pool, "test_schema_migrations", &migrations)
            .await
            .expect("re-run should be idempotent");

        // And re-running again must also succeed (no version-row regression).
        run_migrations(&pool, "test_schema_migrations", &migrations)
            .await
            .expect("second re-run should be a no-op");
    }
}
