use spark_postgres::deadpool_postgres::PoolError;
use spark_postgres::tokio_postgres::error::SqlState;
use spark_postgres::{PostgresError, tokio_postgres};

use crate::repository::LnurlRepositoryError;

impl From<tokio_postgres::Error> for LnurlRepositoryError {
    fn from(err: tokio_postgres::Error) -> Self {
        if err.code() == Some(&SqlState::UNIQUE_VIOLATION) {
            return LnurlRepositoryError::NameTaken;
        }

        LnurlRepositoryError::General(err.into())
    }
}

impl From<PoolError> for LnurlRepositoryError {
    fn from(err: PoolError) -> Self {
        LnurlRepositoryError::General(err.into())
    }
}

impl From<PostgresError> for LnurlRepositoryError {
    fn from(err: PostgresError) -> Self {
        LnurlRepositoryError::General(err.into())
    }
}
