//! Shared fixtures for the postgres-backed tests.

use std::sync::atomic::{AtomicU32, Ordering};

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, Executor, PgConnection, PgPool};

/// Connection string to the throwaway postgres instance the tests run against.
/// The tests create and drop schemas in it, so it must not point at real data.
const URL_ENV: &str = "LNURL_TEST_POSTGRES_URL";

static SCHEMA_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A migrated pool confined to its own freshly created schema, so tests sharing
/// one postgres instance never see each other's rows. `label` only has to be
/// recognizable in a failure message: uniqueness comes from a counter.
///
/// Panics when `LNURL_TEST_POSTGRES_URL` is unset. Skipping instead would leave
/// the whole repository suite passing without ever touching a database.
pub async fn test_pool(label: &str) -> PgPool {
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

    let mut admin = PgConnection::connect(&url)
        .await
        .expect("connect to test postgres");
    admin
        .execute(format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE").as_str())
        .await
        .expect("drop stale test schema");
    admin
        .execute(format!("CREATE SCHEMA \"{schema}\"").as_str())
        .await
        .expect("create test schema");
    admin.close().await.expect("close admin connection");

    let options: PgConnectOptions = url.parse().expect("parse postgres url");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .after_connect(move |conn, _| {
            let schema = schema.clone();
            Box::pin(async move {
                conn.execute(format!("SET search_path TO \"{schema}\"").as_str())
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .expect("connect test pool");

    crate::postgresql::run_migrations(&pool)
        .await
        .expect("run migrations");
    pool
}

/// A repository over a pool from [`test_pool`].
pub async fn test_db(label: &str) -> crate::postgresql::LnurlRepository {
    crate::postgresql::LnurlRepository::new(test_pool(label).await)
}
