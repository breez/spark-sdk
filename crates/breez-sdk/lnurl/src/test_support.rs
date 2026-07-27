//! Shared fixtures for the postgres-backed tests.

use std::sync::atomic::{AtomicU32, Ordering};

use spark_postgres::deadpool_postgres::{Manager, Pool};
use spark_postgres::tokio_postgres::{Config, NoTls};

/// Connection string to the throwaway postgres instance the tests run against.
/// The tests create and drop schemas in it, so it must not point at real data.
const URL_ENV: &str = "LNURL_TEST_POSTGRES_URL";

static SCHEMA_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A migrated pool from [`empty_test_pool`].
pub async fn test_pool(label: &str) -> Pool {
    let pool = empty_test_pool(label).await;
    crate::postgresql::run_migrations(&pool)
        .await
        .expect("run migrations");
    pool
}

/// A pool confined to its own freshly created, empty schema, so tests sharing
/// one postgres instance never see each other's rows. `label` only has to be
/// recognizable in a failure message: uniqueness comes from a counter.
///
/// Panics when `LNURL_TEST_POSTGRES_URL` is unset. Skipping instead would leave
/// the whole repository suite passing without ever touching a database.
pub async fn empty_test_pool(label: &str) -> Pool {
    let url = std::env::var(URL_ENV).unwrap_or_else(|_| {
        panic!(
            "{URL_ENV} is not set. Point it at a disposable postgres instance, \
             e.g. LNURL_TEST_POSTGRES_URL=postgres://postgres:postgres@localhost/lnurl_test"
        )
    });

    let n = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut schema = format!("t{n}_{label}");
    // Postgres truncates identifiers past 63 bytes, which would silently merge
    // two schemas whose names share a long prefix.
    schema.truncate(63);

    let config: Config = url.parse().expect("parse postgres url");
    let admin = connect(&config).await;
    admin
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS \"{schema}\" CASCADE; CREATE SCHEMA \"{schema}\""
        ))
        .await
        .expect("recreate test schema");
    drop(admin);

    // Pins every pooled connection to the test schema at startup, which is the
    // deadpool equivalent of a per-connection `SET search_path`.
    let mut scoped = config;
    scoped.options(format!("-c search_path={schema}"));
    let manager = Manager::new(scoped, NoTls);
    Pool::builder(manager)
        .max_size(5)
        .build()
        .expect("build test pool")
}

/// Connects a standalone client, driving its connection on a background task.
async fn connect(config: &Config) -> spark_postgres::tokio_postgres::Client {
    let (client, connection) = config
        .connect(NoTls)
        .await
        .expect("connect to test postgres");
    tokio::spawn(connection);
    client
}

/// A repository over a pool from [`test_pool`].
pub async fn test_db(label: &str) -> crate::postgresql::LnurlRepository {
    crate::postgresql::LnurlRepository::new(test_pool(label).await)
}
