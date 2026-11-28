use spark_postgres::deadpool_postgres;

use super::base::run_migrations;
use crate::{X402ReplayStore, X402StoreError, persist::StorageError};

const MIGRATIONS_TABLE: &str = "brz_x402_schema_migrations";

/// x402 replay records in `PostgreSQL`, scoped to one tenant identity.
pub(crate) struct PostgresX402ReplayStore {
    pool: deadpool_postgres::Pool,
    identity: Vec<u8>,
}

impl PostgresX402ReplayStore {
    pub(crate) async fn new(
        pool: deadpool_postgres::Pool,
        identity: &[u8],
        run_migration: bool,
    ) -> Result<Self, StorageError> {
        if run_migration {
            run_migrations(&pool, MIGRATIONS_TABLE, &migrations(), None).await?;
        }
        Ok(Self {
            pool,
            identity: identity.to_vec(),
        })
    }
}

fn migrations() -> Vec<Vec<String>> {
    vec![vec![
        "CREATE TABLE IF NOT EXISTS brz_x402_replay (
            user_id BYTEA NOT NULL,
            replay_key TEXT NOT NULL,
            retain_until BIGINT NOT NULL,
            PRIMARY KEY (user_id, replay_key)
        )"
        .to_string(),
        "CREATE INDEX IF NOT EXISTS brz_idx_x402_replay_retain
            ON brz_x402_replay (user_id, retain_until)"
            .to_string(),
    ]]
}

fn map_err<E: std::fmt::Display>(e: E) -> X402StoreError {
    X402StoreError::Generic(e.to_string())
}

#[macros::async_trait]
impl X402ReplayStore for PostgresX402ReplayStore {
    async fn consume(&self, key: String, retain_until_secs: u64) -> Result<bool, X402StoreError> {
        let retain_until = i64::try_from(retain_until_secs).map_err(map_err)?;
        let client = self.pool.get().await.map_err(map_err)?;
        // The database's clock, which every instance sharing the table agrees on.
        client
            .execute(
                "DELETE FROM brz_x402_replay
                 WHERE user_id = $1 AND retain_until < EXTRACT(EPOCH FROM now())::BIGINT",
                &[&self.identity],
            )
            .await
            .map_err(map_err)?;
        let inserted = client
            .execute(
                "INSERT INTO brz_x402_replay (user_id, replay_key, retain_until)
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                &[&self.identity, &key, &retain_until],
            )
            .await
            .map_err(map_err)?;
        Ok(inserted == 1)
    }
}

#[cfg(test)]
mod tests {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    use super::*;
    use crate::persist::postgres::{PostgresStorageConfig, create_pool};

    #[tokio::test]
    async fn postgres_x402_replay_store() {
        let container = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");
        let port = container.get_host_port_ipv4(5432).await.expect("host port");
        let pool = create_pool(&PostgresStorageConfig::with_defaults(format!(
            "host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres"
        )))
        .expect("pool");
        let tenant_a = PostgresX402ReplayStore::new(pool.clone(), &[2; 33], true)
            .await
            .expect("store");
        let tenant_b = PostgresX402ReplayStore::new(pool, &[3; 33], false)
            .await
            .expect("store");
        crate::x402::x402_replay_store_tests::run_all(&tenant_a, &tenant_b).await;
    }
}
