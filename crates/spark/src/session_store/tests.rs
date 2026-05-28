//! Shared test suite for [`SessionStore`] implementations.
//!
//! Each function tests a specific behavior against any `SessionStore` impl.
//! To use, call these functions from implementation-specific test modules
//! passing a concrete store instance.

use bitcoin::secp256k1::PublicKey;

use crate::session_store::{Session, SessionStore, SessionStoreError};

/// Builds a 33-byte compressed public key with `fill_byte` everywhere except
/// the version prefix. Tests use distinct fill bytes to scope sessions.
pub fn create_public_key(fill_byte: u8) -> PublicKey {
    let mut pk_bytes = [fill_byte; 33];
    pk_bytes[0] = 2;
    PublicKey::from_slice(&pk_bytes).unwrap()
}

fn session(token: &str, expiration: u64) -> Session {
    Session {
        token: token.to_string(),
        expiration,
    }
}

pub async fn test_get_session_not_found(store: &dyn SessionStore) {
    let pk = create_public_key(1);
    assert!(matches!(
        store.get_session(&pk).await,
        Err(SessionStoreError::NotFound)
    ));
}

pub async fn test_set_and_get(store: &dyn SessionStore) {
    let pk = create_public_key(1);
    store
        .set_session(&pk, session("token-A", 1_000_000_000))
        .await
        .expect("set_session");

    let stored = store.get_session(&pk).await.expect("get_session");
    assert_eq!(stored.token, "token-A");
    assert_eq!(stored.expiration, 1_000_000_000);
}

pub async fn test_overwrite_session(store: &dyn SessionStore) {
    let pk = create_public_key(1);
    store
        .set_session(&pk, session("first", 1_000_000_000))
        .await
        .expect("set first");
    store
        .set_session(&pk, session("second", 2_000_000_000))
        .await
        .expect("set second");

    let stored = store.get_session(&pk).await.expect("get_session");
    assert_eq!(stored.token, "second");
    assert_eq!(stored.expiration, 2_000_000_000);
}

pub async fn test_sessions_are_isolated_by_key(store: &dyn SessionStore) {
    let pk1 = create_public_key(1);
    let pk2 = create_public_key(2);

    store
        .set_session(&pk1, session("token-1", 1_000_000_000))
        .await
        .expect("set pk1");
    store
        .set_session(&pk2, session("token-2", 2_000_000_000))
        .await
        .expect("set pk2");

    let stored1 = store.get_session(&pk1).await.expect("get pk1");
    let stored2 = store.get_session(&pk2).await.expect("get pk2");

    assert_eq!(stored1.token, "token-1");
    assert_eq!(stored1.expiration, 1_000_000_000);
    assert_eq!(stored2.token, "token-2");
    assert_eq!(stored2.expiration, 2_000_000_000);
}

pub async fn test_get_after_unrelated_set(store: &dyn SessionStore) {
    let pk1 = create_public_key(1);
    let pk2 = create_public_key(2);

    store
        .set_session(&pk1, session("only", 1_000_000_000))
        .await
        .expect("set pk1");

    assert!(matches!(
        store.get_session(&pk2).await,
        Err(SessionStoreError::NotFound)
    ));
}
