//! Regenerates the committed keyshare seed that every local cluster loads.
//!
//! Run it with `make capture-itest-keyshares` and commit the result. This is
//! the only place a cluster still waits on DKG, and it runs alone, so the
//! contention that makes DKG unreliable under a full test suite is absent.

use anyhow::{Result, bail};
use spark_itest::fixtures::bitcoind::BitcoindFixture;
use spark_itest::fixtures::keyshares;
use spark_itest::fixtures::setup::FixtureId;
use spark_itest::fixtures::spark_so::{KeyshareSource, NUM_OPERATORS, SparkSoFixture};
use tracing::info;

#[tokio::test]
#[test_log::test]
#[ignore = "regenerates the committed keyshare seed; run via `make capture-itest-keyshares`"]
async fn capture_keyshare_seed() -> Result<()> {
    let fixture_id = FixtureId::new();

    let mut bitcoind = BitcoindFixture::new(&fixture_id).await?;
    bitcoind.initialize().await?;

    info!("Starting a cluster that generates its own keyshares...");
    let mut spark_so =
        SparkSoFixture::new_with_keyshares(&fixture_id, &bitcoind, KeyshareSource::Dkg).await?;
    spark_so.initialize().await?;

    let captured = spark_so.capture_keyshare_seed().await?;
    let expected = NUM_OPERATORS as u64 * (keyshares::MIN_AVAILABLE_KEYS as u64 + 1);
    if captured < expected {
        bail!("captured only {captured} keyshares across {NUM_OPERATORS} operators");
    }

    info!("Captured {captured} keyshares. Commit crates/spark-itest/keyshares/.");
    Ok(())
}
