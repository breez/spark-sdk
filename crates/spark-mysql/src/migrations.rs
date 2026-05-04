//! Generic `MySQL` migration runner with version tracking and concurrency control.
//!
//! Uses `GET_LOCK`/`RELEASE_LOCK` (`MySQL` named locks) to serialize migrations
//! across concurrent connections. Named locks are session-scoped (not
//! transaction-scoped like Postgres `pg_advisory_xact_lock`), so the lock is
//! acquired before the transaction and explicitly released after commit /
//! rollback. The connection is held for the entire migration sequence so the
//! session — and therefore the lock — stays bound to a single client.

use mysql_async::Pool;
use mysql_async::prelude::*;

use crate::error::MysqlError;
use crate::pool::map_db_error;

/// Timeout (seconds) when waiting for the migration `GET_LOCK`.
const MIGRATION_LOCK_TIMEOUT_SECS: i64 = 60;

/// Runs database migrations with version tracking and concurrency control.
///
/// This function:
/// - Acquires a named lock (derived from the migrations table name) to prevent concurrent migrations
/// - Creates a migrations tracking table if it doesn't exist
/// - Applies only new migrations (based on version number)
/// - Commits all changes in a single transaction
/// - Releases the lock before returning
///
/// # Arguments
/// * `pool` - The connection pool to use
/// * `migrations_table` - Name of the table to track migration versions (e.g., `schema_migrations`)
/// * `migrations` - List of migrations, where each migration is a list of SQL statements
#[allow(clippy::arithmetic_side_effects)]
pub async fn run_migrations(
    pool: &Pool,
    migrations_table: &str,
    migrations: &[&[&str]],
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
    migrations: &[&[&str]],
) -> Result<(), MysqlError> {
    // Begin transaction. Migration table creation lives inside the transaction
    // for parity with the postgres impl (DDL inside a txn is supported on InnoDB
    // but not transactional — that's fine because we still gate on GET_LOCK).
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
            for statement in *migration {
                conn.query_drop(*statement).await.map_err(|e| {
                    MysqlError::Database(format!("Migration {version} failed: {e}"))
                })?;
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
