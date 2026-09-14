mod chain_repository;

pub use chain_repository::ChainRepository;
use sqlx::{Pool, Postgres};

pub async fn migrate(pool: &Pool<Postgres>) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("src/postgresql/migrations").run(pool).await
}
