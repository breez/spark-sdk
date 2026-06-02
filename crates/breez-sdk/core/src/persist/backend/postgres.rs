//! `PostgreSQL`-backed storage backend.

use std::sync::Arc;

use macros::async_trait;
use spark_postgres::deadpool_postgres;
use tokio::sync::OnceCell;

use crate::{
    Network, SdkError,
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
    /// Ensures the schema migrations run at most once per pool.
    ///
    /// The migrations are global to the database: schema-level, version-tracked
    /// `brz_*` tables, not per-tenant data. Each `run_migrations` call also
    /// takes a *global* `pg_advisory_xact_lock` while holding a pooled
    /// connection. With a fresh store set built per request (as a multi-tenant
    /// server does), running them on every `create_stores` serializes every
    /// build across every tenant behind that lock and starves the pool under
    /// load. Gating behind a `OnceCell` runs them once per process; the global
    /// lock still serializes the first run across processes, so two instances
    /// booting together can't double-migrate.
    migrated: OnceCell<()>,
}

impl PostgresBackend {
    pub(super) fn new(pool: deadpool_postgres::Pool, run_migration: bool) -> Self {
        Self {
            pool,
            run_migration,
            migrated: OnceCell::new(),
        }
    }

    /// Runs every store's migrations exactly once per pool. Concurrent
    /// first-callers await the same run; a failure is not cached, so a later
    /// build retries.
    async fn ensure_migrated(&self, identity: &[u8]) -> Result<(), SdkError> {
        self.migrated
            .get_or_try_init(|| async {
                // Construct each store once with migrations enabled, then drop
                // it — we only want the migration side effect here. `identity`
                // feeds only the one-shot single-tenant backfill migration,
                // which is version-gated and runs once per database regardless
                // of which tenant happens to trigger it first.
                PostgresStorage::new_with_pool(self.pool.clone(), identity, true).await?;
                create_postgres_tree_store(self.pool.clone(), identity, true).await?;
                create_postgres_token_store(self.pool.clone(), identity, true).await?;
                create_postgres_session_store(self.pool.clone(), identity, true).await?;
                Ok::<(), SdkError>(())
            })
            .await
            .copied()
    }
}

#[async_trait]
impl StorageBackend for PostgresBackend {
    async fn create_stores(
        &self,
        _network: Network,
        identity: Vec<u8>,
    ) -> Result<Arc<ResolvedStores>, SdkError> {
        if self.run_migration {
            self.ensure_migrated(&identity).await?;
        }
        // Migrations are handled once per pool by `ensure_migrated`; the stores
        // themselves are built migration-free so a per-request build never
        // re-runs (and re-locks) them.
        let storage =
            Arc::new(PostgresStorage::new_with_pool(self.pool.clone(), &identity, false).await?);
        let tree_store = create_postgres_tree_store(self.pool.clone(), &identity, false).await?;
        let token_output_store =
            create_postgres_token_store(self.pool.clone(), &identity, false).await?;
        let session_store =
            create_postgres_session_store(self.pool.clone(), &identity, false).await?;
        Ok(Arc::new(ResolvedStores {
            storage,
            tree_store: Some(tree_store),
            token_output_store: Some(token_output_store),
            session_store: Some(session_store),
        }))
    }
}
