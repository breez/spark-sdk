//! MySQL-backed [`TurnkeyActivityStore`].
//!
//! Persists the timestamp chosen the first time an activity (identified by a
//! hash of its content) is submitted, so the approval-trigger submission and a
//! later re-submission reproduce the same Turnkey body byte-for-byte even when
//! they happen in separate processes, which the in-memory store cannot bridge.

use spark_mysql::mysql_async::prelude::*;
use spark_mysql::mysql_async::{Pool, Row};
use spark_mysql::{
    Migration, MysqlError, MysqlStorageConfig, create_pool, map_db_error, run_migrations,
};

use super::activity_store::TurnkeyActivityStore;

/// Tracks which migrations have been applied. `brz_`-prefixed to stay clear of
/// customer tables sharing the database.
const MIGRATIONS_TABLE: &str = "brz_turnkey_activity_schema_migrations";

fn migrations() -> Vec<Vec<Migration>> {
    vec![vec![Migration::sql(
        "CREATE TABLE IF NOT EXISTS brz_turnkey_activity_timestamps (
            activity_key VARCHAR(64) NOT NULL,
            timestamp_ms BIGINT NOT NULL,
            PRIMARY KEY (activity_key)
        )",
    )]]
}

/// A [`TurnkeyActivityStore`] backed by a `MySQL` table keyed by the activity
/// content hash. The key already encodes the Turnkey organization (it hashes
/// the activity type, organization id, and parameters), so a single table is
/// safe to share across wallets.
///
/// Construct it and hand it to the signer builder:
/// `TurnkeySignerBuilder::new(config).activity_store(Arc::new(store))`.
pub struct MysqlTurnkeyActivityStore {
    pool: Pool,
}

impl MysqlTurnkeyActivityStore {
    /// Connects using `config`, running the migration unless
    /// `config.run_migration` is false.
    pub async fn new(config: MysqlStorageConfig) -> Result<Self, MysqlError> {
        let run_migration = config.run_migration;
        let pool = create_pool(&config)?;
        Self::from_pool(pool, run_migration).await
    }

    /// Builds the store on an existing pool (e.g. one shared with the SDK's
    /// other `MySQL` stores), running the migration when `run_migration` is set.
    pub async fn from_pool(pool: Pool, run_migration: bool) -> Result<Self, MysqlError> {
        let store = Self { pool };
        if run_migration {
            run_migrations(&store.pool, MIGRATIONS_TABLE, &migrations(), None).await?;
        }
        Ok(store)
    }

    /// Records `fallback_now_ms` for `key` only if the key is new, then returns
    /// the stored value (the first writer's, for an existing key).
    async fn record_or_get(&self, key: &str, fallback_now_ms: u64) -> Result<u64, MysqlError> {
        let fallback = i64::try_from(fallback_now_ms)
            .map_err(|e| MysqlError::Database(format!("timestamp out of range: {e}")))?;
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        // Insert the fallback for a new key; keep the existing value otherwise
        // (the no-op update preserves the first recorded timestamp).
        conn.exec_drop(
            "INSERT INTO brz_turnkey_activity_timestamps (activity_key, timestamp_ms) \
             VALUES (?, ?) ON DUPLICATE KEY UPDATE timestamp_ms = timestamp_ms",
            (key, fallback),
        )
        .await
        .map_err(map_db_error)?;
        let row: Option<Row> = conn
            .exec_first(
                "SELECT timestamp_ms FROM brz_turnkey_activity_timestamps WHERE activity_key = ?",
                (key,),
            )
            .await
            .map_err(map_db_error)?;
        let row =
            row.ok_or_else(|| MysqlError::Database("timestamp row missing after upsert".into()))?;
        let stored: i64 = row
            .get::<Option<i64>, _>("timestamp_ms")
            .ok_or_else(|| MysqlError::Database("missing timestamp_ms column".into()))?
            .ok_or_else(|| MysqlError::Database("timestamp_ms is NULL".into()))?;
        u64::try_from(stored).map_err(|e| MysqlError::Database(format!("invalid timestamp: {e}")))
    }
}

#[macros::async_trait]
impl TurnkeyActivityStore for MysqlTurnkeyActivityStore {
    async fn timestamp_ms(&self, key: &str, fallback_now_ms: u64) -> u64 {
        // The trait is infallible, so a database failure degrades to the
        // current time: best effort. A re-submission then risks a fresh
        // timestamp (a new activity needing approval) rather than reusing the
        // recorded one, so failures are logged.
        match self.record_or_get(key, fallback_now_ms).await {
            Ok(timestamp_ms) => timestamp_ms,
            Err(e) => {
                tracing::warn!(
                    "MySQL turnkey activity store failed for key {key}: {e}; using current time"
                );
                fallback_now_ms
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mysql::Mysql;

    fn config(port: u16) -> MysqlStorageConfig {
        MysqlStorageConfig::with_defaults(format!("mysql://root@127.0.0.1:{port}/test"))
    }

    #[tokio::test]
    async fn records_then_reuses_timestamp_per_key() {
        let container = Mysql::default().start().await.expect("start mysql");
        let port = container
            .get_host_port_ipv4(3306)
            .await
            .expect("mysql host port");

        let store = MysqlTurnkeyActivityStore::new(config(port))
            .await
            .expect("create store");

        // First sight of a key records the fallback.
        assert_eq!(store.timestamp_ms("activity-a", 1000).await, 1000);
        // A later submission of the same activity reuses it, ignoring the new now.
        assert_eq!(store.timestamp_ms("activity-a", 9999).await, 1000);
        // A different activity gets its own timestamp.
        assert_eq!(store.timestamp_ms("activity-b", 2000).await, 2000);

        // A fresh store over the same database still sees the recorded value:
        // the whole point of persistence is to bridge separate processes.
        let reopened = MysqlTurnkeyActivityStore::new(config(port))
            .await
            .expect("reopen store");
        assert_eq!(reopened.timestamp_ms("activity-a", 5555).await, 1000);
    }
}
