mod chain_repository;
mod pool_store;
mod swap_store;

pub use chain_repository::ChainRepository;
pub use pool_store::PoolRepository;
use sqlx::{Pool, Postgres};
pub use swap_store::SwapRepository;

pub async fn migrate(pool: &Pool<Postgres>) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("src/postgresql/migrations").run(pool).await
}
