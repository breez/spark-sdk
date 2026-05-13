//! Shareable Postgres connection pool wrapper.

use std::sync::Arc;

use spark_postgres::deadpool_postgres;

use crate::error::SdkError;

use super::{PostgresStorageConfig, base::create_pool};

/// A shareable Postgres connection pool.
///
/// Construct via [`create_postgres_connection_pool`] and pass the same `Arc` to multiple
/// [`SdkBuilder`](crate::SdkBuilder)s via
/// [`SdkBuilder::with_postgres_connection_pool`](crate::SdkBuilder::with_postgres_connection_pool).
/// All SDKs sharing a pool target the same database; per-tenant isolation is
/// derived from each SDK's seed (the identity public key scopes every row).
///
/// The pool's lifecycle is owned by the integrator: it stays alive as long
/// as any `Arc<PostgresConnectionPool>` is held. [`BreezSdk::disconnect`](crate::BreezSdk::disconnect)
/// does **not** close the pool.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct PostgresConnectionPool {
    pub(crate) inner: deadpool_postgres::Pool,
    pub(crate) run_migration: bool,
}

/// Creates a shareable Postgres connection pool from the given configuration.
///
/// Hand the returned `Arc` to one or more
/// [`SdkBuilder::with_postgres_connection_pool`](crate::SdkBuilder::with_postgres_connection_pool)
/// calls to share a single pool across multiple SDK instances.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn create_postgres_connection_pool(
    config: &PostgresStorageConfig,
) -> Result<Arc<PostgresConnectionPool>, SdkError> {
    let inner = create_pool(config).map_err(SdkError::from)?;
    Ok(Arc::new(PostgresConnectionPool {
        inner,
        run_migration: config.run_migration,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pool creation builds a deadpool eagerly but does not connect to the
    /// server until first use, so this works against a non-existent host.
    /// Verifies `Arc::clone` semantics — a single factory call yields one
    /// pool that can be cheaply cloned for sharing.
    #[test]
    fn pool_arc_is_cheaply_shareable() {
        let cfg =
            default_postgres_storage_config("host=localhost port=5432 user=u dbname=d".to_string());
        let pool = create_postgres_connection_pool(&cfg).expect("build pool");
        assert_eq!(Arc::strong_count(&pool), 1);

        let clone_a = Arc::clone(&pool);
        let clone_b = Arc::clone(&pool);
        assert_eq!(Arc::strong_count(&pool), 3);

        drop(clone_a);
        assert_eq!(Arc::strong_count(&pool), 2);
        drop(clone_b);
        assert_eq!(Arc::strong_count(&pool), 1);
    }

    fn default_postgres_storage_config(connection_string: String) -> PostgresStorageConfig {
        super::super::default_postgres_storage_config(connection_string)
    }
}
