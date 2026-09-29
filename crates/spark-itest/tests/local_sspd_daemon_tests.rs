use std::time::Duration;

use anyhow::Result;
use spark::tree::TreeStore;
use spark_itest::fixtures::setup::TestFixtures;
use spark_itest::fixtures::sspd::{FULL_POOL_ONCHAIN_SATS, SspdFixture};
use tracing::info;

use spark_itest::fixtures::setup::SSPD_WALLET_SEED_HEX as WALLET_SEED;

#[tokio::test]
#[test_log::test]
async fn test_sspd_daemon_starts_against_the_local_cluster() -> Result<()> {
    let fixtures = TestFixtures::new().await?;
    let sspd = SspdFixture::start(
        &fixtures.fixture_id,
        &fixtures.bitcoind,
        &fixtures.spark_so.operators,
        WALLET_SEED,
        None,
    )
    .await?;

    info!("sspd serving graphql at {}", sspd.base_url);
    assert!(sspd.base_url.starts_with("http://127.0.0.1:"));
    assert!(sspd.internal_url.starts_with("http://127.0.0.1:"));
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn test_sspd_stocks_its_own_leaf_pool() -> Result<()> {
    let fixtures = TestFixtures::new().await?;
    let sspd = SspdFixture::start(
        &fixtures.fixture_id,
        &fixtures.bitcoind,
        &fixtures.spark_so.operators,
        WALLET_SEED,
        None,
    )
    .await?;

    let utxos = (FULL_POOL_ONCHAIN_SATS / 1_000_000) as usize;
    sspd.fund_onchain(&fixtures.bitcoind, 1_000_000, utxos)
        .await?;
    sspd.wait_for_onchain_balance(
        &fixtures.bitcoind,
        FULL_POOL_ONCHAIN_SATS,
        Duration::from_secs(60),
    )
    .await?;
    sspd.wait_for_pool(&fixtures.bitcoind, 1, Duration::from_secs(600))
        .await?;

    let counts = sspd.pool_leaf_counts().await?;
    assert!(counts.get(&1).copied().unwrap_or(0) >= 1);
    assert!(counts.get(&65536).copied().unwrap_or(0) >= 1);
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn test_stocked_leaves_carry_their_exit_chains() -> Result<()> {
    let fixtures = TestFixtures::new().await?;
    let sspd = SspdFixture::start(
        &fixtures.fixture_id,
        &fixtures.bitcoind,
        &fixtures.spark_so.operators,
        WALLET_SEED,
        None,
    )
    .await?;

    let utxos = (FULL_POOL_ONCHAIN_SATS / 1_000_000) as usize;
    sspd.fund_onchain(&fixtures.bitcoind, 1_000_000, utxos)
        .await?;
    sspd.wait_for_onchain_balance(
        &fixtures.bitcoind,
        FULL_POOL_ONCHAIN_SATS,
        Duration::from_secs(60),
    )
    .await?;
    sspd.wait_for_pool(&fixtures.bitcoind, 1, Duration::from_secs(600))
        .await?;

    let store = sspd.tree_store().await?;
    let missing = store.leaves_missing_exit_chains().await?;
    assert!(
        missing.is_empty(),
        "{} stocked leaves have no chain to exit along",
        missing.len()
    );
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn test_claimed_leaves_gain_their_exit_chains() -> Result<()> {
    let fixtures = TestFixtures::new().await?;
    let sspd = SspdFixture::start(
        &fixtures.fixture_id,
        &fixtures.bitcoind,
        &fixtures.spark_so.operators,
        WALLET_SEED,
        None,
    )
    .await?;
    sspd.wait_for_pool(&fixtures.bitcoind, 1, Duration::from_secs(600))
        .await?;

    let store = sspd.tree_store().await?;
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    loop {
        let missing = store.leaves_missing_exit_chains().await?;
        if missing.is_empty() {
            info!("every leaf the daemon holds carries its exit chain");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("{} leaves still have no exit chain", missing.len());
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
