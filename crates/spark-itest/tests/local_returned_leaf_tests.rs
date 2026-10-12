use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use bitcoin::hashes::{Hash as _, sha256};
use rstest::*;
use spark_itest::helpers::{
    WalletsFixture, deposit_to_wallet, refund_timelock, wait_for_event, wallets,
};
use spark_wallet::{SparkWallet, WalletEvent};
use tracing::info;

/// The refund timelock at or below which the operators renew a returned leaf
/// through the arms that reset its refund, rather than decrementing it.
const OPERATOR_RENEW_TIMELOCK_THRESHOLD: u32 = 300;

/// The refund timelock a reset renews a leaf to.
const INITIAL_REFUND_TIMELOCK: u32 = 2_000;

/// Locks `from`'s whole balance in an HTLC to `to` that nobody claims, and waits
/// for it to expire and the balance to come back.
async fn send_htlc_and_let_it_expire(from: &SparkWallet, to: &SparkWallet) -> Result<()> {
    let balance = from.get_balance().await?;
    let payment_hash = sha256::Hash::hash(&rand::random::<[u8; 32]>());
    from.create_htlc(
        balance,
        &to.get_spark_address()?,
        &payment_hash,
        Duration::from_secs(30),
        None,
    )
    .await?;
    ensure!(
        from.get_balance().await? == 0,
        "the HTLC holds the whole balance"
    );
    info!("Locked {balance} sats in an HTLC nobody claims");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        from.sync().await?;
        if from.get_balance().await? == balance {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("the balance did not come back after the HTLC expired");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Sends `from`'s whole balance to `to` and waits for `to` to claim it.
async fn send_all(from: &SparkWallet, to: &SparkWallet) -> Result<()> {
    let balance = from.get_balance().await?;
    let mut events = to.subscribe_events();
    from.transfer(balance, &to.get_spark_address()?, None)
        .await?;
    wait_for_event(&mut events, 60, "TransferClaimed", |event| {
        Ok(matches!(event, WalletEvent::TransferClaimed(_)).then_some(event))
    })
    .await?;
    ensure!(
        to.get_balance().await? == balance,
        "the receiver holds the whole balance"
    );
    Ok(())
}

/// An HTLC nobody claims expires and is returned. The operators park its leaves
/// until the sender renews their refunds below the refund the HTLC left signed,
/// after which they are spendable again.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn an_expired_htlc_returns_its_leaves_renewed(
    #[future] wallets: WalletsFixture,
) -> Result<()> {
    let fixture = wallets.await;
    let alice = fixture.alice_wallet;
    let bob = fixture.bob_wallet;

    deposit_to_wallet(&alice, &fixture.fixtures.bitcoind).await?;
    let sent = alice.list_leaves().await?.available;

    send_htlc_and_let_it_expire(&alice, &bob).await?;

    let returned = alice.list_leaves().await?.available;
    for leaf in &sent {
        let back = returned
            .iter()
            .find(|l| l.id == leaf.id)
            .context("a leaf the HTLC sent is back")?;
        assert_eq!(refund_timelock(back), refund_timelock(leaf) - 100);
    }

    send_all(&alice, &bob).await
}

/// A returned leaf whose refund timelock is at the operators' threshold cannot
/// be decremented, so it renews through an arm that resets its refund.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn a_returned_leaf_at_the_threshold_renews_through_a_reset(
    #[future] wallets: WalletsFixture,
) -> Result<()> {
    let fixture = wallets.await;
    let alice = fixture.alice_wallet;
    let bob = fixture.bob_wallet;

    deposit_to_wallet(&alice, &fixture.fixtures.bitcoind).await?;
    let balance = alice.get_balance().await?;

    // Each transfer lowers the leaf's refund timelock by one interval.
    let (mut holder, mut other) = (&alice, &bob);
    loop {
        let leaves = holder.list_leaves().await?.available;
        ensure!(leaves.len() == 1, "the balance stays in a single leaf");
        let timelock = refund_timelock(&leaves[0]);
        if timelock <= OPERATOR_RENEW_TIMELOCK_THRESHOLD {
            assert_eq!(timelock, OPERATOR_RENEW_TIMELOCK_THRESHOLD);
            break;
        }
        info!("Leaf refund timelock {timelock}, passing it on");
        send_all(holder, other).await?;
        (holder, other) = (other, holder);
    }

    send_htlc_and_let_it_expire(holder, other).await?;

    let returned = holder.list_leaves().await?.available;
    ensure!(!returned.is_empty(), "the leaf is back");
    for leaf in &returned {
        assert_eq!(refund_timelock(leaf), INITIAL_REFUND_TIMELOCK);
    }
    assert_eq!(holder.get_balance().await?, balance);

    send_all(holder, other).await
}
