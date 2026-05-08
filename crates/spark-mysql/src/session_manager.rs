//! `MySQL`-backed implementation of the `SessionManager` trait.
//!
//! Direct port of `crates/spark-postgres/src/session_manager.rs`. See
//! `tree_store.rs` for the SQL translation rules used here.

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use macros::async_trait;
use mysql_async::prelude::*;
use mysql_async::{Pool, Row};
use spark_wallet::{Session, SessionManager, SessionManagerError};

use crate::config::MysqlStorageConfig;
use crate::error::MysqlError;
use crate::migrations::{Migration, run_migrations};
use crate::pool::create_pool;

const SESSION_MIGRATIONS_TABLE: &str = "session_schema_migrations";

/// `MySQL`-backed session manager.
///
/// Each instance is scoped to a single tenant identity so multiple tenants
/// can share one `MySQL` database without leaking sessions across tenants.
pub struct MysqlSessionManager {
    pool: Pool,
    /// 33-byte secp256k1 compressed pubkey identifying the tenant. All reads
    /// and writes are filtered by `user_id = self.identity`.
    identity: Vec<u8>,
}

#[async_trait]
impl SessionManager for MysqlSessionManager {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionManagerError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        let service_key = service_identity_key.serialize().to_vec();
        let row: Option<Row> = conn
            .exec_first(
                "SELECT token, expiration FROM sessions \
                 WHERE user_id = ? AND service_identity_key = ?",
                (self.identity.clone(), service_key),
            )
            .await
            .map_err(map_err)?;
        let row = row.ok_or(SessionManagerError::NotFound)?;
        let token: String = row
            .get::<Option<String>, _>("token")
            .ok_or_else(|| SessionManagerError::Generic("missing token column".to_string()))?
            .ok_or_else(|| SessionManagerError::Generic("token column is NULL".to_string()))?;
        let expiration: i64 = row
            .get::<Option<i64>, _>("expiration")
            .ok_or_else(|| SessionManagerError::Generic("missing expiration column".to_string()))?
            .ok_or_else(|| SessionManagerError::Generic("expiration column is NULL".to_string()))?;
        let expiration = u64::try_from(expiration)
            .map_err(|e| SessionManagerError::Generic(format!("invalid expiration: {e}")))?;
        Ok(Session { token, expiration })
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionManagerError> {
        let mut conn = self.pool.get_conn().await.map_err(map_err)?;
        let service_key = service_identity_key.serialize().to_vec();
        let expiration = i64::try_from(session.expiration)
            .map_err(|e| SessionManagerError::Generic(format!("expiration overflow: {e}")))?;
        conn.exec_drop(
            "INSERT INTO sessions (user_id, service_identity_key, token, expiration) \
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

impl MysqlSessionManager {
    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_config(
        config: MysqlStorageConfig,
        identity: &[u8],
    ) -> Result<Self, MysqlError> {
        let pool = create_pool(&config)?;
        Self::init(pool, identity).await
    }

    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_pool(pool: Pool, identity: &[u8]) -> Result<Self, MysqlError> {
        Self::init(pool, identity).await
    }

    async fn init(pool: Pool, identity: &[u8]) -> Result<Self, MysqlError> {
        let store = Self {
            pool,
            identity: identity.to_vec(),
        };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), MysqlError> {
        run_migrations(&self.pool, SESSION_MIGRATIONS_TABLE, &Self::migrations()).await
    }

    fn migrations() -> Vec<Vec<Migration>> {
        vec![vec![Migration::sql(
            "CREATE TABLE IF NOT EXISTS sessions (
                user_id VARBINARY(33) NOT NULL,
                service_identity_key VARBINARY(33) NOT NULL,
                token TEXT NOT NULL,
                expiration BIGINT NOT NULL,
                PRIMARY KEY (user_id, service_identity_key)
            )",
        )]]
    }
}

fn map_err<E: std::fmt::Display>(e: E) -> SessionManagerError {
    SessionManagerError::Generic(e.to_string())
}

/// Creates a `MysqlSessionManager` from a configuration.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_mysql_session_manager(
    config: MysqlStorageConfig,
    identity: &[u8],
) -> Result<Arc<dyn SessionManager>, MysqlError> {
    Ok(Arc::new(
        MysqlSessionManager::from_config(config, identity).await?,
    ))
}

/// Creates a `MysqlSessionManager` from an existing connection pool.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_mysql_session_manager_from_pool(
    pool: Pool,
    identity: &[u8],
) -> Result<Arc<dyn SessionManager>, MysqlError> {
    Ok(Arc::new(
        MysqlSessionManager::from_pool(pool, identity).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::session_manager_tests as shared_tests;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::mysql::Mysql;

    /// Fixed 33-byte test identity. Tests run in their own ephemeral container,
    /// so a single shared identity is fine — the schema still gets exercised.
    const TEST_IDENTITY: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];

    struct MysqlSessionManagerTestFixture {
        store: MysqlSessionManager,
        #[allow(dead_code)]
        container: ContainerAsync<Mysql>,
    }

    impl MysqlSessionManagerTestFixture {
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

            let store = MysqlSessionManager::from_config(
                MysqlStorageConfig::with_defaults(connection_string),
                &TEST_IDENTITY,
            )
            .await
            .expect("Failed to create MysqlSessionManager");

            Self { store, container }
        }
    }

    #[tokio::test]
    async fn test_get_session_not_found() {
        let fixture = MysqlSessionManagerTestFixture::new().await;
        shared_tests::test_get_session_not_found(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_and_get() {
        let fixture = MysqlSessionManagerTestFixture::new().await;
        shared_tests::test_set_and_get(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_overwrite_session() {
        let fixture = MysqlSessionManagerTestFixture::new().await;
        shared_tests::test_overwrite_session(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_sessions_are_isolated_by_key() {
        let fixture = MysqlSessionManagerTestFixture::new().await;
        shared_tests::test_sessions_are_isolated_by_key(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_after_unrelated_set() {
        let fixture = MysqlSessionManagerTestFixture::new().await;
        shared_tests::test_get_after_unrelated_set(&fixture.store).await;
    }
}
