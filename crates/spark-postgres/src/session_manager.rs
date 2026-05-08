//! `PostgreSQL`-backed implementation of the `SessionManager` trait.
//!
//! Provides a persistent session store keyed by tenant identity + service
//! identity public key, suitable for multi-pod deployments where multiple
//! SDK instances share authentication state through a common database.

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use deadpool_postgres::Pool;
use macros::async_trait;
use spark_wallet::{Session, SessionManager, SessionManagerError};

use crate::config::PostgresStorageConfig;
use crate::error::PostgresError;
use crate::migrations::run_migrations;
use crate::pool::create_pool;

const SESSION_MIGRATIONS_TABLE: &str = "session_schema_migrations";

/// `PostgreSQL`-backed session manager.
///
/// Each instance is scoped to a single tenant identity so multiple tenants
/// can share one Postgres database without leaking sessions across tenants.
pub struct PostgresSessionManager {
    pool: Pool,
    /// 33-byte secp256k1 compressed pubkey identifying the tenant. All reads
    /// and writes are filtered by `user_id = self.identity`.
    identity: Vec<u8>,
}

#[async_trait]
impl SessionManager for PostgresSessionManager {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionManagerError> {
        let client = self.pool.get().await.map_err(map_err)?;
        let service_key = service_identity_key.serialize().to_vec();
        let row = client
            .query_opt(
                r"SELECT token, expiration FROM sessions
                  WHERE user_id = $1 AND service_identity_key = $2",
                &[&self.identity, &service_key],
            )
            .await
            .map_err(map_err)?;
        let row = row.ok_or(SessionManagerError::NotFound)?;
        let token: String = row.get(0);
        let expiration: i64 = row.get(1);
        let expiration = u64::try_from(expiration)
            .map_err(|e| SessionManagerError::Generic(format!("invalid expiration: {e}")))?;
        Ok(Session { token, expiration })
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionManagerError> {
        let client = self.pool.get().await.map_err(map_err)?;
        let service_key = service_identity_key.serialize().to_vec();
        let expiration = i64::try_from(session.expiration)
            .map_err(|e| SessionManagerError::Generic(format!("expiration overflow: {e}")))?;
        client
            .execute(
                r"INSERT INTO sessions (user_id, service_identity_key, token, expiration)
                  VALUES ($1, $2, $3, $4)
                  ON CONFLICT (user_id, service_identity_key)
                  DO UPDATE SET token = EXCLUDED.token, expiration = EXCLUDED.expiration",
                &[&self.identity, &service_key, &session.token, &expiration],
            )
            .await
            .map_err(map_err)?;
        Ok(())
    }
}

impl PostgresSessionManager {
    /// Creates a new `PostgresSessionManager` from a configuration.
    ///
    /// Allocates its own connection pool and runs migrations.
    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_config(
        config: PostgresStorageConfig,
        identity: &[u8],
    ) -> Result<Self, PostgresError> {
        let schema_managed_externally = config.schema_managed_externally;
        let pool = create_pool(&config)?;
        Self::init(pool, identity, schema_managed_externally).await
    }

    /// Creates a new `PostgresSessionManager` from an existing connection pool.
    ///
    /// Useful when sharing a pool with other components (e.g., `PostgresStorage`).
    pub async fn from_pool(pool: Pool, identity: &[u8]) -> Result<Self, PostgresError> {
        Self::init(pool, identity, false).await
    }

    /// Creates a new `PostgresSessionManager` from an existing connection pool.
    ///
    /// When `schema_managed_externally` is true, initialization trusts the
    /// existing schema and skips session manager migrations entirely.
    pub async fn from_pool_with_schema_management(
        pool: Pool,
        identity: &[u8],
        schema_managed_externally: bool,
    ) -> Result<Self, PostgresError> {
        Self::init(pool, identity, schema_managed_externally).await
    }

    async fn init(
        pool: Pool,
        identity: &[u8],
        schema_managed_externally: bool,
    ) -> Result<Self, PostgresError> {
        let store = Self {
            pool,
            identity: identity.to_vec(),
        };
        if !schema_managed_externally {
            store.migrate().await?;
        }
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), PostgresError> {
        run_migrations(&self.pool, SESSION_MIGRATIONS_TABLE, &Self::migrations()).await
    }

    fn migrations() -> Vec<Vec<String>> {
        vec![vec![
            "CREATE TABLE IF NOT EXISTS sessions (
                user_id BYTEA NOT NULL,
                service_identity_key BYTEA NOT NULL,
                token TEXT NOT NULL,
                expiration BIGINT NOT NULL,
                PRIMARY KEY (user_id, service_identity_key)
            )"
            .to_string(),
        ]]
    }
}

fn map_err<E: std::fmt::Display>(e: E) -> SessionManagerError {
    SessionManagerError::Generic(e.to_string())
}

/// Creates a `PostgresSessionManager` from a configuration.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_postgres_session_manager(
    config: PostgresStorageConfig,
    identity: &[u8],
) -> Result<Arc<dyn SessionManager>, PostgresError> {
    Ok(Arc::new(
        PostgresSessionManager::from_config(config, identity).await?,
    ))
}

/// Creates a `PostgresSessionManager` from an existing connection pool.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_postgres_session_manager_from_pool(
    pool: Pool,
    identity: &[u8],
) -> Result<Arc<dyn SessionManager>, PostgresError> {
    Ok(Arc::new(
        PostgresSessionManager::from_pool(pool, identity).await?,
    ))
}

/// Creates a `PostgresSessionManager` from an existing connection pool.
///
/// If `schema_managed_externally` is true, skips SDK-managed schema
/// migrations and trusts that the `sessions` table already exists.
pub async fn create_postgres_session_manager_from_pool_with_schema_management(
    pool: Pool,
    identity: &[u8],
    schema_managed_externally: bool,
) -> Result<Arc<dyn SessionManager>, PostgresError> {
    Ok(Arc::new(
        PostgresSessionManager::from_pool_with_schema_management(
            pool,
            identity,
            schema_managed_externally,
        )
        .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::session_manager_tests as shared_tests;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::postgres::Postgres;

    /// Fixed 33-byte test identity. Tests run in their own ephemeral container,
    /// so a single shared identity is fine — the schema still gets exercised.
    const TEST_IDENTITY: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];

    struct PostgresSessionManagerTestFixture {
        store: PostgresSessionManager,
        #[allow(dead_code)]
        container: ContainerAsync<Postgres>,
    }

    impl PostgresSessionManagerTestFixture {
        async fn new() -> Self {
            let container = Postgres::default()
                .start()
                .await
                .expect("Failed to start PostgreSQL container");

            let host_port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get host port");

            let connection_string = format!(
                "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
            );

            let store = PostgresSessionManager::from_config(
                PostgresStorageConfig::with_defaults(connection_string),
                &TEST_IDENTITY,
            )
            .await
            .expect("Failed to create PostgresSessionManager");

            Self { store, container }
        }
    }

    #[tokio::test]
    async fn test_get_session_not_found() {
        let fixture = PostgresSessionManagerTestFixture::new().await;
        shared_tests::test_get_session_not_found(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_and_get() {
        let fixture = PostgresSessionManagerTestFixture::new().await;
        shared_tests::test_set_and_get(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_overwrite_session() {
        let fixture = PostgresSessionManagerTestFixture::new().await;
        shared_tests::test_overwrite_session(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_sessions_are_isolated_by_key() {
        let fixture = PostgresSessionManagerTestFixture::new().await;
        shared_tests::test_sessions_are_isolated_by_key(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_after_unrelated_set() {
        let fixture = PostgresSessionManagerTestFixture::new().await;
        shared_tests::test_get_after_unrelated_set(&fixture.store).await;
    }
}
