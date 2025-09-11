use sqlx::SqlitePool;

use crate::repository::LnurlRepositoryError;

mod repository;

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), LnurlRepositoryError> {
    let migrator = sqlx::migrate!("migrations/sqlite");
    Ok(migrator.run(pool).await?)
}

pub use repository::LnurlRepository;
