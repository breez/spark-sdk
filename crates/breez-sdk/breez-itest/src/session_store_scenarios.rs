//! Backend-agnostic scenarios for the persistent `SessionStore`.
//!
//! Verifies that two SDK pods backed by the same database — wired up via
//! a shared [`SdkContext`](breez_sdk_spark::SdkContext) constructed with a
//! postgres / mysql config — share authentication state across restarts: a
//! fresh instance reuses the cached SSP/SO sessions instead of re-running
//! the challenge-response handshake.
//!
//! Each scenario takes a `build_instance` closure that produces a fresh
//! `SdkInstance` bound to the shared backend, and a pair of helpers that
//! read and clear the `sessions` table for the shared tenant identity.
//! Backend-specific tests provide them via their fixture and retain the
//! testcontainer for the scenario's lifetime.

use std::collections::HashSet;
use std::future::Future;

use anyhow::Result;
use tracing::info;

use crate::SdkInstance;

/// One row of the persistent `sessions` table, normalised for cross-backend
/// comparison.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionRow {
    /// 33-byte service identity public key (SSP or operator).
    pub service_identity_key: Vec<u8>,
    /// Bearer token issued by the service after challenge-response.
    pub token: String,
    /// Unix-seconds expiration timestamp.
    pub expiration: u64,
}

/// Verifies session persistence and reuse across SDK restarts:
///
/// 1. Build instance A, drive it through one sync so the SSP/SO clients
///    populate the `sessions` table via challenge-response.
/// 2. Drop A and build a fresh instance B against the same database with
///    the same tenant identity. Confirm the cached rows are unchanged —
///    proving B reused them rather than re-authenticating.
/// 3. Wipe the sessions table, build instance C, and confirm the table is
///    repopulated with *new* tokens — proving the auth path still works
///    when the cache is empty.
///
/// Each instance is `disconnect`-ed before drop so the periodic sync loop
/// finishes any in-flight tick before the next phase observes (or wipes)
/// the database. Without the explicit disconnect, the loop would survive
/// past `drop` until its current `await` resolves and could race with
/// `clear_sessions` in Phase 3 by re-inserting a row after the DELETE.
pub async fn run_session_persistence_across_restart<F, Fut, R, RFut, C, CFut>(
    build_instance: F,
    read_sessions: R,
    clear_sessions: C,
) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<SdkInstance>>,
    R: Fn() -> RFut,
    RFut: Future<Output = Result<Vec<SessionRow>>>,
    C: Fn() -> CFut,
    CFut: Future<Output = Result<()>>,
{
    info!("=== Phase 1: build instance A and populate the sessions table ===");
    let instance_a = build_instance().await?;
    let sessions_a = read_sessions().await?;
    assert!(
        !sessions_a.is_empty(),
        "instance A should have populated the sessions table after sync; got 0 rows"
    );
    info!(
        "instance A populated {} session row(s) for tenant",
        sessions_a.len()
    );
    instance_a.sdk.disconnect().await?;
    drop(instance_a);

    info!("=== Phase 2: build instance B and confirm sessions are reused ===");
    let instance_b = build_instance().await?;
    let sessions_b = read_sessions().await?;

    let set_a: HashSet<_> = sessions_a.iter().collect();
    let set_b: HashSet<_> = sessions_b.iter().collect();
    assert_eq!(
        set_a, set_b,
        "instance B should have reused the cached sessions verbatim — \
         a divergence means B re-authenticated despite a valid cached session.\n\
         A: {sessions_a:?}\n\
         B: {sessions_b:?}"
    );
    info!(
        "instance B reused all {} cached session row(s)",
        sessions_b.len()
    );
    instance_b.sdk.disconnect().await?;
    drop(instance_b);

    info!("=== Phase 3: clear sessions, build instance C, expect fresh auth ===");
    clear_sessions().await?;
    let cleared = read_sessions().await?;
    assert!(
        cleared.is_empty(),
        "expected sessions table empty after clear; got {} row(s)",
        cleared.len()
    );

    let instance_c = build_instance().await?;
    let sessions_c = read_sessions().await?;
    assert!(
        !sessions_c.is_empty(),
        "instance C should have re-populated the sessions table after the \
         cache was cleared"
    );

    let tokens_a: HashSet<_> = sessions_a.iter().map(|s| &s.token).collect();
    let tokens_c: HashSet<_> = sessions_c.iter().map(|s| &s.token).collect();
    assert!(
        tokens_a.is_disjoint(&tokens_c),
        "instance C should have produced fresh tokens after re-auth, but \
         shares at least one token with A. A: {sessions_a:?}\nC: {sessions_c:?}"
    );
    info!(
        "instance C re-authenticated and wrote {} new session row(s)",
        sessions_c.len()
    );
    instance_c.sdk.disconnect().await?;
    drop(instance_c);

    Ok(())
}
