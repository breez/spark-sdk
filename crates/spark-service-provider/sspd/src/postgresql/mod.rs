mod chain_outputs;
mod chain_repository;
mod coop_exit_store;
mod instant_quote_store;
mod leaf_store;
mod lightning_store;
mod migrations;
mod pool_store;
mod static_deposit_address_store;
mod static_deposit_store;
mod swap_store;
#[cfg(test)]
pub(crate) mod testing;
mod unilateral_exit_store;

use std::sync::Arc;
use std::time::Duration;

use deadpool_postgres::{BuildError, GenericClient, Manager, Pool, PoolError, Runtime};
use rustls::crypto::ring;
use rustls::{ClientConfig, RootCertStore};
use spark::services::TransferId;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use tokio_postgres_rustls::MakeRustlsConnect;
use uuid::Uuid;

pub use chain_repository::ChainRepository;
pub use coop_exit_store::PostgresCoopExitStore;
pub use instant_quote_store::PostgresInstantStaticDepositQuoteStore;
pub use leaf_store::MatureTransaction;
pub use leaf_store::RefundOutput;
pub use leaf_store::{ChainTransaction, StoredChains};
pub use leaf_store::{
    Dispute, DisputeDetails, ExitChainStep, LeafStoreError, PoolCounts, PostgresLeafStore,
    RenewalCursor, Reported, Reservation, exit_chain, exiting_steps, refund_step,
};
pub use lightning_store::PostgresLightningStore;
pub use migrations::migrate;
pub use pool_store::PoolRepository;
pub use static_deposit_address_store::PostgresStaticDepositAddressStore;
pub use static_deposit_store::PostgresStaticDepositClaimStore;
pub use swap_store::SwapRepository;
pub use unilateral_exit_store::{
    PostgresUnilateralExitStore, StoredCpfpChild, UnilateralExitStoreError,
};

/// How long opening a database connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a caller waits for a database connection while all of them are in use.
const POOL_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("invalid database URL: {0}")]
    InvalidUrl(tokio_postgres::Error),

    #[error("failed to set up TLS: {0}")]
    Tls(#[from] rustls::Error),

    #[error("failed to set up the connection pool: {0}")]
    Pool(#[from] BuildError),

    #[error("no database connection: {0}")]
    Connection(#[from] PoolError),

    #[error("database error: {0}")]
    Database(#[from] tokio_postgres::Error),

    #[error("migration {version} failed: {source}")]
    Migration {
        version: i32,
        source: tokio_postgres::Error,
    },

    #[error("only an sspd that used sqlx has migrated this database")]
    MigratedBySqlx,
}

/// Opens a pool of at most `max_connections` connections to the database at `url`.
///
/// In `url`, `sslmode` is `disable`, `prefer` (the default) or `require`. Unlike
/// libpq, sspd verifies the server's certificate and host name against Mozilla's
/// root certificates whenever it uses TLS.
pub async fn connect(url: &str, max_connections: usize) -> Result<Pool, DatabaseError> {
    let config: tokio_postgres::Config = url.parse().map_err(DatabaseError::InvalidUrl)?;
    let tls = MakeRustlsConnect::new(tls_config()?);
    let pool = Pool::builder(Manager::new(config, tls))
        .max_size(max_connections)
        .create_timeout(Some(CONNECT_TIMEOUT))
        .wait_timeout(Some(POOL_WAIT_TIMEOUT))
        .runtime(Runtime::Tokio1)
        .build()?;
    // The pool opens a connection only when a caller needs one, so connect takes
    // one here and fails when sspd cannot reach the database.
    drop(pool.get().await?);
    Ok(pool)
}

fn tls_config() -> Result<ClientConfig, rustls::Error> {
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    Ok(
        ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

pub(crate) fn transfer_uuid(id: &TransferId) -> Uuid {
    // `TransferId::to_bytes` has the UUID's fields in little-endian order.
    Uuid::from_bytes_le(id.to_bytes())
}

// Each of these prepares `sql` once per connection and then runs it with `params`.

pub(crate) async fn query(
    db: &impl GenericClient,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, tokio_postgres::Error> {
    let statement = db.prepare_cached(sql).await?;
    db.query(&statement, params).await
}

pub(crate) async fn query_one(
    db: &impl GenericClient,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Row, tokio_postgres::Error> {
    let statement = db.prepare_cached(sql).await?;
    db.query_one(&statement, params).await
}

pub(crate) async fn query_opt(
    db: &impl GenericClient,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Option<Row>, tokio_postgres::Error> {
    let statement = db.prepare_cached(sql).await?;
    db.query_opt(&statement, params).await
}

pub(crate) async fn execute(
    db: &impl GenericClient,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<u64, tokio_postgres::Error> {
    let statement = db.prepare_cached(sql).await?;
    db.execute(&statement, params).await
}

#[cfg(test)]
mod tests {
    use spark::services::TransferId;

    use super::testing::TestDatabase;
    use super::{DatabaseError, migrate};

    #[test]
    fn a_transfer_id_is_stored_as_its_own_uuid() {
        let id = TransferId::generate();
        assert_eq!(super::transfer_uuid(&id).to_string(), id.to_string());
    }

    #[tokio::test]
    async fn sspd_does_not_migrate_a_database_that_sqlx_migrated() {
        let database = TestDatabase::new().await;
        let client = database.pool.get().await.unwrap();
        client
            .batch_execute(
                "DROP TABLE brz_ssp_schema_migrations;
                 CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY)",
            )
            .await
            .unwrap();

        let migrated = migrate(&database.pool).await;

        assert!(
            matches!(migrated, Err(DatabaseError::MigratedBySqlx)),
            "{migrated:?}"
        );
    }

    #[tokio::test]
    async fn sspd_migrates_a_database_with_its_own_record_beside_a_sqlx_record() {
        let database = TestDatabase::new().await;
        let client = database.pool.get().await.unwrap();
        client
            .batch_execute("CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY)")
            .await
            .unwrap();

        migrate(&database.pool).await.unwrap();
    }
}
