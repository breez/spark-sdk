use std::{path::PathBuf, time::Duration};

use rusqlite::{Connection, TransactionBehavior, params};

use crate::{X402ReplayStore, X402StoreError, persist::StorageError};

/// Versioned like the tree store: the main storage owns `PRAGMA user_version`
/// in the file both share.
const MIGRATIONS_TABLE: &str = "brz_x402_schema_migrations";
const MIGRATIONS: &[&str] = &["CREATE TABLE brz_x402_replay (
        replay_key TEXT PRIMARY KEY,
        retain_until INTEGER NOT NULL
    );
    CREATE INDEX brz_idx_x402_replay_retain ON brz_x402_replay (retain_until);"];
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// x402 replay records in the `SQLite` file of one tenant's storage.
pub(crate) struct SqliteX402ReplayStore {
    db_path: PathBuf,
}

impl SqliteX402ReplayStore {
    pub(crate) fn new(db_path: PathBuf) -> Result<Self, StorageError> {
        let store = Self { db_path };
        store.migrate()?;
        Ok(store)
    }

    fn connection(&self) -> Result<Connection, rusqlite::Error> {
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        Ok(conn)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {MIGRATIONS_TABLE} (version INTEGER PRIMARY KEY)"
        ))?;
        let current: i64 = tx.query_row(
            &format!("SELECT COALESCE(MAX(version), 0) FROM {MIGRATIONS_TABLE}"),
            [],
            |row| row.get(0),
        )?;
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            let version = i64::try_from(index)
                .map_err(|e| StorageError::InitializationError(e.to_string()))?
                .saturating_add(1);
            if version > current {
                tx.execute_batch(migration)?;
                tx.execute(
                    &format!("INSERT INTO {MIGRATIONS_TABLE} (version) VALUES (?1)"),
                    params![version],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

fn map_err<E: std::fmt::Display>(e: E) -> X402StoreError {
    X402StoreError::Generic(e.to_string())
}

#[macros::async_trait]
impl X402ReplayStore for SqliteX402ReplayStore {
    async fn consume(&self, key: String, retain_until_secs: u64) -> Result<bool, X402StoreError> {
        let retain_until = i64::try_from(retain_until_secs).map_err(map_err)?;
        let now = i64::from(breez_sdk_common::utils::now());
        let mut conn = self.connection().map_err(map_err)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_err)?;
        tx.execute(
            "DELETE FROM brz_x402_replay WHERE retain_until < ?1",
            params![now],
        )
        .map_err(map_err)?;
        let inserted = tx
            .execute(
                "INSERT OR IGNORE INTO brz_x402_replay (replay_key, retain_until) VALUES (?1, ?2)",
                params![key, retain_until],
            )
            .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(inserted == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("breez-test-x402-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[tokio::test]
    async fn sqlite_x402_store() {
        let storage = crate::SqliteStorage::new(&temp_dir()).expect("storage");
        let user_version = || -> i64 {
            Connection::open(storage.get_db_path())
                .and_then(|conn| conn.query_row("PRAGMA user_version", [], |row| row.get(0)))
                .expect("user_version")
        };
        let main_version = user_version();
        let store = SqliteX402ReplayStore::new(storage.get_db_path()).expect("store");
        // Migrating again leaves the schema and the main storage's version alone.
        let store_again = SqliteX402ReplayStore::new(storage.get_db_path()).expect("store");
        assert_eq!(user_version(), main_version);
        let other_storage = crate::SqliteStorage::new(&temp_dir()).expect("storage");
        let other = SqliteX402ReplayStore::new(other_storage.get_db_path()).expect("store");
        crate::x402::x402_replay_store_tests::run_all(&store, &other).await;
        assert!(
            !store_again
                .consume("lnbtc:a:key".to_string(), u64::MAX >> 2)
                .await
                .unwrap(),
            "shares the first store's records"
        );
    }

    #[test]
    fn sqlite_x402_store_race_across_connections() {
        let storage = crate::SqliteStorage::new(&temp_dir()).expect("storage");
        let path = storage.get_db_path();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let racers: Vec<_> = (0..8)
            .map(|_| {
                let store = SqliteX402ReplayStore::new(path.clone()).expect("store");
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    futures::executor::block_on(
                        store.consume("lnbtc:a:race".to_string(), u64::MAX >> 2),
                    )
                    .expect("consume")
                })
            })
            .collect();
        let winners = racers
            .into_iter()
            .map(|racer| racer.join().expect("racer"))
            .filter(|won| *won)
            .count();
        assert_eq!(winners, 1);
    }
}
