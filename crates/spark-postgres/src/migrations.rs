//! Generic `PostgreSQL` migration runner with version tracking and concurrency control.

use deadpool_postgres::Pool;

use crate::error::PostgresError;
use crate::pool::{map_db_error, map_pool_error};

/// Runs database migrations with version tracking and concurrency control.
///
/// This function:
/// - Acquires an advisory lock (derived from `migration_lock_{table_name}`) to prevent concurrent migrations
/// - Creates a migrations tracking table if it doesn't exist
/// - Applies only new migrations (based on version number)
/// - Commits all changes in a single transaction
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
) -> Result<(), PostgresError> {
    let mut client = pool.get().await.map_err(map_pool_error)?;

    // Generate a unique advisory lock ID from a descriptive lock name
    let lock_name = format!("migration_lock_{migrations_table}");
    let lock_id: i64 = lock_name.bytes().map(i64::from).sum();

    // Run all migrations in a single transaction with a transaction-level advisory lock.
    // pg_advisory_xact_lock is automatically released on commit/rollback, making it safe
    // with connection pools (no risk of leaked locks if the task is cancelled or panics).
    let tx = client.transaction().await.map_err(map_db_error)?;

    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&lock_id])
        .await
        .map_err(|e| {
            PostgresError::Initialization(format!("Failed to acquire migration lock: {e}"))
        })?;

    // Create migrations table if it doesn't exist
    // Note: table names cannot be parameterized in PostgreSQL, so we use format!
    let create_table_sql = format!(
        "CREATE TABLE IF NOT EXISTS {migrations_table} (
            version INTEGER PRIMARY KEY,
            applied_at TIMESTAMPTZ DEFAULT NOW()
        )"
    );
    tx.execute(&create_table_sql, &[])
        .await
        .map_err(map_db_error)?;

    // Get current version
    let get_version_sql = format!("SELECT COALESCE(MAX(version), 0) FROM {migrations_table}");
    let current_version: i32 = tx
        .query_opt(&get_version_sql, &[])
        .await
        .map_err(map_db_error)?
        .map_or(0, |row| row.get(0));

    for (i, migration) in migrations.iter().enumerate() {
        let version = i32::try_from(i + 1).unwrap_or(i32::MAX);
        if version > current_version {
            for statement in *migration {
                tx.execute(*statement, &[]).await.map_err(|e| {
                    PostgresError::Database(format!("Migration {version} failed: {e}"))
                })?;
            }
            let insert_version_sql =
                format!("INSERT INTO {migrations_table} (version) VALUES ($1)");
            tx.execute(&insert_version_sql, &[&version])
                .await
                .map_err(map_db_error)?;
        }
    }

    tx.commit().await.map_err(map_db_error)?;

    Ok(())
}
