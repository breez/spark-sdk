use sqlx::PgPool;

use crate::repository::LnurlRepositoryError;

mod repository;

pub async fn run_migrations(pool: &PgPool) -> Result<(), LnurlRepositoryError> {
    Ok(sqlx::migrate!("migrations/postgres").run(pool).await?)
}

pub use repository::LnurlRepository;
