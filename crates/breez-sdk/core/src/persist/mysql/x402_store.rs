use mysql_async::prelude::Queryable;
use spark_mysql::{Migration, mysql_async};

use super::base::run_migrations;
use crate::{X402ReplayStore, X402StoreError, persist::StorageError};

const MIGRATIONS_TABLE: &str = "brz_x402_schema_migrations";
const ER_DUP_ENTRY: u16 = 1062;

/// x402 replay records in `MySQL`, scoped to one tenant identity.
pub(crate) struct MysqlX402ReplayStore {
    pool: mysql_async::Pool,
    identity: Vec<u8>,
}

impl MysqlX402ReplayStore {
    pub(crate) async fn new(
        pool: mysql_async::Pool,
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

fn migrations() -> Vec<Vec<Migration>> {
    vec![vec![Migration::sql(
        "CREATE TABLE IF NOT EXISTS brz_x402_replay (
            user_id VARBINARY(33) NOT NULL,
            replay_key VARCHAR(255) NOT NULL,
            retain_until BIGINT NOT NULL,
            PRIMARY KEY (user_id, replay_key),
            KEY brz_idx_x402_replay_retain (user_id, retain_until)
        )",
    )]]
}

fn map_err<E: std::fmt::Display>(e: E) -> X402StoreError {
    X402StoreError::Generic(e.to_string())
}

#[macros::async_trait]
impl X402ReplayStore for MysqlX402ReplayStore {
    async fn consume(&self, key: String, retain_until_secs: u64) -> Result<bool, X402StoreError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        // The database's clock, which every instance sharing the table agrees on.
        conn.exec_drop(
            "DELETE FROM brz_x402_replay WHERE user_id = ? AND retain_until < UNIX_TIMESTAMP()",
            (&self.identity,),
        )
        .await
        .map_err(map_err)?;
        // A plain insert, not `ON DUPLICATE KEY UPDATE`: under `CLIENT_FOUND_ROWS`
        // a duplicate reports one affected row too, so only the error tells them apart.
        match conn
            .exec_drop(
                "INSERT INTO brz_x402_replay (user_id, replay_key, retain_until) VALUES (?, ?, ?)",
                (&self.identity, &key, retain_until_secs),
            )
            .await
        {
            Ok(()) => Ok(true),
            Err(mysql_async::Error::Server(e)) if e.code == ER_DUP_ENTRY => Ok(false),
            Err(e) => Err(map_err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mysql::Mysql;

    use super::*;
    use crate::persist::mysql::{MysqlStorageConfig, create_pool};

    #[tokio::test]
    async fn mysql_x402_replay_store() {
        let container = Mysql::default()
            .start()
            .await
            .expect("Failed to start MySQL container");
        let port = container.get_host_port_ipv4(3306).await.expect("host port");
        let pool = create_pool(&MysqlStorageConfig::with_defaults(format!(
            "mysql://root@127.0.0.1:{port}/test"
        )))
        .expect("pool");
        let tenant_a = MysqlX402ReplayStore::new(pool.clone(), &[2; 33], true)
            .await
            .expect("store");
        let tenant_b = MysqlX402ReplayStore::new(pool, &[3; 33], false)
            .await
            .expect("store");
        crate::x402::x402_replay_store_tests::run_all(&tenant_a, &tenant_b).await;
    }
}
