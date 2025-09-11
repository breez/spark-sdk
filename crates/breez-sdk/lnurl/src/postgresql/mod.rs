use sqlx::PgPool;

use crate::repository::LnurlRepositoryError;

mod repository;

pub async fn run_migrations(pool: &PgPool) -> Result<(), LnurlRepositoryError> {
    let migrator = sqlx::migrate!("migrations/postgres");
    Ok(migrator.run(pool).await?)
}

pub use repository::LnurlRepository;
