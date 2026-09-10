//! Shared fixtures for the postgres-backed tests.

use spark_postgres::deadpool_postgres::Pool;
use spark_postgres::{PostgresStorageConfig, create_pool};
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

/// A pool over a throwaway postgres, migrated and empty of rows. The container
/// is returned with it and stops as soon as it is dropped, so a test has to
/// hold on to it for as long as it uses the pool.
pub async fn test_pool() -> (ContainerAsync<Postgres>, Pool) {
    let (container, pool) = empty_test_pool().await;
    crate::postgresql::run_migrations(&pool)
        .await
        .expect("run migrations");
    (container, pool)
}

/// A pool over a throwaway postgres with no schema applied.
pub async fn empty_test_pool() -> (ContainerAsync<Postgres>, Pool) {
    let container = Postgres::default()
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get container port");

    let pool = create_pool(&PostgresStorageConfig::with_defaults(format!(
        "host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres"
    )))
    .expect("create test pool");

    (container, pool)
}

/// A repository over a pool from [`test_pool`].
pub async fn test_db() -> (ContainerAsync<Postgres>, crate::postgresql::LnurlRepository) {
    let (container, pool) = test_pool().await;
    (container, crate::postgresql::LnurlRepository::new(pool))
}
