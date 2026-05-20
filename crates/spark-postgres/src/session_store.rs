//! `PostgreSQL`-backed implementation of the `SessionStore` trait.
//!
//! Provides a persistent session store keyed by tenant identity + service
//! identity public key, suitable for multi-pod deployments where multiple
//! SDK instances share authentication state through a common database.

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use deadpool_postgres::Pool;
use macros::async_trait;
use spark_wallet::{Session, SessionStore, SessionStoreError};

use crate::config::PostgresStorageConfig;
use crate::error::PostgresError;
use crate::migrations::{SchemaRenames, run_migrations};
use crate::pool::create_pool;

const SESSION_MIGRATIONS_TABLE: &str = "brz_session_schema_migrations";

/// Pre-prefix rename map for upgrading session-store deployments.
const SCHEMA_RENAMES: SchemaRenames<'static> = SchemaRenames {
    old_migrations_table: "session_schema_migrations",
    new_migrations_table: SESSION_MIGRATIONS_TABLE,
    tables: &[("sessions", "brz_sessions")],
    indexes: &[],
    constraints: &[("brz_sessions", "sessions_pkey", "brz_sessions_pkey")],
};

/// `PostgreSQL`-backed session store.
///
/// Each instance is scoped to a single tenant identity so multiple tenants
/// can share one Postgres database without leaking sessions across tenants.
pub struct PostgresSessionStore {
    pool: Pool,
    /// 33-byte secp256k1 compressed pubkey identifying the tenant. All reads
    /// and writes are filtered by `user_id = self.identity`.
    identity: Vec<u8>,
}

#[async_trait]
impl SessionStore for PostgresSessionStore {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionStoreError> {
        let client = self.pool.get().await.map_err(map_err)?;
        let service_key = service_identity_key.serialize().to_vec();
        let row = client
            .query_opt(
                r"SELECT token, expiration FROM brz_sessions
                  WHERE user_id = $1 AND service_identity_key = $2",
                &[&self.identity, &service_key],
            )
            .await
            .map_err(map_err)?;
        let row = row.ok_or(SessionStoreError::NotFound)?;
        let token: String = row.get(0);
        let expiration: i64 = row.get(1);
        let expiration = u64::try_from(expiration)
            .map_err(|e| SessionStoreError::Generic(format!("invalid expiration: {e}")))?;
        Ok(Session { token, expiration })
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionStoreError> {
        let client = self.pool.get().await.map_err(map_err)?;
        let service_key = service_identity_key.serialize().to_vec();
        let expiration = i64::try_from(session.expiration)
            .map_err(|e| SessionStoreError::Generic(format!("expiration overflow: {e}")))?;
        client
            .execute(
                r"INSERT INTO brz_sessions (user_id, service_identity_key, token, expiration)
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

impl PostgresSessionStore {
    /// Creates a new `PostgresSessionStore` from a configuration.
    ///
    /// Allocates its own connection pool and runs migrations.
    /// `identity` is the 33-byte secp256k1 pubkey of the tenant.
    pub async fn from_config(
        config: PostgresStorageConfig,
        identity: &[u8],
    ) -> Result<Self, PostgresError> {
        let run_migration = config.run_migration;
        let pool = create_pool(&config)?;
        Self::from_pool(pool, identity, run_migration).await
    }

    /// Creates a new `PostgresSessionStore` from an existing connection pool.
    ///
    /// Useful when sharing a pool with other components (e.g., `PostgresStorage`).
    /// When `run_migration` is `false`, initialization trusts the existing
    /// schema and skips session store migrations entirely.
    pub async fn from_pool(
        pool: Pool,
        identity: &[u8],
        run_migration: bool,
    ) -> Result<Self, PostgresError> {
        let store = Self {
            pool,
            identity: identity.to_vec(),
        };
        if run_migration {
            store.migrate().await?;
        }
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), PostgresError> {
        run_migrations(
            &self.pool,
            SESSION_MIGRATIONS_TABLE,
            &Self::migrations(),
            Some(&SCHEMA_RENAMES),
        )
        .await
    }

    fn migrations() -> Vec<Vec<String>> {
        vec![vec![
            "CREATE TABLE IF NOT EXISTS brz_sessions (
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

fn map_err<E: std::fmt::Display>(e: E) -> SessionStoreError {
    SessionStoreError::Generic(e.to_string())
}

/// Creates a `PostgresSessionStore` from a configuration.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
pub async fn create_postgres_session_store(
    config: PostgresStorageConfig,
    identity: &[u8],
) -> Result<Arc<dyn SessionStore>, PostgresError> {
    Ok(Arc::new(
        PostgresSessionStore::from_config(config, identity).await?,
    ))
}

/// Creates a `PostgresSessionStore` from an existing connection pool.
///
/// `identity` is the 33-byte secp256k1 pubkey scoping all reads and writes.
/// When `run_migration` is `false`, skips SDK-managed schema migrations and
/// trusts that the `sessions` table already exists.
pub async fn create_postgres_session_store_from_pool(
    pool: Pool,
    identity: &[u8],
    run_migration: bool,
) -> Result<Arc<dyn SessionStore>, PostgresError> {
    Ok(Arc::new(
        PostgresSessionStore::from_pool(pool, identity, run_migration).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_wallet::session_store_tests as shared_tests;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::postgres::Postgres;

    /// Fixed 33-byte test identity. Tests run in their own ephemeral container,
    /// so a single shared identity is fine — the schema still gets exercised.
    const TEST_IDENTITY: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];

    struct PostgresSessionStoreTestFixture {
        store: PostgresSessionStore,
        #[allow(dead_code)]
        container: ContainerAsync<Postgres>,
    }

    impl PostgresSessionStoreTestFixture {
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

            let store = PostgresSessionStore::from_config(
                PostgresStorageConfig::with_defaults(connection_string),
                &TEST_IDENTITY,
            )
            .await
            .expect("Failed to create PostgresSessionStore");

            Self { store, container }
        }
    }

    #[tokio::test]
    async fn test_get_session_not_found() {
        let fixture = PostgresSessionStoreTestFixture::new().await;
        shared_tests::test_get_session_not_found(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_set_and_get() {
        let fixture = PostgresSessionStoreTestFixture::new().await;
        shared_tests::test_set_and_get(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_overwrite_session() {
        let fixture = PostgresSessionStoreTestFixture::new().await;
        shared_tests::test_overwrite_session(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_sessions_are_isolated_by_key() {
        let fixture = PostgresSessionStoreTestFixture::new().await;
        shared_tests::test_sessions_are_isolated_by_key(&fixture.store).await;
    }

    #[tokio::test]
    async fn test_get_after_unrelated_set() {
        let fixture = PostgresSessionStoreTestFixture::new().await;
        shared_tests::test_get_after_unrelated_set(&fixture.store).await;
    }
}
