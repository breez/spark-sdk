use std::sync::Arc;

use sqlx::PgPool;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Docker Desktop on macOS gives a container a host port without checking
/// what the host listens on. Now and then a container gets no port, or the
/// test reaches another program on that port.
const ATTEMPTS: u32 = 3;

pub(crate) struct TestDatabase {
    pub(crate) pool: Arc<PgPool>,
    _postgres: ContainerAsync<Postgres>,
}

impl TestDatabase {
    pub(crate) async fn new() -> Self {
        for _ in 1..ATTEMPTS {
            match Self::start().await {
                Ok(database) => return database,
                Err(e) => eprintln!("no Postgres database for the test, trying again: {e}"),
            }
        }
        Self::start()
            .await
            .unwrap_or_else(|e| panic!("no Postgres database for the test: {e}"))
    }

    async fn start() -> Result<Self, BoxError> {
        let postgres = Postgres::default().start().await?;
        let port = postgres.get_host_port_ipv4(5432).await?;
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let pool = Arc::new(PgPool::connect(&url).await?);
        super::migrate(&pool).await?;
        Ok(Self {
            pool,
            _postgres: postgres,
        })
    }
}
