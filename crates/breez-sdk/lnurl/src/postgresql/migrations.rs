//! Schema migrations for the LNURL server database.

use std::collections::HashSet;

use spark_postgres::deadpool_postgres::{Pool, Transaction};
use spark_postgres::{PostgresError, map_db_error, map_pool_error};

/// Tracker table for applied migrations. Prefixed so a database shared with
/// other Breez components gets a distinct table and a distinct migration
/// advisory lock, which `run_migrations` derives from this name.
const MIGRATIONS_TABLE: &str = "lnurl_schema_migrations";

/// Advisory lock guarding the one-shot adoption of the previous tracker.
/// Serializes instances starting against the same database so the `CREATE
/// TABLE` and the version inserts cannot interleave.
const ADOPT_LOCK_ID: i64 = 0x6c6e_7572_6c5f_6164; // "lnurl_ad"

/// The file versions the previous runner recorded, in the order it applied
/// them, matching [`migrations`] entry for entry: a database that stopped after
/// the Nth of these resumes at entry N+1.
///
/// That order is the order the files were added, which is not their version
/// order: `users_pubkey_index` carries a 14-digit version that sorts after the
/// 8-digit ones added over the following weeks, but it shipped before them.
/// Every deploy applied whatever was missing, so a database carries the files
/// that existed when it last ran.
///
/// The list is closed. It names the migrations that once existed as `.sql`
/// files, so anything appended to [`migrations`] since has no counterpart here
/// and is never adopted.
const PREVIOUS_VERSIONS: [i64; 19] = [
    20_250_911,         // initial
    20_251_013,         // zaps
    20_251_107,         // sender_comments
    20_251_127,         // server_signed
    20_251_204,         // allowed_domains
    20_260_123,         // invoices
    20_260_303,         // metadata_query_indexes
    20_260_303_211_700, // users_pubkey_index
    20_260_312,         // webhook_secret
    20_260_314,         // multi_instance
    20_260_323,         // drop_unused_user_columns
    20_260_326_110_000, // rename_pending_zap_receipts
    20_260_326_115_000, // invoice_domain_columns
    20_260_326_120_000, // domain_webhooks
    20_260_416_120_000, // webhook_signing
    20_260_709_000_000, // domain_attribution
    20_260_803_120_000, // used_signed_messages
    20_260_814_120_000, // released_names
    20_260_826_120_000, // address_registrations
];

/// Runs any migrations the database is missing.
pub async fn run(pool: &Pool) -> Result<(), PostgresError> {
    adopt_applied_prefix(pool).await?;
    spark_postgres::run_migrations(pool, MIGRATIONS_TABLE, &migrations(), None).await
}

/// Resumes a database migrated by the previous runner, which tracked applied
/// migrations in `_sqlx_migrations` keyed by file version instead of by the
/// ordinal used here. [`PREVIOUS_VERSIONS`] maps one onto the other.
///
/// Returns as soon as [`MIGRATIONS_TABLE`] exists, so this runs at most once per
/// database and can be deleted outright once every deployment is past the
/// cutover. `_sqlx_migrations` is deliberately left in place: a rollback to the
/// previous binary has to find its own tracker, or it re-runs migration 1
/// against a populated schema and fails to start.
async fn adopt_applied_prefix(pool: &Pool) -> Result<(), PostgresError> {
    let mut client = pool.get().await.map_err(map_pool_error)?;
    let tx = client.transaction().await.map_err(map_db_error)?;

    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&ADOPT_LOCK_ID])
        .await
        .map_err(|e| PostgresError::Initialization(format!("Failed to acquire adopt lock: {e}")))?;

    let table_exists_sql = "SELECT EXISTS (
             SELECT 1 FROM information_schema.tables
             WHERE table_schema = current_schema() AND table_name = $1
         )";

    let already_adopted: bool = tx
        .query_one(table_exists_sql, &[&MIGRATIONS_TABLE])
        .await
        .map_err(map_db_error)?
        .get(0);
    if already_adopted {
        return Ok(());
    }

    let previous_tracker_exists: bool = tx
        .query_one(table_exists_sql, &[&"_sqlx_migrations"])
        .await
        .map_err(map_db_error)?
        .get(0);
    if !previous_tracker_exists {
        return Ok(());
    }

    // `generate_series` yields nothing at 0, so a tracker holding none of ours
    // starts the schema from scratch.
    let adopted = applied_prefix_len(&tx).await?;
    tx.batch_execute(&format!(
        "CREATE TABLE {MIGRATIONS_TABLE} (
             version INTEGER PRIMARY KEY,
             applied_at TIMESTAMPTZ DEFAULT NOW()
         );
         INSERT INTO {MIGRATIONS_TABLE} (version)
         SELECT generate_series(1, {adopted});"
    ))
    .await
    .map_err(map_db_error)?;

    tx.commit().await.map_err(map_db_error)?;
    Ok(())
}

/// How many of [`PREVIOUS_VERSIONS`] the previous runner recorded as applied.
///
/// Reads the versions themselves rather than a row count. `_sqlx_migrations` is
/// sqlx's own table name, so a schema an unrelated application migrated with
/// sqlx carries one too, and a count taken off it would be that application's:
/// a server pointed at a database a partner already uses would resume part way
/// through a schema that was never created, or find every migration applied
/// against one holding no lnurl tables at all. Ignoring rows outside
/// [`PREVIOUS_VERSIONS`] also keeps the number right in a schema shared with
/// such an application.
///
/// The versions found have to be a leading run of [`PREVIOUS_VERSIONS`], since
/// every deploy applied whatever was missing. One recorded past a gap means
/// that list is in the wrong order, and adopting would mark a migration applied
/// whose statements never ran, so it is refused instead.
async fn applied_prefix_len(tx: &Transaction<'_>) -> Result<usize, PostgresError> {
    let known: &[i64] = &PREVIOUS_VERSIONS;
    let applied: HashSet<i64> = tx
        .query(
            "SELECT version FROM _sqlx_migrations WHERE success AND version = ANY($1)",
            &[&known],
        )
        .await
        .map_err(map_db_error)?
        .iter()
        .map(|row| row.get(0))
        .collect();

    let prefix = PREVIOUS_VERSIONS
        .iter()
        .take_while(|version| applied.contains(version))
        .count();

    if prefix < applied.len() {
        return Err(PostgresError::Initialization(format!(
            "_sqlx_migrations holds {} of this server's migrations but only the first {prefix} \
             run consecutively, so PREVIOUS_VERSIONS does not match the order they were applied in",
            applied.len(),
        )));
    }

    Ok(prefix)
}

/// The migrations, in the order the previous runner applied them, which
/// [`PREVIOUS_VERSIONS`] names version by version.
///
/// Position is the tracked version, so entries are only ever appended.
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
        vec!["CREATE INDEX idx_users_pubkey ON users(pubkey)".to_string()],
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
        vec![
            // Records every signed statement a mutating route has acted on, so
            // the same signature is never acted on twice. Keyed on the
            // statement rather than the signature bytes: ECDSA signatures are
            // malleable, so two distinct signatures can authorize the same
            // statement.
            "CREATE TABLE used_signed_messages (
                    statement_hash BYTEA PRIMARY KEY,
                    -- Which route acted on the statement. Not part of any
                    -- decision: a claimed statement is refused whoever claimed
                    -- it. Kept for diagnostics.
                    route TEXT NOT NULL,
                    -- When the row may be pruned, which is the point past which
                    -- the statement is refused on its own. A statement nothing
                    -- else bounds in time is recorded with an expiry that never
                    -- arrives.
                    expires_at BIGINT NOT NULL,
                    created_at BIGINT NOT NULL
                )"
            .to_string(),
            "CREATE INDEX idx_used_signed_messages_expires_at
                 ON used_signed_messages(expires_at)"
                .to_string(),
        ],
        vec![
            // A name whose owner gave it up. While the row stands, only
            // `pubkey` may register the name again: payers keep paying an
            // address long after its owner drops it, so handing it to a
            // stranger misdirects their payments.
            //
            // reclaimable_from is when anyone may register it. Every hold is
            // written NULL, which reserves the name for good; the column is
            // what a policy that lets holds lapse (a per-partner cooldown, say)
            // would fill in.
            "CREATE TABLE released_names (
                    domain VARCHAR(255) NOT NULL,
                    name VARCHAR(64) NOT NULL,
                    pubkey VARCHAR(66) NOT NULL,
                    released_at BIGINT NOT NULL,
                    reclaimable_from BIGINT,
                    PRIMARY KEY (domain, name)
                )"
            .to_string(),
        ],
        vec![
            // One row per successful registration that changed the name a
            // pubkey holds in a domain, counted to bound how fast one client
            // can churn through addresses. Rows older than the counting window
            // are pruned by the cleanup processor.
            "CREATE TABLE address_registrations (
                    id BIGSERIAL PRIMARY KEY,
                    domain VARCHAR(255) NOT NULL,
                    pubkey VARCHAR(66) NOT NULL,
                    created_at BIGINT NOT NULL
                )"
            .to_string(),
            "CREATE INDEX idx_address_registrations_domain_pubkey_created
                 ON address_registrations (domain, pubkey, created_at)"
                .to_string(),
            // The pruner filters on created_at alone, which the index above
            // cannot serve.
            "CREATE INDEX idx_address_registrations_created_at
                 ON address_registrations (created_at)"
                .to_string(),
        ],
    ]
}

#[cfg(test)]
mod tests {
    use spark_postgres::deadpool_postgres::{Client, Pool};

    use super::{MIGRATIONS_TABLE, PREVIOUS_VERSIONS, migrations, run};
    use crate::test_support::empty_test_pool;

    /// Brings a schema to the state the previous runner left it in after
    /// applying the first `count` migrations: the statements themselves, plus
    /// its tracker table keyed by the file versions it really used.
    async fn migrate_as_previous_runner(pool: &Pool, count: usize) {
        let client = pool.get().await.unwrap();
        create_previous_tracker(&client).await;

        for (i, statements) in migrations().iter().take(count).enumerate() {
            for statement in statements {
                client.execute(statement.as_str(), &[]).await.unwrap();
            }
            record_applied(&client, i, true).await;
        }
    }

    async fn create_previous_tracker(client: &Client) {
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
    }

    /// Records migration `i` under the file version it shipped with.
    async fn record_applied(client: &Client, i: usize, success: bool) {
        record_version(client, PREVIOUS_VERSIONS[i], success).await;
    }

    async fn record_version(client: &Client, version: i64, success: bool) {
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

    /// `_sqlx_migrations` is sqlx's hardcoded table name, so a schema an
    /// unrelated application migrated with sqlx carries one. Adopting its count
    /// would leave us running against a schema with no lnurl tables in it.
    #[tokio::test]
    async fn a_foreign_tracker_is_not_adopted() {
        let (_pg, pool) = empty_test_pool().await;
        {
            let client = pool.get().await.unwrap();
            create_previous_tracker(&client).await;
            for version in [20_240_101_i64, 20_240_202, 20_240_303] {
                record_version(&client, version, true).await;
            }
        }

        run(&pool)
            .await
            .expect("migrate alongside a foreign tracker");

        assert_eq!(applied_versions(&pool).await, all_versions());
        assert!(table_exists(&pool, "domain_attribution").await);
    }

    /// A schema holding our tables and somebody else's, sharing the one tracker
    /// sqlx gives them both. Only our own versions may be counted: the foreign
    /// rows would otherwise inflate the prefix and skip migrations that never
    /// ran.
    #[tokio::test]
    async fn a_shared_tracker_counts_only_our_versions() {
        let (_pg, pool) = empty_test_pool().await;
        migrate_as_previous_runner(&pool, 6).await;
        {
            let client = pool.get().await.unwrap();
            for version in [20_240_101_i64, 20_240_202, 20_240_303, 20_270_101] {
                record_version(&client, version, true).await;
            }
        }
        assert!(!table_exists(&pool, "domain_attribution").await);

        run(&pool)
            .await
            .expect("resume alongside a foreign tracker");

        assert_eq!(applied_versions(&pool).await, all_versions());
        assert!(table_exists(&pool, "domain_attribution").await);
    }

    /// A version recorded past a gap means [`PREVIOUS_VERSIONS`] is not the
    /// order these were applied in. Adopting the leading run would mark the
    /// skipped one applied though its statements never ran, so startup fails
    /// instead.
    #[tokio::test]
    async fn a_gap_in_the_recorded_versions_is_refused() {
        let (_pg, pool) = empty_test_pool().await;
        migrate_as_previous_runner(&pool, 6).await;
        // Entry 7 never ran, so entry 8 cannot legitimately be recorded.
        record_applied(&pool.get().await.unwrap(), 7, true).await;

        let err = run(&pool)
            .await
            .expect_err("refuse an out-of-order tracker");

        assert!(
            format!("{err}").contains("PREVIOUS_VERSIONS"),
            "expected the order to be named, got {err}"
        );
        assert!(!table_exists(&pool, MIGRATIONS_TABLE).await);
    }

    /// Every version has to name a migration that is still in the list, or
    /// adoption would record one no statement backs, and no two may name the
    /// same one: a repeated version counts once and shortens every prefix past
    /// it.
    ///
    /// That the versions are in the order they were applied is not checkable
    /// here, since the fixtures read the same list. It was established against
    /// the original `.sql` files, and a database that contradicts it is refused
    /// at startup rather than adopted (see [`applied_prefix_len`]).
    #[test]
    fn previous_versions_line_up_with_the_migrations() {
        assert!(
            PREVIOUS_VERSIONS.len() <= migrations().len(),
            "PREVIOUS_VERSIONS names {} migrations but only {} exist",
            PREVIOUS_VERSIONS.len(),
            migrations().len()
        );

        let distinct: std::collections::HashSet<i64> = PREVIOUS_VERSIONS.into_iter().collect();
        assert_eq!(
            distinct.len(),
            PREVIOUS_VERSIONS.len(),
            "PREVIOUS_VERSIONS repeats a version"
        );
    }

    #[tokio::test]
    async fn fresh_database_applies_every_migration() {
        let (_pg, pool) = empty_test_pool().await;

        run(&pool).await.expect("migrate fresh database");

        assert_eq!(applied_versions(&pool).await, all_versions());
        assert!(table_exists(&pool, "domain_attribution").await);
    }

    #[tokio::test]
    async fn fully_migrated_database_reruns_nothing() {
        let (_pg, pool) = empty_test_pool().await;
        migrate_as_previous_runner(&pool, migrations().len()).await;

        // Re-running migration 1 would fail: `users` already exists.
        run(&pool).await.expect("adopt migrated database");

        assert_eq!(applied_versions(&pool).await, all_versions());
    }

    #[tokio::test]
    async fn partially_migrated_database_applies_the_rest() {
        let (_pg, pool) = empty_test_pool().await;
        migrate_as_previous_runner(&pool, 6).await;
        assert!(!table_exists(&pool, "domain_attribution").await);

        run(&pool).await.expect("resume migrated database");

        assert_eq!(applied_versions(&pool).await, all_versions());
        assert!(table_exists(&pool, "domain_attribution").await);
    }

    #[tokio::test]
    async fn adoption_is_idempotent() {
        let (_pg, pool) = empty_test_pool().await;
        migrate_as_previous_runner(&pool, migrations().len()).await;

        run(&pool).await.expect("first run");
        run(&pool).await.expect("second run");

        assert_eq!(applied_versions(&pool).await, all_versions());
    }

    /// A tracker carrying versions past the last one this server ever shipped
    /// as a file must not record migrations no statement backs: those would
    /// mark later ones as already applied and skip them forever.
    #[tokio::test]
    async fn adoption_is_bounded_by_the_known_versions() {
        let (_pg, pool) = empty_test_pool().await;
        migrate_as_previous_runner(&pool, PREVIOUS_VERSIONS.len()).await;
        let client = pool.get().await.unwrap();
        for version in [20_270_101_i64, 20_270_202, 20_270_303] {
            record_version(&client, version, true).await;
        }

        run(&pool).await.expect("adopt over-long tracker");

        assert_eq!(applied_versions(&pool).await, all_versions());
    }

    /// A migration the previous runner recorded as failed never reached the
    /// database, so it has to be applied rather than adopted.
    #[tokio::test]
    async fn failed_migration_is_not_adopted() {
        let (_pg, pool) = empty_test_pool().await;
        migrate_as_previous_runner(&pool, 5).await;
        record_applied(&pool.get().await.unwrap(), 5, false).await;

        run(&pool).await.expect("apply the failed migration");

        assert_eq!(applied_versions(&pool).await, all_versions());
        assert!(table_exists(&pool, "invoices").await);
    }
}
