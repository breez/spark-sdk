//! `MySQL`-backed implementation of the `SessionStore` trait.
//!
//! Direct port of `crates/spark-postgres/src/session_store.rs`. See
//! `tree_store.rs` for the SQL translation rules used here.

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use macros::async_trait;
use mysql_async::prelude::*;
use mysql_async::{Pool, Row};
use spark_wallet::{Session, SessionStore, SessionStoreError};

use crate::config::MysqlStorageConfig;
use crate::error::MysqlError;
use crate::migrations::{Migration, SchemaRenames, run_migrations};
use crate::pool::create_pool;

const SESSION_MIGRATIONS_TABLE: &str = "brz_session_schema_migrations";

/// Pre-prefix rename map for upgrading session-store deployments.
/// `MySQL` PKs are always named `PRIMARY` (table-scoped), so only the table
/// and migrations tracker need renaming.
const SCHEMA_RENAMES: SchemaRenames<'static> = SchemaRenames {
    old_migrations_table: "session_schema_migrations",
    new_migrations_table: SESSION_MIGRATIONS_TABLE,
    tables: &[("sessions", "brz_sessions")],
    indexes: &[],
    foreign_keys: &[],
};

/// `MySQL`-backed session store.
///
/// Each instance is scoped to a single tenant identity so multiple tenants
/// can share one `MySQL` database without leaking sessions across tenants.
pub struct MysqlSessionStore {
    pool: Pool,
    /// 33-byte secp256k1 compressed pubkey identifying the tenant. All reads
    /// and writes are filtered by `user_id = self.identity`.
    identity: Vec<u8>,
}

#[async_trait]
impl SessionStore for MysqlSessionStore {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionStoreError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        let service_key = service_identity_key.serialize().to_vec();
        let row: Option<Row> = conn
            .exec_first(
                "SELECT token, expiration FROM brz_sessions \
                 WHERE user_id = ? AND service_identity_key = ?",
                (self.identity.clone(), service_key),
            )
            .await
            .map_err(map_err)?;
        let row = row.ok_or(SessionStoreError::NotFound)?;
        let token: String = row
            .get::<Option<String>, _>("token")
            .ok_or_else(|| SessionStoreError::Generic("missing token column".to_string()))?
            .ok_or_else(|| SessionStoreError::Generic("token column is NULL".to_string()))?;
        let expiration: i64 = row
            .get::<Option<i64>, _>("expiration")
            .ok_or_else(|| SessionStoreError::Generic("missing expiration column".to_string()))?
            .ok_or_else(|| SessionStoreError::Generic("expiration column is NULL".to_string()))?;
        let expiration = u64::try_from(expiration)
            .map_err(|e| SessionStoreError::Generic(format!("invalid expiration: {e}")))?;
        Ok(Session { token, expiration })
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionStoreError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        let service_key = service_identity_key.serialize().to_vec();
        let expiration = i64::try_from(session.expiration)
            .map_err(|e| SessionStoreError::Generic(format!("expiration overflow: {e}")))?;
        conn.exec_drop(
            "INSERT INTO brz_sessions (user_id, service_identity_key, token, expiration) \
             VALUES (?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE token = VALUES(token), expiration = VALUES(expiration)",
            (
                self.identity.clone(),
                service_key,
                session.token,
                expiration,
            ),
        )
        .await
        .map_err(map_err)?;
        Ok(())
    }
}

impl MysqlSessionStore {
    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_config(
        config: MysqlStorageConfig,
        identity: &[u8],
    ) -> Result<Self, MysqlError> {
        let run_migration = config.run_migration;
        let pool = create_pool(&config)?;
        Self::from_pool(pool, identity, run_migration).await
    }

    /// Creates a new `MysqlSessionStore` from an existing connection pool.
    ///
    /// `identity` is the 33-byte secp256k1 pubkey of the tenant. When
    /// `run_migration` is `false`, initialization trusts the existing schema
    /// and skips session store migrations entirely.
    pub async fn from_pool(
        pool: Pool,
        identity: &[u8],
        run_migration: bool,
    ) -> Result<Self, MysqlError> {
        let store = Self {
            pool,
            identity: identity.to_vec(),
        };
        if run_migration {
            store.migrate().await?;
        }
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), MysqlError> {
        run_migrations(
            &self.pool,
            SESSION_MIGRATIONS_TABLE,
            &Self::migrations(),
            Some(&SCHEMA_RENAMES),
        )
        .await
    }

    fn migrations() -> Vec<Vec<Migration>> {
        vec![
            vec![Migration::sql(
                "CREATE TABLE IF NOT EXISTS brz_sessions (
                    user_id VARBINARY(33) NOT NULL,
                    service_identity_key VARBINARY(33) NOT NULL,
                    token TEXT NOT NULL,
                    expiration BIGINT NOT NULL,
                    PRIMARY KEY (user_id, service_identity_key)
                )",
            )],
            vec![Migration::sql(
                "ALTER TABLE brz_session_schema_migrations MODIFY COLUMN applied_at \
                 DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))",
            )],
        ]
    }
}

fn map_err<E: std::fmt::Display>(e: E) -> SessionStoreError {
    SessionStoreError::Generic(e.to_string())
}

/// Creates a `MysqlSessionStore` from a configuration.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_mysql_session_store(
    config: MysqlStorageConfig,
    identity: &[u8],
) -> Result<Arc<dyn SessionStore>, MysqlError> {
    Ok(Arc::new(
        MysqlSessionStore::from_config(config, identity).await?,
    ))
}

/// Creates a `MysqlSessionStore` from an existing connection pool.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
/// When `run_migration` is `false`, skips SDK-managed schema migrations and
/// trusts that the `sessions` table already exists.
pub async fn create_mysql_session_store_from_pool(
    pool: Pool,
    identity: &[u8],
    run_migration: bool,
) -> Result<Arc<dyn SessionStore>, MysqlError> {
    Ok(Arc::new(
        MysqlSessionStore::from_pool(pool, identity, run_migration).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::session_store_tests as shared_tests;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::mysql::Mysql;

    /// Fixed 33-byte test identity. Tests run in their own ephemeral container,
    /// so a single shared identity is fine — the schema still gets exercised.
    const TEST_IDENTITY: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];

    struct MysqlSessionStoreTestFixture {
        store: MysqlSessionStore,
        #[allow(dead_code)]
        container: ContainerAsync<Mysql>,
    }

    impl MysqlSessionStoreTestFixture {
        async fn new() -> Self {
            let container = Mysql::default()
                .start()
                .await
                .expect("Failed to start MySQL container");

            let host_port = container
                .get_host_port_ipv4(3306)
                .await
                .expect("Failed to get host port");

            let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

            let store = MysqlSessionStore::from_config(
                MysqlStorageConfig::with_defaults(connection_string),
                &TEST_IDENTITY,
            )
            .await
            .expect("Failed to create MysqlSessionStore");

            Self { store, container }
        }
    }

    #[tokio::test]
    async fn test_get_session_not_found() {
        let fixture = MysqlSessionStoreTestFixture::new().await;
        shared_tests::test_get_session_not_found(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_and_get() {
        let fixture = MysqlSessionStoreTestFixture::new().await;
        shared_tests::test_set_and_get(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_overwrite_session() {
        let fixture = MysqlSessionStoreTestFixture::new().await;
        shared_tests::test_overwrite_session(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_sessions_are_isolated_by_key() {
        let fixture = MysqlSessionStoreTestFixture::new().await;
        shared_tests::test_sessions_are_isolated_by_key(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_after_unrelated_set() {
        let fixture = MysqlSessionStoreTestFixture::new().await;
        shared_tests::test_get_after_unrelated_set(&fixture.store).await;
    }
}
