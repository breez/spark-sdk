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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::{
        StorageListPaymentsRequest,
        postgres::{PostgresStorageConfig, create_pool},
    };
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use spark_wallet::SessionStoreError;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    /// Two distinct 33-byte tenant identities sharing one pool / database.
    const TENANT_A: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];
    const TENANT_B: [u8; 33] = [
        0x03, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae,
        0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd,
        0xbe, 0xbf, 0xc0,
    ];

    fn dummy_pubkey() -> PublicKey {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x11; 32]).expect("valid secret key");
        PublicKey::from_secret_key(&secp, &sk)
    }

    /// Asserts every store in `stores` is present and backed by a fully migrated
    /// schema by issuing a trivial read against each of the four tables. This is
    /// what guards the fix: per-tenant stores are built migration-free, so each
    /// of these reads only succeeds if `ensure_migrated` already created that
    /// store's schema.
    async fn assert_all_stores_usable(stores: &ResolvedStores) {
        stores
            .storage
            .list_payments(StorageListPaymentsRequest::default())
            .await
            .expect("main storage schema migrated");

        let tree = stores.tree_store.as_ref().expect("tree store present");
        tree.get_leaves().await.expect("tree store schema migrated");

        let tokens = stores
            .token_output_store
            .as_ref()
            .expect("token store present");
        tokens
            .list_tokens_outputs()
            .await
            .expect("token store schema migrated");

        let sessions = stores
            .session_store
            .as_ref()
            .expect("session store present");
        // No session has been written, so a *migrated* (but empty) schema returns
        // `NotFound` because the query ran and matched no row. A missing table
        // would surface as a different error, failing this assertion.
        match sessions.get_session(&dummy_pubkey()).await {
            Err(SessionStoreError::NotFound) => {}
            Err(e) => {
                panic!("expected NotFound from migrated empty session store, got error {e:?}")
            }
            Ok(_) => panic!("unexpected session returned from empty session store"),
        }
    }

    /// With `run_migration = true`, the backend migrates the shared database once
    /// via `ensure_migrated`, then builds each tenant's stores migration-free.
    /// This exercises that path end-to-end: concurrent first-callers race into
    /// the `OnceCell`, all four stores' migrations must be covered (so the
    /// migration-free per-tenant builds see a complete schema), and two tenants
    /// are provisioned over the same pool.
    #[tokio::test]
    async fn test_create_stores_migrates_once_for_all_tenants() {
        let container = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");
        let connection_string = format!(
            "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
        );
        let pool = create_pool(&PostgresStorageConfig::with_defaults(connection_string))
            .expect("Failed to create pool");

        let backend = PostgresBackend::new(pool, true);

        // Provision both tenants concurrently: the first to reach `ensure_migrated`
        // runs the migrations while the other awaits the same `OnceCell`.
        let (a, b) = tokio::join!(
            backend.create_stores(Network::Regtest, TENANT_A.to_vec()),
            backend.create_stores(Network::Regtest, TENANT_B.to_vec()),
        );
        let a = a.expect("tenant A stores created");
        let b = b.expect("tenant B stores created");

        assert_all_stores_usable(&a).await;
        assert_all_stores_usable(&b).await;
    }
}
