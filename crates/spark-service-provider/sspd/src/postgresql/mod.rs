mod chain_repository;
mod pool_store;

pub use chain_repository::ChainRepository;
pub use pool_store::PoolRepository;
use sqlx::{Pool, Postgres};

pub async fn migrate(pool: &Pool<Postgres>) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("src/postgresql/migrations").run(pool).await
}
