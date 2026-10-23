//! The wallet announces a deposit once every operator holds its leaf as
//! available.
//!
//! Each operator marks the leaf available when its own chain watcher processes
//! the block that contains the deposit transaction, so the coordinator can hold
//! the leaf as available before the other operators do. Until they follow, they
//! reject a transfer of that leaf.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use spark_itest::fixtures::setup::{
    TestFixtures, create_test_signer_alice, create_test_signer_bob,
};
use spark_wallet::WalletEvent;
use tracing::info;

use spark_itest::helpers::{build_test_wallet, deposit_with_amount};

const DEPOSIT_SATS: u64 = 10_000;

/// A caller waits for the announcement before spending a deposit, so the wallet
/// has to be able to spend the leaf by the time it announces the deposit. A
/// transfer needs every operator, and an operator that still holds the leaf as
/// creating rejects it.
#[test_log::test(tokio::test)]
async fn test_the_wallet_can_spend_an_announced_deposit_at_once() -> Result<()> {
    let fixtures = TestFixtures::new().await?;
    let backend = spark_itest::backend::resolve_backend().await?;

    let alice_config = fixtures.create_wallet_config().await?;
    let alice =
        build_test_wallet(alice_config, Arc::new(create_test_signer_alice()), &backend).await?;
    alice.start_background_processing().await;

    let bob_config = fixtures.create_wallet_config().await?;
    let bob = build_test_wallet(bob_config, Arc::new(create_test_signer_bob()), &backend).await?;
    bob.start_background_processing().await;
    let bob_address = bob.get_spark_address()?;

    // Returns on the announcement, which is the state under test.
    deposit_with_amount(&alice, &fixtures.bitcoind, DEPOSIT_SATS).await?;

    let transfer = alice
        .transfer(DEPOSIT_SATS, &bob_address, None)
        .await
        .context("an announced deposit must be spendable without waiting any longer")?;
    info!("spent the announced deposit in transfer {}", transfer.id);
    Ok(())
}

/// The wallet announces a deposit once: when it finds the same leaf in a later
/// refresh, it does not announce the deposit again, so a caller counting
/// deposits hears about each one once.
#[test_log::test(tokio::test)]
async fn test_the_wallet_announces_a_deposit_once() -> Result<()> {
    let fixtures = TestFixtures::new().await?;
    let backend = spark_itest::backend::resolve_backend().await?;
    let config = fixtures.create_wallet_config().await?;
    let wallet = build_test_wallet(config, Arc::new(create_test_signer_alice()), &backend).await?;
    wallet.start_background_processing().await;

    deposit_with_amount(&wallet, &fixtures.bitcoind, DEPOSIT_SATS).await?;

    let mut events = wallet.subscribe_events();
    wallet.sync().await?;
    wallet.sync().await?;

    tokio::time::sleep(Duration::from_secs(1)).await;
    // Read past the events the wallet raises in every refresh: the first one
    // queued is not the deposit's.
    loop {
        match events.try_recv() {
            Ok(WalletEvent::DepositConfirmed(id)) => {
                anyhow::bail!("deposit {id} was announced a second time")
            }
            Ok(other) => info!("ignoring {other:?}"),
            Err(_) => return Ok(()),
        }
    }
}
