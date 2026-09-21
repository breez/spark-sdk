use std::time::Duration;

use anyhow::{Result, bail};
use spark::signer::{DefaultSigner, derive_identity_public_key};
use spark_itest::fixtures::bitcoind::BitcoindFixture;
use spark_itest::fixtures::database::{DatabaseFixture, SSPD_DATABASE, operator_database};
use spark_itest::fixtures::setup::{FixtureId, SSPD_WALLET_SEED_HEX};
use spark_itest::fixtures::spark_so::{SparkSoFixture, StateSource};
use spark_itest::fixtures::sspd::{FULL_POOL_ONCHAIN_SATS, LEAVES_PER_DENOMINATION, SspdFixture};
use spark_itest::fixtures::{keyshares, state_snapshot};
use tracing::info;

#[tokio::test]
#[test_log::test]
#[ignore = "state snapshot maintenance; run via the itest xtask targets"]
async fn ensure_state_snapshot() -> Result<()> {
    if let Err(e) = state_snapshot::check() {
        info!("Building a state snapshot: {e}");
        capture().await?;
        state_snapshot::database_image().await?;
        return Ok(());
    }

    // A change to what the daemon stores can leave the manifest unchanged, so the
    // restored pool is also read back, and a stale one is rebuilt.
    let signer = DefaultSigner::new(&hex::decode(SSPD_WALLET_SEED_HEX)?, spark::Network::Regtest)?;
    let identity = derive_identity_public_key(&signer).await?;
    if let Err(e) = state_snapshot::verify(&identity.serialize()).await {
        info!("Rebuilding the state snapshot: {e}");
        capture().await?;
        state_snapshot::database_image().await?;
        return Ok(());
    }

    info!("State snapshot is current.");
    Ok(())
}

#[tokio::test]
#[test_log::test]
#[ignore = "builds the state snapshot; run via `make capture-itest-state`"]
async fn capture_state_snapshot() -> Result<()> {
    capture().await
}

async fn capture() -> Result<()> {
    // Discarded first: the daemon's fixture restores a current snapshot, and this
    // cluster's fresh bitcoind lacks that snapshot's chain.
    state_snapshot::discard()?;

    let fixture_id = FixtureId::new();

    let (mut bitcoind, database) = tokio::try_join!(
        BitcoindFixture::new(&fixture_id),
        DatabaseFixture::start(&fixture_id, false),
    )?;
    bitcoind.initialize().await?;

    info!(
        "Generating {} keyshares per coordinator...",
        keyshares::CAPTURE_KEYS_PER_COORDINATOR
    );
    let mut spark_so = SparkSoFixture::new_with_keyshares(
        &fixture_id,
        &bitcoind,
        &database,
        StateSource::Dkg {
            target: keyshares::CAPTURE_KEYS_PER_COORDINATOR,
        },
    )
    .await?;
    spark_so.initialize().await?;

    info!("Stocking the SSP's leaf pool...");
    let sspd = SspdFixture::start(
        &fixture_id,
        &bitcoind,
        &spark_so.operators,
        &database,
        SSPD_WALLET_SEED_HEX,
        None,
    )
    .await?;
    // One pool's worth builds the pool; tests spend the rest refilling it and
    // fronting coop exits.
    sspd.fund_onchain(
        &bitcoind,
        1_000_000,
        (FULL_POOL_ONCHAIN_SATS / 1_000_000) as usize * 4,
    )
    .await?;
    sspd.wait_for_pool(&bitcoind, LEAVES_PER_DENOMINATION, Duration::from_secs(900))
        .await?;

    let leaves = sspd.pool_leaf_counts().await?;
    let total: u32 = leaves.values().sum();
    if leaves.is_empty() {
        bail!("captured a snapshot with an empty pool");
    }
    info!(
        "Pool holds {total} leaves across {} denominations",
        leaves.len()
    );

    // Nothing may change between the dumps. The daemon stops first, so that it
    // takes no more keyshares. The operators stop once DKG has refilled what the
    // pool took: DKG commits a batch to each operator's database separately, so
    // stopping one mid-batch would leave the batch on some databases only.
    sspd.container.stop().await?;
    spark_so
        .wait_for_keyshares(keyshares::CAPTURE_KEYS_PER_COORDINATOR)
        .await?;
    spark_so.stop_operators().await?;

    let network = fixture_id.to_network();
    for operator in &spark_so.operators {
        state_snapshot::capture_database(
            &network,
            &database.host_name,
            &operator_database(operator.index),
            &format!("operator-{}", operator.index),
        )
        .await?;
    }
    state_snapshot::capture_database(&network, &database.host_name, SSPD_DATABASE, "sspd").await?;

    bitcoind
        .stop_and_archive(&state_snapshot::bitcoind_datadir())
        .await?;

    state_snapshot::write_manifest()?;
    state_snapshot::write_digest()?;
    info!(
        "State snapshot written to {}",
        state_snapshot::snapshot_dir().display()
    );
    Ok(())
}
