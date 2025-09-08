use sqlx::SqlitePool;

use crate::repository::LnurlRepositoryError;

mod repository;

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), LnurlRepositoryError> {
    Ok(sqlx::migrate!("migrations/sqlite").run(pool).await?)
}

pub use repository::LnurlRepository;
