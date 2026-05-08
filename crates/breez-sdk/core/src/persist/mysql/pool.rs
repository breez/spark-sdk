//! Shareable `MySQL` connection pool wrapper.

use std::sync::Arc;

use spark_mysql::mysql_async;

use crate::error::SdkError;

use super::{MysqlStorageConfig, base::create_pool};

/// A shareable `MySQL` connection pool. See
/// [`PostgresConnectionPool`](crate::PostgresConnectionPool) for sharing semantics and lifecycle.
///
// TODO(post-#865): once https://github.com/breez/spark-sdk/pull/865 lands,
// add `foreign_key_mode: MysqlForeignKeyMode` here, snapshot it from
// `config.foreign_key_mode` in `create_mysql_connection_pool`, and forward it from the
// SDK builder to `create_mysql_tree_store` / `create_mysql_token_store`.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct MysqlConnectionPool {
    pub(crate) inner: mysql_async::Pool,
    pub(crate) schema_managed_externally: bool,
}

/// Creates a shareable `MySQL` connection pool from the given configuration.
///
/// Hand the returned `Arc` to one or more
/// [`SdkBuilder::with_mysql_connection_pool`](crate::SdkBuilder::with_mysql_connection_pool)
/// calls to share a single pool across multiple SDK instances.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn create_mysql_connection_pool(
    config: &MysqlStorageConfig,
) -> Result<Arc<MysqlConnectionPool>, SdkError> {
    let inner = create_pool(config).map_err(SdkError::from)?;
    Ok(Arc::new(MysqlConnectionPool {
        inner,
        schema_managed_externally: config.schema_managed_externally,
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
