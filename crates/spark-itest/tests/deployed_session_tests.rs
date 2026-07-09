use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use spark_itest::helpers::create_regtest_wallet_with_session_store;
use spark_wallet::{InMemorySessionStore, PublicKey, Session, SessionStore, SessionStoreError};
use tracing::info;

/// A session token that parses as a valid header value but which no operator
/// ever issued, so the operator rejects it with `Unauthenticated`.
const STALE_TOKEN: &str = "stale-operator-session-token-that-no-operator-issued";
/// Far enough out (year 2100) that the client-side validity check passes and
/// the stale token is actually sent to the operator instead of being re-minted.
const FAR_FUTURE_EXPIRATION: u64 = 4_102_444_800;

/// Serves a stale-but-unexpired token the first time each operator identity is
/// looked up, then delegates to an in-memory store. The stale token passes the
/// client-side validity check, so it reaches the operator, which rejects it
/// with `Unauthenticated` and drives the force-refresh self-heal that mints and
/// stores a fresh token.
struct StaleTokenSessionStore {
    inner: InMemorySessionStore,
    seeded: Mutex<HashSet<PublicKey>>,
    stale_served: AtomicUsize,
    fresh_stored: AtomicUsize,
}

impl StaleTokenSessionStore {
    fn new() -> Self {
        Self {
            inner: InMemorySessionStore::default(),
            seeded: Mutex::new(HashSet::new()),
            stale_served: AtomicUsize::new(0),
            fresh_stored: AtomicUsize::new(0),
        }
    }

    fn stale_served(&self) -> usize {
        self.stale_served.load(Ordering::SeqCst)
    }

    fn fresh_stored(&self) -> usize {
        self.fresh_stored.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl SessionStore for StaleTokenSessionStore {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionStoreError> {
        // First lookup for this operator: hand out the stale token. The lock is
        // released before any await, so it never spans a suspension point.
        let is_first_lookup = {
            let mut seeded = self.seeded.lock().expect("seeded mutex poisoned");
            seeded.insert(*service_identity_key)
        };
        if is_first_lookup {
            self.stale_served.fetch_add(1, Ordering::SeqCst);
            return Ok(Session {
                token: STALE_TOKEN.to_string(),
                expiration: FAR_FUTURE_EXPIRATION,
            });
        }
        self.inner.get_session(service_identity_key).await
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionStoreError> {
        self.fresh_stored.fetch_add(1, Ordering::SeqCst);
        self.inner.set_session(service_identity_key, session).await
    }
}

/// End-to-end coverage for the operator-side force-refresh self-heal: a cached
/// token that is unexpired but server-rejected must not fail the call. The
/// wallet connects with a session store seeded to serve such a token, then
/// issues an operator RPC. It succeeds only if the `Unauthenticated` rejection
/// triggered a force-refresh that re-minted and stored a valid token.
///
/// This is also the only coverage for `with_session_store` on the operator path
/// and confirms the operator returns `Unauthenticated` for a bad token, which
/// the self-heal depends on.
#[tokio::test]
#[test_log::test]
async fn test_operator_session_force_refresh_self_heal() -> Result<()> {
    let store = Arc::new(StaleTokenSessionStore::new());

    // Reaching Synced already exercises authenticated operator calls; the
    // explicit RPC below makes the "operator call succeeds" assertion direct.
    let (wallet, _listener) = create_regtest_wallet_with_session_store(store.clone()).await?;

    let deposit = wallet.generate_deposit_address().await?;
    info!("Generated deposit address: {}", deposit.address);
    assert!(
        !deposit.address.to_string().is_empty(),
        "operator RPC returned an empty deposit address"
    );

    assert!(
        store.stale_served() >= 1,
        "stale token was never served, so the self-heal path was not exercised"
    );
    assert!(
        store.fresh_stored() >= 1,
        "no fresh token was stored, so the force-refresh never ran"
    );

    Ok(())
}
