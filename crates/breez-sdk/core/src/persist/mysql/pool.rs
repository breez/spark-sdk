//! Shareable `MySQL` connection pool wrapper.

use std::sync::Arc;

use spark_mysql::mysql_async;

use crate::error::SdkError;

use super::{MysqlForeignKeyMode, MysqlStorageConfig, base::create_pool};

/// A shareable `MySQL` connection pool. See
/// [`PostgresConnectionPool`](crate::PostgresConnectionPool) for sharing semantics and lifecycle.
///
/// Snapshots the `foreign_key_mode` from the originating config so every SDK
/// instance built on top of this pool migrates with the same FK policy.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct MysqlConnectionPool {
    pub(crate) inner: mysql_async::Pool,
    pub(crate) run_migration: bool,
    pub(crate) foreign_key_mode: MysqlForeignKeyMode,
}

/// Creates a shareable `MySQL` connection pool from the given configuration.
///
/// Used internally by [`new_sdk_context`](crate::new_sdk_context). Exposed
/// for advanced use cases where a caller wants the pool itself.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn create_mysql_connection_pool(
    config: &MysqlStorageConfig,
) -> Result<Arc<MysqlConnectionPool>, SdkError> {
    let inner = create_pool(config).map_err(SdkError::from)?;
    Ok(Arc::new(MysqlConnectionPool {
        inner,
        run_migration: config.run_migration,
        foreign_key_mode: config.foreign_key_mode,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pool creation parses the URL and builds a `mysql_async::Pool` lazily,
    /// so this works without a server. Verifies `Arc::clone` semantics — a
    /// single factory call yields one pool that can be cheaply cloned.
    #[test]
    fn pool_arc_is_cheaply_shareable() {
        let cfg = default_mysql_storage_config("mysql://u:p@127.0.0.1:3306/d".to_string());
        let pool = create_mysql_connection_pool(&cfg).expect("build pool");
        assert_eq!(Arc::strong_count(&pool), 1);

        let clone_a = Arc::clone(&pool);
        let clone_b = Arc::clone(&pool);
        assert_eq!(Arc::strong_count(&pool), 3);

        drop(clone_a);
        assert_eq!(Arc::strong_count(&pool), 2);
        drop(clone_b);
        assert_eq!(Arc::strong_count(&pool), 1);
    }

    fn default_mysql_storage_config(connection_string: String) -> MysqlStorageConfig {
        super::super::default_mysql_storage_config(connection_string)
    }
}
