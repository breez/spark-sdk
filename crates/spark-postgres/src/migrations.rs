//! Generic `PostgreSQL` migration runner with version tracking and concurrency control.

use deadpool_postgres::Pool;
use tokio_postgres::Transaction;

use crate::error::PostgresError;
use crate::pool::{map_db_error, map_pool_error};

/// One-shot rename map passed to [`run_migrations`] when upgrading an
/// existing deployment to a renamed schema. Describes how to transition
/// every Breez-owned schema object — tables, indexes, constraints, plus the
/// per-store schema migrations tracking table itself — from its old name to
/// its new name. The original use case is applying a `brz_` namespace
/// prefix.
///
/// `old_migrations_table` doubles as the **canary**: the runner probes
/// `information_schema.tables` for it; if missing, the DB is either fresh
/// or already upgraded and the rename block is skipped.
pub struct SchemaRenames<'a> {
    /// Old name of the per-store schema migrations table; used as the canary
    /// check.
    pub old_migrations_table: &'a str,
    /// New name of the per-store schema migrations table. Must match the
    /// `migrations_table` argument to [`run_migrations`].
    pub new_migrations_table: &'a str,
    /// `(old_table, new_table)` pairs.
    pub tables: &'a [(&'a str, &'a str)],
    /// `(old_index, new_index)` pairs.
    pub indexes: &'a [(&'a str, &'a str)],
    /// `(parent_table_new_name, old_constraint, new_constraint)` triples.
    /// The table reference uses the **renamed** table name because
    /// constraints are renamed after their parent table.
    pub constraints: &'a [(&'a str, &'a str, &'a str)],
}

/// Runs database migrations with version tracking and concurrency control.
///
/// This function:
/// - Acquires an advisory lock (derived from `migration_lock_{table_name}`) to prevent concurrent migrations
/// - Optionally applies a one-shot schema rename before running migrations
///   (see [`SchemaRenames`]) — useful when upgrading an existing deployment
///   whose tables were created under different names
/// - Creates the migrations tracking table if it doesn't exist
/// - Applies only new migrations (based on version number)
/// - Commits all changes in a single transaction
///
/// The rename block and the version-tracked migrations run under the same
/// `pg_advisory_xact_lock`, so the whole schema transition is one critical
/// section.
///
/// # Arguments
/// * `pool` - The connection pool to use
/// * `migrations_table` - Name of the table to track migration versions (e.g., `schema_migrations`)
/// * `migrations` - List of migrations, where each migration is a list of SQL statements.
///   Statements are owned `String`s so callers can build them at runtime (e.g. inlining a
///   tenant identity into a backfill).
/// * `renames` - Optional one-shot rename map for upgrading from an
///   unprefixed schema. When `Some`, the runner probes the canary table and,
///   if present, renames everything before processing version-tracked
///   migrations. When `None`, no rename is attempted.
#[allow(clippy::arithmetic_side_effects)]
pub async fn run_migrations(
    pool: &Pool,
    migrations_table: &str,
    migrations: &[Vec<String>],
    renames: Option<&SchemaRenames<'_>>,
) -> Result<(), PostgresError> {
    let mut client = pool.get().await.map_err(map_pool_error)?;

    // Generate a unique advisory lock ID from a descriptive lock name
    let lock_name = format!("migration_lock_{migrations_table}");
    let lock_id: i64 = lock_name.bytes().map(i64::from).sum();

    // Run renames + version-tracked migrations under one transaction-level
    // advisory lock. pg_advisory_xact_lock auto-releases on commit/rollback,
    // making it safe with connection pools (no risk of leaked locks if the
    // task is cancelled or panics).
    let tx = client.transaction().await.map_err(map_db_error)?;

    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&lock_id])
        .await
        .map_err(|e| {
            PostgresError::Initialization(format!("Failed to acquire migration lock: {e}"))
        })?;

    if let Some(renames) = renames {
        apply_renames_in_tx(&tx, renames).await?;
    }

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
            for statement in migration {
                tx.execute(statement.as_str(), &[]).await.map_err(|e| {
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

/// Applies a one-shot schema rename inside the caller's transaction. Gated
/// on the canary check against `renames.old_migrations_table`: if that
/// table doesn't exist, returns early (the DB is fresh or already upgraded).
async fn apply_renames_in_tx(
    tx: &Transaction<'_>,
    renames: &SchemaRenames<'_>,
) -> Result<(), PostgresError> {
    let canary_sql = "SELECT EXISTS (
        SELECT 1 FROM information_schema.tables
        WHERE table_schema = current_schema() AND table_name = $1
    )";
    let old_exists: bool = tx
        .query_one(canary_sql, &[&renames.old_migrations_table])
        .await
        .map_err(map_db_error)?
        .get(0);

    if !old_exists {
        return Ok(());
    }

    // Order matters only for readability — Postgres wraps the whole sequence
    // in one transaction so a crash rolls everything back.
    for (old, new) in renames.tables {
        let sql = format!("ALTER TABLE {old} RENAME TO {new}");
        tx.execute(&sql, &[]).await.map_err(|e| {
            PostgresError::Database(format!("Failed to rename table {old} -> {new}: {e}"))
        })?;
    }

    for (old, new) in renames.indexes {
        let sql = format!("ALTER INDEX {old} RENAME TO {new}");
        tx.execute(&sql, &[]).await.map_err(|e| {
            PostgresError::Database(format!("Failed to rename index {old} -> {new}: {e}"))
        })?;
    }

    for (table, old, new) in renames.constraints {
        let sql = format!("ALTER TABLE {table} RENAME CONSTRAINT {old} TO {new}");
        tx.execute(&sql, &[]).await.map_err(|e| {
            PostgresError::Database(format!(
                "Failed to rename constraint {old} -> {new} on {table}: {e}"
            ))
        })?;
    }

    // Rename the migrations tracking table last; it doubles as the canary
    // signaling that the rename is complete.
    let sql = format!(
        "ALTER TABLE {} RENAME TO {}",
        renames.old_migrations_table, renames.new_migrations_table
    );
    tx.execute(&sql, &[]).await.map_err(map_db_error)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PostgresStorageConfig;
    use crate::pool::create_pool;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::postgres::Postgres;

    /// Verifies that an existing deployment with the legacy unprefixed schema
    /// gets upgraded to `brz_`-prefixed names without data loss when
    /// `run_migrations` is called with `Some(&renames)`. Exercises the full
    /// rename surface: tables, indexes, named PK constraint, plus the
    /// migrations tracking table that doubles as the canary.
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn test_bootstrap_rename_upgrades_legacy_schema() {
        let container: ContainerAsync<Postgres> = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");
        let pool = create_pool(&PostgresStorageConfig::with_defaults(format!(
            "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
        )))
        .expect("Failed to create pool");

        // Pre-populate the schema as a pre-prefix deployment would have it:
        // legacy table + legacy index + legacy migrations tracker with a
        // recorded version, all unprefixed. Seed a data row that must survive
        // the rename.
        {
            let client = pool.get().await.expect("get_conn");
            client
                .batch_execute(
                    "CREATE TABLE widgets (id TEXT PRIMARY KEY, name TEXT NOT NULL);
                     CREATE INDEX idx_widgets_name ON widgets(name);
                     INSERT INTO widgets (id, name) VALUES ('w1', 'alpha');
                     CREATE TABLE legacy_widget_migrations (
                         version INTEGER PRIMARY KEY,
                         applied_at TIMESTAMPTZ DEFAULT NOW()
                     );
                     INSERT INTO legacy_widget_migrations (version) VALUES (1);",
                )
                .await
                .expect("seed legacy schema");
        }

        let renames = SchemaRenames {
            old_migrations_table: "legacy_widget_migrations",
            new_migrations_table: "brz_widget_migrations",
            tables: &[("widgets", "brz_widgets")],
            indexes: &[("idx_widgets_name", "brz_idx_widgets_name")],
            constraints: &[("brz_widgets", "widgets_pkey", "brz_widgets_pkey")],
        };

        // run_migrations with an empty migration list — we're only exercising
        // the rename block and the migrations-table-version round-trip.
        run_migrations(&pool, "brz_widget_migrations", &[], Some(&renames))
            .await
            .expect("rename should succeed");

        let client = pool.get().await.expect("get_conn");

        let new_table_exists: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables
                                WHERE table_schema = current_schema()
                                  AND table_name = 'brz_widgets')",
                &[],
            )
            .await
            .expect("probe brz_widgets")
            .get(0);
        assert!(new_table_exists, "brz_widgets must exist after rename");

        let old_table_exists: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables
                                WHERE table_schema = current_schema()
                                  AND table_name = 'widgets')",
                &[],
            )
            .await
            .expect("probe widgets")
            .get(0);
        assert!(!old_table_exists, "legacy widgets table must be gone");

        let row = client
            .query_one("SELECT id, name FROM brz_widgets WHERE id = 'w1'", &[])
            .await
            .expect("seed row preserved");
        let id: String = row.get(0);
        let name: String = row.get(1);
        assert_eq!(id, "w1");
        assert_eq!(name, "alpha", "seed data must survive the rename");

        let new_index_exists: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_indexes
                                WHERE schemaname = current_schema()
                                  AND indexname = 'brz_idx_widgets_name')",
                &[],
            )
            .await
            .expect("probe brz index")
            .get(0);
        assert!(new_index_exists, "brz_idx_widgets_name must exist");

        let new_pk_exists: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.table_constraints
                                WHERE table_schema = current_schema()
                                  AND table_name = 'brz_widgets'
                                  AND constraint_name = 'brz_widgets_pkey')",
                &[],
            )
            .await
            .expect("probe brz pk")
            .get(0);
        assert!(new_pk_exists, "PK constraint must be renamed");

        let version: i32 = client
            .query_one("SELECT MAX(version) FROM brz_widget_migrations", &[])
            .await
            .expect("probe canary table")
            .get(0);
        assert_eq!(version, 1, "migrations tracker version row must survive");

        // Re-running must be a no-op (canary is gone now, returns early).
        run_migrations(&pool, "brz_widget_migrations", &[], Some(&renames))
            .await
            .expect("re-run should be idempotent");
    }
}
