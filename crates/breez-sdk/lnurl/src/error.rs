use sqlx::migrate::MigrateError;

use crate::repository::LnurlRepositoryError;

impl From<sqlx::Error> for LnurlRepositoryError {
    fn from(err: sqlx::Error) -> Self {
        if let sqlx::Error::Database(database_error) = &err
            && database_error.is_unique_violation()
        {
            return LnurlRepositoryError::NameTaken;
        }

        LnurlRepositoryError::General(err.into())
    }
}
impl From<MigrateError> for LnurlRepositoryError {
    fn from(err: MigrateError) -> Self {
        LnurlRepositoryError::General(err.into())
    }
}
