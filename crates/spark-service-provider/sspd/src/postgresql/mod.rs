mod chain_repository;
mod coop_exit_store;
mod lightning_store;
mod pool_store;
mod swap_store;

pub use chain_repository::ChainRepository;
pub use coop_exit_store::PostgresCoopExitStore;
pub use lightning_store::PostgresLightningStore;
pub use pool_store::PoolRepository;
use sqlx::{Pool, Postgres};
pub use swap_store::SwapRepository;

pub async fn migrate(pool: &Pool<Postgres>) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("src/postgresql/migrations").run(pool).await
}
