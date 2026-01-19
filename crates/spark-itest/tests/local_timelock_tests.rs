use anyhow::Result;
use rstest::*;
use spark_itest::helpers::{WalletsFixture, deposit_to_wallet, wait_for_event, wallets};
use spark_wallet::{SparkWallet, WalletEvent};
use tracing::info;

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_renew_timelocks(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;

    let mut alice = fixture.alice_wallet;
    let mut bob = fixture.bob_wallet;

    deposit_to_wallet(&alice, &fixture.fixtures.bitcoind).await?;

    // Get the total balance that will be sent back and forth
    let total_balance = alice.get_balance().await?;
    info!("Total balance to send back and forth: {total_balance} sats");

    let send_sdk_payment = async |from_wallet: &mut SparkWallet,
                                  to_wallet: &mut SparkWallet|
           -> Result<()> {
        info!("Sending via Spark started...");

        // Get current balances
        let sender_balance = from_wallet.get_balance().await?;
        let receiver_balance_before = to_wallet.get_balance().await?;

        // Verify we're sending the entire balance
        assert_eq!(
            sender_balance, total_balance,
            "Sender should have the entire balance"
        );
        assert_eq!(
            receiver_balance_before, 0,
            "Receiver should have zero balance before transfer"
        );

        info!(
            "Sender balance: {sender_balance}, Receiver balance before: {receiver_balance_before}"
        );

        // Get spark address of "to" SDK
        let spark_address = to_wallet.get_spark_address()?;

        // Subscribe to receiver's events BEFORE sending to avoid missing the event
        let mut listener = to_wallet.subscribe_events();

        info!("Sending {sender_balance} sats to {spark_address:?}...");

        // Send entire balance
        let _transfer = from_wallet
            .transfer(sender_balance, &spark_address, None)
            .await?;

        // Wait for TransferClaimed event on the receiver
        info!("Waiting for TransferClaimed event...");
        wait_for_event(&mut listener, 60, "TransferClaimed", |event| match &event {
            WalletEvent::TransferClaimed(_) => Ok(Some(event)),
            _ => Ok(None),
        })
        .await?;

        // Verify sender now has zero
        let sender_balance_after = from_wallet.get_balance().await?;
        assert_eq!(
            sender_balance_after, 0,
            "Sender should have zero balance after transfer"
        );

        info!("Sending via Spark completed - TransferClaimed received");
        Ok(())
    };

    for n in 0..200 {
        info!("Iteration {n}");
        info!("Sending from Alice to Bob via Spark...");
        send_sdk_payment(&mut alice, &mut bob).await?;
        info!("Sending from Bob to Alice via Spark...");
        send_sdk_payment(&mut bob, &mut alice).await?;
    }

    Ok(())
}
