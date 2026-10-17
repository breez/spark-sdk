mod chain_outputs;
mod chain_repository;
mod coop_exit_store;
mod instant_quote_store;
mod leaf_store;
mod lightning_store;
mod pool_store;
mod static_deposit_address_store;
mod static_deposit_store;
mod swap_store;
#[cfg(test)]
pub(crate) mod testing;
mod unilateral_exit_store;

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
pub use pool_store::PoolRepository;
use sqlx::{Pool, Postgres};
pub use static_deposit_address_store::PostgresStaticDepositAddressStore;
pub use static_deposit_store::PostgresStaticDepositClaimStore;
pub use swap_store::SwapRepository;
pub use unilateral_exit_store::{
    PostgresUnilateralExitStore, StoredCpfpChild, UnilateralExitStoreError,
};

pub async fn migrate(pool: &Pool<Postgres>) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("src/postgresql/migrations").run(pool).await
}
