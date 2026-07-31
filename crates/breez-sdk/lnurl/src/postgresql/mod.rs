use spark_postgres::deadpool_postgres::Pool;

use crate::repository::LnurlRepositoryError;

mod migrations;
mod repository;

pub async fn run_migrations(pool: &Pool) -> Result<(), LnurlRepositoryError> {
    migrations::run(pool).await?;
    Ok(())
}

pub use repository::LnurlRepository;
