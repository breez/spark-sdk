//! Schema migrations for the LNURL server database.

use spark_postgres::{PostgresError, deadpool_postgres::Pool, map_db_error, map_pool_error};

/// Tracker table for applied migrations. Prefixed so a database shared with
/// other Breez components gets a distinct table and a distinct migration
/// advisory lock, which `run_migrations` derives from this name.
const MIGRATIONS_TABLE: &str = "lnurl_schema_migrations";

/// Advisory lock guarding the one-shot adoption of the previous tracker.
/// Serializes instances starting against the same database so the `CREATE
/// TABLE` and the version inserts cannot interleave.
const ADOPT_LOCK_ID: i64 = 0x6c6e_7572_6c5f_6164; // "lnurl_ad"

/// Runs any migrations the database is missing.
pub async fn run(pool: &Pool) -> Result<(), PostgresError> {
    adopt_applied_prefix(pool).await?;
    spark_postgres::run_migrations(pool, MIGRATIONS_TABLE, &migrations(), None).await
}

/// Resumes a database migrated by the previous runner, which tracked applied
/// migrations in `_sqlx_migrations` keyed by file version instead of by the
/// ordinal used here. It applied them in order and stopped at the first
/// failure, so its successful rows are a prefix of [`migrations`]: recording
/// that many ordinals picks up where it left off rather than re-running from
/// migration 1. No-op unless `_sqlx_migrations` is present, which costs a fresh
/// database a single probe.
async fn adopt_applied_prefix(pool: &Pool) -> Result<(), PostgresError> {
    let mut client = pool.get().await.map_err(map_pool_error)?;
    let tx = client.transaction().await.map_err(map_db_error)?;

    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&ADOPT_LOCK_ID])
        .await
        .map_err(|e| PostgresError::Initialization(format!("Failed to acquire adopt lock: {e}")))?;

    let previous_tracker_exists: bool = tx
        .query_one(
            "SELECT EXISTS (
                 SELECT 1 FROM information_schema.tables
                 WHERE table_schema = current_schema() AND table_name = '_sqlx_migrations'
             )",
            &[],
        )
        .await
        .map_err(map_db_error)?
        .get(0);

    if !previous_tracker_exists {
        return Ok(());
    }

    tx.batch_execute(&format!(
        "CREATE TABLE IF NOT EXISTS {MIGRATIONS_TABLE} (
             version INTEGER PRIMARY KEY,
             applied_at TIMESTAMPTZ DEFAULT NOW()
         );
         INSERT INTO {MIGRATIONS_TABLE} (version)
         SELECT generate_series(
             1,
             (SELECT COUNT(*) FROM _sqlx_migrations WHERE success)::int
         )
         ON CONFLICT (version) DO NOTHING;"
    ))
    .await
    .map_err(map_db_error)?;

    tx.commit().await.map_err(map_db_error)?;
    Ok(())
}

/// The migrations, in the order the previous runner applied them: ascending
/// numeric file version, which put every 14-digit version after every 8-digit
/// one. Position is the tracked version, so entries are only ever appended.
#[allow(clippy::too_many_lines)]
fn migrations() -> Vec<Vec<String>> {
    vec![
        vec![
            "CREATE TABLE users (
                    domain VARCHAR(255) NOT NULL,
                    pubkey VARCHAR(66) NOT NULL,
                    name VARCHAR(64) NOT NULL,
                    description VARCHAR(255) NOT NULL,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (domain, pubkey),
                    UNIQUE (domain, name)
                )"
            .to_string(),
        ],
        vec![
            "CREATE TABLE zaps (
                    payment_hash   VARCHAR(64) NOT NULL PRIMARY KEY,
                    zap_request    TEXT        NOT NULL,
                    zap_event      TEXT,
                    user_pubkey    VARCHAR(66) NOT NULL,
                    invoice_expiry BIGINT      NOT NULL
                )"
            .to_string(),
            "CREATE INDEX idx_zaps_user_pubkey ON zaps(user_pubkey)".to_string(),
            "CREATE INDEX idx_zaps_invoice_expiry ON zaps(invoice_expiry)".to_string(),
        ],
        vec![
            "CREATE TABLE sender_comments (
                    payment_hash   VARCHAR(64)  NOT NULL PRIMARY KEY,
                    user_pubkey    VARCHAR(66)  NOT NULL,
                    sender_comment VARCHAR(255) NOT NULL,
                    updated_at     BIGINT       NOT NULL
                )"
            .to_string(),
            "CREATE INDEX idx_sender_comments_user_pubkey ON sender_comments(user_pubkey)"
                .to_string(),
            "CREATE INDEX idx_sender_comments_updated_at ON sender_comments(updated_at)"
                .to_string(),
            "ALTER TABLE zaps ADD COLUMN updated_at BIGINT NOT NULL
                 DEFAULT round(extract(epoch from now()) * 1000)"
                .to_string(),
            "CREATE INDEX idx_zaps_updated_at ON zaps(updated_at)".to_string(),
            "ALTER TABLE users ADD COLUMN nostr_pubkey VARCHAR(66)".to_string(),
        ],
        vec![
            "ALTER TABLE zaps ADD COLUMN is_user_nostr_key BOOLEAN NOT NULL DEFAULT FALSE"
                .to_string(),
        ],
        vec![
            "CREATE TABLE allowed_domains (
                    domain VARCHAR(255) PRIMARY KEY
                )"
            .to_string(),
        ],
        vec![
            "CREATE TABLE invoices (
                    payment_hash   VARCHAR(64) PRIMARY KEY,
                    user_pubkey    VARCHAR(66) NOT NULL,
                    invoice        TEXT        NOT NULL,
                    preimage       VARCHAR(64),
                    invoice_expiry BIGINT      NOT NULL,
                    created_at     BIGINT      NOT NULL,
                    updated_at     BIGINT      NOT NULL
                )"
            .to_string(),
            "CREATE INDEX idx_invoices_user_pubkey ON invoices(user_pubkey)".to_string(),
            "CREATE INDEX idx_invoices_invoice_expiry ON invoices(invoice_expiry)".to_string(),
            "CREATE INDEX idx_invoices_updated_at ON invoices(updated_at)".to_string(),
            "CREATE TABLE newly_paid (
                    payment_hash  VARCHAR(64) PRIMARY KEY,
                    created_at    BIGINT      NOT NULL,
                    retry_count   INTEGER     NOT NULL DEFAULT 0,
                    next_retry_at BIGINT      NOT NULL
                )"
            .to_string(),
            "CREATE INDEX idx_newly_paid_next_retry_at ON newly_paid(next_retry_at)".to_string(),
            "ALTER TABLE users
                 ADD COLUMN lnurl_private_mode_enabled BOOLEAN NOT NULL DEFAULT FALSE"
                .to_string(),
        ],
        vec![
            "CREATE INDEX idx_invoices_user_pubkey_updated_at
                 ON invoices(user_pubkey, updated_at)"
                .to_string(),
            "CREATE INDEX idx_zaps_user_pubkey_updated_at ON zaps(user_pubkey, updated_at)"
                .to_string(),
            "CREATE INDEX idx_sender_comments_user_pubkey_updated_at
                 ON sender_comments(user_pubkey, updated_at)"
                .to_string(),
        ],
        vec!["ALTER TABLE users ADD COLUMN webhook_secret TEXT".to_string()],
        vec![
            "CREATE TABLE IF NOT EXISTS settings (
                    key   TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                )"
            .to_string(),
            "ALTER TABLE newly_paid ADD COLUMN claimed_at BIGINT".to_string(),
        ],
        vec![
            "ALTER TABLE users DROP COLUMN lnurl_private_mode_enabled".to_string(),
            "ALTER TABLE users DROP COLUMN nostr_pubkey".to_string(),
            "ALTER TABLE users DROP COLUMN webhook_secret".to_string(),
        ],
        vec!["CREATE INDEX idx_users_pubkey ON users(pubkey)".to_string()],
        vec![
            // This table specifically tracks zap receipt publishing, not
            // generic payment events.
            "ALTER TABLE newly_paid RENAME TO pending_zap_receipts".to_string(),
            "ALTER INDEX idx_newly_paid_next_retry_at
                 RENAME TO idx_pending_zap_receipts_next_retry_at"
                .to_string(),
        ],
        vec![
            "ALTER TABLE invoices ADD COLUMN domain VARCHAR(255)".to_string(),
            "ALTER TABLE invoices ADD COLUMN amount_received_sat BIGINT".to_string(),
        ],
        vec![
            // Deliveries store the complete payload so the receiving domain
            // is self-contained and never queries LNURL tables.
            "CREATE TABLE domain_webhooks (
                    domain VARCHAR(255) PRIMARY KEY,
                    url    TEXT NOT NULL
                )"
            .to_string(),
            "CREATE TABLE webhook_deliveries (
                    id                     BIGSERIAL PRIMARY KEY,
                    identifier             TEXT      NOT NULL,
                    url                    TEXT      NOT NULL,
                    payload                TEXT      NOT NULL,
                    created_at             BIGINT    NOT NULL,
                    succeeded_at           BIGINT,
                    retry_count            INTEGER   NOT NULL DEFAULT 0,
                    next_retry_at          BIGINT    NOT NULL,
                    claimed_at             BIGINT,
                    last_error_status_code INTEGER,
                    last_error_body        TEXT,
                    UNIQUE (identifier, url)
                )"
            .to_string(),
            "CREATE INDEX idx_webhook_deliveries_pending
                 ON webhook_deliveries (url, next_retry_at)
                 WHERE succeeded_at IS NULL"
                .to_string(),
            "CREATE INDEX idx_webhook_deliveries_created_at
                 ON webhook_deliveries (created_at)"
                .to_string(),
        ],
        vec![
            "ALTER TABLE domain_webhooks ADD COLUMN webhook_secret TEXT NOT NULL".to_string(),
            // Deliveries route by domain; the URL is resolved at send time
            // from domain_webhooks and stored for audit.
            "ALTER TABLE webhook_deliveries ADD COLUMN domain TEXT NOT NULL DEFAULT 'unknown'"
                .to_string(),
            "ALTER TABLE webhook_deliveries ALTER COLUMN url DROP NOT NULL".to_string(),
            "ALTER TABLE webhook_deliveries
                 DROP CONSTRAINT webhook_deliveries_identifier_url_key"
                .to_string(),
            "CREATE UNIQUE INDEX idx_webhook_deliveries_identifier_domain
                 ON webhook_deliveries (identifier, domain)"
                .to_string(),
            "DROP INDEX IF EXISTS idx_webhook_deliveries_pending".to_string(),
            "CREATE INDEX idx_webhook_deliveries_pending
                 ON webhook_deliveries (domain, next_retry_at)
                 WHERE succeeded_at IS NULL"
                .to_string(),
        ],
        vec![
            // api_key is the domain's admin-set Breez API key, exchanged for
            // a partner JWT. jwt caches that token so restarts and sibling
            // instances start warm instead of re-fetching.
            "CREATE TABLE domain_attribution (
                    domain  VARCHAR(255) PRIMARY KEY,
                    api_key TEXT NOT NULL,
                    jwt     TEXT
                )"
            .to_string(),
        ],
    ]
}

#[cfg(test)]
mod tests {
    use spark_postgres::deadpool_postgres::{Client, Pool};

    use super::{MIGRATIONS_TABLE, migrations, run};
    use crate::test_support::empty_test_pool;

    /// Brings a schema to the state the previous runner left it in after
    /// applying the first `count` migrations: the statements themselves, plus
    /// its tracker table. It keyed rows by file version; only how many of them
    /// succeeded is read back, so the values just have to be distinct.
    async fn migrate_as_previous_runner(pool: &Pool, count: usize) {
        let client = pool.get().await.unwrap();
        client
            .execute(
                "CREATE TABLE _sqlx_migrations (
                     version        BIGINT PRIMARY KEY,
                     description    TEXT NOT NULL,
                     installed_on   TIMESTAMPTZ NOT NULL DEFAULT now(),
                     success        BOOLEAN NOT NULL,
                     checksum       BYTEA NOT NULL,
                     execution_time BIGINT NOT NULL
                 )",
                &[],
            )
            .await
            .unwrap();

        for (i, statements) in migrations().iter().take(count).enumerate() {
            for statement in statements {
                client.execute(statement.as_str(), &[]).await.unwrap();
            }
            record_applied(&client, i, true).await;
        }
    }

    /// Records migration `i` in the previous runner's tracker.
    async fn record_applied(client: &Client, i: usize, success: bool) {
        let version = 20_250_911_i64.saturating_add(i64::try_from(i).unwrap());
        client
            .execute(
                "INSERT INTO _sqlx_migrations
                     (version, description, success, checksum, execution_time)
                 VALUES ($1, 'test', $2, '\\x00'::bytea, 0)",
                &[&version, &success],
            )
            .await
            .unwrap();
    }

    async fn applied_versions(pool: &Pool) -> Vec<i32> {
        pool.get()
            .await
            .unwrap()
            .query(
                &format!("SELECT version FROM {MIGRATIONS_TABLE} ORDER BY version"),
                &[],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect()
    }

    async fn table_exists(pool: &Pool, table: &str) -> bool {
        pool.get()
            .await
            .unwrap()
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM information_schema.tables
                     WHERE table_schema = current_schema() AND table_name = $1
                 )",
                &[&table],
            )
            .await
            .unwrap()
            .get(0)
    }

    fn all_versions() -> Vec<i32> {
        (1..=i32::try_from(migrations().len()).unwrap()).collect()
    }

    #[tokio::test]
    async fn fresh_database_applies_every_migration() {
        let pool = empty_test_pool("fresh_database").await;

        run(&pool).await.expect("migrate fresh database");

        assert_eq!(applied_versions(&pool).await, all_versions());
        assert!(table_exists(&pool, "domain_attribution").await);
    }

    #[tokio::test]
    async fn fully_migrated_database_reruns_nothing() {
        let pool = empty_test_pool("migrated_full").await;
        migrate_as_previous_runner(&pool, migrations().len()).await;

        // Re-running migration 1 would fail: `users` already exists.
        run(&pool).await.expect("adopt migrated database");

        assert_eq!(applied_versions(&pool).await, all_versions());
    }

    #[tokio::test]
    async fn partially_migrated_database_applies_the_rest() {
        let pool = empty_test_pool("migrated_partial").await;
        migrate_as_previous_runner(&pool, 6).await;
        assert!(!table_exists(&pool, "domain_attribution").await);

        run(&pool).await.expect("resume migrated database");

        assert_eq!(applied_versions(&pool).await, all_versions());
        assert!(table_exists(&pool, "domain_attribution").await);
    }

    #[tokio::test]
    async fn adoption_is_idempotent() {
        let pool = empty_test_pool("adoption_idempotent").await;
        migrate_as_previous_runner(&pool, migrations().len()).await;

        run(&pool).await.expect("first run");
        run(&pool).await.expect("second run");

        assert_eq!(applied_versions(&pool).await, all_versions());
    }

    /// A migration the previous runner recorded as failed never reached the
    /// database, so it has to be applied rather than adopted.
    #[tokio::test]
    async fn failed_migration_is_not_adopted() {
        let pool = empty_test_pool("failed_migration").await;
        migrate_as_previous_runner(&pool, 5).await;
        record_applied(&pool.get().await.unwrap(), 5, false).await;

        run(&pool).await.expect("apply the failed migration");

        assert_eq!(applied_versions(&pool).await, all_versions());
        assert!(table_exists(&pool, "invoices").await);
    }
}
