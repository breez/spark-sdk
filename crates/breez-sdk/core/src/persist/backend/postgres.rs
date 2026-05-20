//! `PostgreSQL`-backed storage backend.

use std::sync::Arc;

use macros::async_trait;
use spark_postgres::deadpool_postgres;
use spark_wallet::PublicKey;

use crate::{
    SdkError,
    persist::postgres::{
        PostgresStorage, create_postgres_session_store, create_postgres_token_store,
        create_postgres_tree_store,
    },
};

use super::{ResolvedStores, StorageBackend};

/// `PostgreSQL` backend. The connection pool may be shared across many SDK
/// instances; each [`create_stores`](StorageBackend::create_stores) call scopes
/// its stores to one tenant identity.
pub(super) struct PostgresBackend {
    pool: deadpool_postgres::Pool,
    run_migration: bool,
}

impl PostgresBackend {
    pub(super) fn new(pool: deadpool_postgres::Pool, run_migration: bool) -> Self {
        Self {
            pool,
            run_migration,
        }
    }
}

#[async_trait]
impl StorageBackend for PostgresBackend {
    async fn create_stores(&self, identity: &PublicKey) -> Result<ResolvedStores, SdkError> {
        let identity = identity.serialize();
        let storage = Arc::new(
            PostgresStorage::new_with_pool(self.pool.clone(), &identity, self.run_migration)
                .await?,
        );
        let tree_store =
            create_postgres_tree_store(self.pool.clone(), &identity, self.run_migration).await?;
        let token_output_store =
            create_postgres_token_store(self.pool.clone(), &identity, self.run_migration).await?;
        let session_store =
            create_postgres_session_store(self.pool.clone(), &identity, self.run_migration).await?;
        Ok(ResolvedStores {
            storage,
            tree_store: Some(tree_store),
            token_output_store: Some(token_output_store),
            session_store: Some(session_store),
        })
    }
}
