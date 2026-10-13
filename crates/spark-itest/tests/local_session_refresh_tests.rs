use std::sync::Arc;

use anyhow::Result;
use rand::Rng;
use rstest::*;
use spark_itest::fixtures::setup::TestFixtures;
use spark_itest::helpers::test_fixtures;
use spark_itest::session_refresh::{ControlledSessionStore, RefreshWallet, build_refresh_wallet};

const FAR_FUTURE_EXPIRATION: u64 = 4_102_444_800;
const BAD_REFRESH_TOKEN: &str = "not-a-refresh-token";
const BAD_SESSION_TOKEN: &str = "session-token-no-operator-issued";

fn random_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill(&mut seed);
    seed
}

async fn authenticated_wallet(
    fixtures: &TestFixtures,
) -> Result<(RefreshWallet, Arc<ControlledSessionStore>)> {
    let config = fixtures.create_wallet_config().await?;
    let store = Arc::new(ControlledSessionStore::default());
    let wallet = build_refresh_wallet(config, random_seed(), store.clone()).await?;
    wallet.wallet.generate_deposit_address().await?;
    let sessions = store.snapshot();
    assert!(!sessions.is_empty(), "no operator session was stored");
    assert!(
        sessions.values().all(|s| s.refresh_token.is_some()),
        "operator issued no refresh token, is spark.so.authn.refresh_tokens_enabled set?"
    );
    Ok((wallet, store))
}

fn expire_sessions(store: &ControlledSessionStore) {
    store.update_all(|s| s.expiration = 0);
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_expired_session_refreshes_without_signing(
    #[future] test_fixtures: TestFixtures,
) -> Result<()> {
    let fixtures = test_fixtures.await;
    let (refresh_wallet, store) = authenticated_wallet(&fixtures).await?;
    let before = store.snapshot();
    expire_sessions(&store);
    let signatures = refresh_wallet.signer.auth_signatures();

    refresh_wallet.wallet.generate_deposit_address().await?;

    assert_eq!(refresh_wallet.signer.auth_signatures(), signatures);
    let after = store.snapshot();
    let mut touched = 0;
    for (key, session) in &after {
        if session.token != before[key].token {
            touched += 1;
            assert_ne!(
                session.refresh_token, before[key].refresh_token,
                "operator {key} rotated its session token without rotating its refresh token"
            );
        }
    }
    assert!(touched >= 1, "no operator session token was refreshed");
    Ok(())
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_replayed_refresh_token_returns_the_same_successor(
    #[future] test_fixtures: TestFixtures,
) -> Result<()> {
    let fixtures = test_fixtures.await;
    let (refresh_wallet, store) = authenticated_wallet(&fixtures).await?;
    expire_sessions(&store);
    let consumed = store.snapshot();
    refresh_wallet.wallet.generate_deposit_address().await?;
    let successors = store.snapshot();
    let rotated_keys: Vec<_> = successors
        .iter()
        .filter(|(key, session)| session.refresh_token != consumed[*key].refresh_token)
        .map(|(key, _)| *key)
        .collect();
    assert!(!rotated_keys.is_empty(), "no refresh token was rotated");
    store.restore(consumed.clone());
    let signatures = refresh_wallet.signer.auth_signatures();

    refresh_wallet.wallet.generate_deposit_address().await?;

    assert_eq!(refresh_wallet.signer.auth_signatures(), signatures);
    let replayed = store.snapshot();
    for key in rotated_keys {
        assert_eq!(
            replayed[&key].refresh_token, successors[&key].refresh_token,
            "operator {key} returned a different successor on replay"
        );
    }
    Ok(())
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_rejected_refresh_token_falls_back_to_signing(
    #[future] test_fixtures: TestFixtures,
) -> Result<()> {
    let fixtures = test_fixtures.await;
    let (refresh_wallet, store) = authenticated_wallet(&fixtures).await?;
    store.update_all(|s| {
        s.expiration = 0;
        s.refresh_token = Some(BAD_REFRESH_TOKEN.to_string());
    });
    let signatures = refresh_wallet.signer.auth_signatures();

    refresh_wallet.wallet.generate_deposit_address().await?;

    assert!(refresh_wallet.signer.auth_signatures() > signatures);
    assert!(
        store.snapshot().values().any(|s| s
            .refresh_token
            .as_deref()
            .is_some_and(|t| t != BAD_REFRESH_TOKEN)),
        "the signed handshake stored no new refresh token"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_rejected_session_token_refreshes_without_signing(
    #[future] test_fixtures: TestFixtures,
) -> Result<()> {
    let fixtures = test_fixtures.await;
    let (refresh_wallet, store) = authenticated_wallet(&fixtures).await?;
    let before = store.snapshot();
    store.update_all(|s| {
        s.token = BAD_SESSION_TOKEN.to_string();
        s.expiration = FAR_FUTURE_EXPIRATION;
    });
    let signatures = refresh_wallet.signer.auth_signatures();

    refresh_wallet.wallet.generate_deposit_address().await?;

    assert_eq!(refresh_wallet.signer.auth_signatures(), signatures);
    let after = store.snapshot();
    let mut touched = 0;
    for (key, session) in &after {
        if session.token != BAD_SESSION_TOKEN {
            touched += 1;
            assert_ne!(
                session.refresh_token, before[key].refresh_token,
                "operator {key} replaced a rejected session token without rotating its refresh token"
            );
        }
    }
    assert!(touched >= 1, "no rejected session token was replaced");
    Ok(())
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_concurrent_refresh_from_a_shared_store_does_not_sign(
    #[future] test_fixtures: TestFixtures,
) -> Result<()> {
    let fixtures = test_fixtures.await;
    let config = fixtures.create_wallet_config().await?;
    let store = Arc::new(ControlledSessionStore::default());
    let seed = random_seed();
    let first = build_refresh_wallet(config.clone(), seed, store.clone()).await?;
    let second = build_refresh_wallet(config, seed, store.clone()).await?;
    first.wallet.generate_deposit_address().await?;
    expire_sessions(&store);
    let first_signatures = first.signer.auth_signatures();
    let second_signatures = second.signer.auth_signatures();

    let (first_result, second_result) = tokio::join!(
        first.wallet.generate_deposit_address(),
        second.wallet.generate_deposit_address()
    );
    first_result?;
    second_result?;

    assert_eq!(first.signer.auth_signatures(), first_signatures);
    assert_eq!(second.signer.auth_signatures(), second_signatures);
    Ok(())
}
