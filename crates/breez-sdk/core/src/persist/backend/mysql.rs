//! `MySQL`-backed storage backend.

use std::sync::Arc;

use macros::async_trait;
use spark_mysql::mysql_async;
use spark_wallet::PublicKey;

use crate::{
    SdkError,
    persist::mysql::{
        MysqlForeignKeyMode, MysqlStorage, create_mysql_session_store, create_mysql_token_store,
        create_mysql_tree_store,
    },
};

use super::{ResolvedStores, StorageBackend};

/// `MySQL` backend. The connection pool may be shared across many SDK
/// instances; each [`create_stores`](StorageBackend::create_stores) call scopes
/// its stores to one tenant identity.
pub(super) struct MysqlBackend {
    pool: mysql_async::Pool,
    run_migration: bool,
    foreign_key_mode: MysqlForeignKeyMode,
}

impl MysqlBackend {
    pub(super) fn new(
        pool: mysql_async::Pool,
        run_migration: bool,
        foreign_key_mode: MysqlForeignKeyMode,
    ) -> Self {
        Self {
            pool,
            run_migration,
            foreign_key_mode,
        }
    }
}

#[async_trait]
impl StorageBackend for MysqlBackend {
    async fn create_stores(&self, identity: &PublicKey) -> Result<ResolvedStores, SdkError> {
        let identity = identity.serialize();
        let storage = Arc::new(
            MysqlStorage::new_with_pool(self.pool.clone(), &identity, self.run_migration).await?,
        );
        let tree_store = create_mysql_tree_store(
            self.pool.clone(),
            &identity,
            self.run_migration,
            self.foreign_key_mode,
        )
        .await?;
        let token_output_store = create_mysql_token_store(
            self.pool.clone(),
            &identity,
            self.run_migration,
            self.foreign_key_mode,
        )
        .await?;
        let session_store =
            create_mysql_session_store(self.pool.clone(), &identity, self.run_migration).await?;
        Ok(ResolvedStores {
            storage,
            tree_store: Some(tree_store),
            token_output_store: Some(token_output_store),
            session_store: Some(session_store),
        })
    }
}
