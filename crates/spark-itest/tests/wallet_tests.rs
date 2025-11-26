use std::sync::Arc;

use anyhow::Result;
use bitcoin::Amount;
use rstest::*;
use spark_wallet::{SparkWallet, WalletEvent};
use tracing::{debug, info};

use spark_itest::{
    fixtures::{
        bitcoind::BitcoindFixture,
        setup::{TestFixtures, create_test_signer_alice, create_test_signer_bob},
    },
    helpers::wait_for_event,
};

// Setup test fixtures
#[fixture]
async fn fixtures() -> TestFixtures {
    TestFixtures::new()
        .await
        .expect("Failed to initialize test fixtures")
}

pub struct WalletsFixture {
    #[allow(dead_code)]
    fixtures: TestFixtures,
    alice_wallet: SparkWallet,
    bob_wallet: SparkWallet,
}

// Create a wallet for testing
#[fixture]
async fn wallets(#[future] fixtures: TestFixtures) -> WalletsFixture {
    let fixtures = fixtures.await;
    let config = fixtures
        .create_wallet_config()
        .await
        .expect("failed to create wallet config");
    let signer = create_test_signer_alice();

    let alice_wallet = SparkWallet::connect(config.clone(), Arc::new(signer))
        .await
        .expect("Failed to connect alice wallet");

    let bob_wallet = SparkWallet::connect(config, Arc::new(create_test_signer_bob()))
        .await
        .expect("Failed to connect bob wallet");

    let mut alice_listener = alice_wallet.subscribe_events();
    let mut bob_listener = bob_wallet.subscribe_events();
    loop {
        let event = alice_listener
            .recv()
            .await
            .expect("Failed to receive alicewallet event");
        info!("Alice wallet event: {:?}", event);
        if event == WalletEvent::Synced {
            break;
        }
    }
    loop {
        let event = bob_listener
            .recv()
            .await
            .expect("Failed to receive bob wallet event");
        info!("Bob wallet event: {:?}", event);
        if event == WalletEvent::Synced {
            break;
        }
    }

    WalletsFixture {
        fixtures,
        alice_wallet,
        bob_wallet,
    }
}
// Test creating a deposit address
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_create_deposit_address(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = fixture.alice_wallet;

    let address = wallet.generate_deposit_address(false).await?;
    info!("Generated deposit address: {}", address);

    assert!(
        !address.to_string().is_empty(),
        "Address should not be empty"
    );

    Ok(())
}

async fn deposit_wallet(wallet: &SparkWallet, bitcoind: &BitcoindFixture) -> Result<()> {
    // Generate a deposit address
    let deposit_address = wallet.generate_deposit_address(false).await?;
    info!("Generated deposit address: {}", deposit_address);

    // Fund the deposit address with a certain amount
    let deposit_amount = Amount::from_sat(100_000); // 0.001 BTC
    let txid = bitcoind
        .fund_address(&deposit_address, deposit_amount)
        .await?;
    info!(
        "Funded deposit address with {}, txid: {}",
        deposit_amount, txid
    );

    // Get the transaction to claim
    let tx = bitcoind.get_transaction(&txid).await?;
    info!("Got transaction: {:?}", tx);

    // Find the output index for our address
    let mut output_index = None;
    for (vout, output) in tx.output.iter().enumerate() {
        if let Ok(address) =
            bitcoin::Address::from_script(&output.script_pubkey, bitcoin::Network::Regtest)
            && address == deposit_address
        {
            output_index = Some(vout as u32);
            break;
        }
    }

    let vout = output_index.expect("Could not find deposit address in transaction outputs");
    info!("Found deposit output at index: {}", vout);

    let mut listener = wallet.subscribe_events();
    let leaves = wallet.claim_deposit(tx, vout).await?;
    info!("Claimed deposit, got leaves: {:?}", leaves);

    // Mine a block to confirm the transaction
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&txid, 1).await?;
    info!("Transaction confirmed");

    // Wait for the deposit confirmation event from the SO.
    loop {
        let event: WalletEvent = listener
            .recv()
            .await
            .expect("Failed to receive wallet event");
        match event {
            WalletEvent::DepositConfirmed(_) => {
                info!("Received deposit confirmed event");
                break;
            }
            _ => debug!("Received other event: {:?}", event),
        }
    }

    // Check that balance increased by the deposit amount
    let final_balance = wallet.get_balance().await?;
    info!("Final balance: {}", final_balance);

    // The deposited amount should now be reflected in the balance
    assert_eq!(
        final_balance,
        deposit_amount.to_sat(),
        "Balance should be the deposit amount"
    );
    Ok(())
}

// Test claiming a deposit
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_claim_unconfirmed_deposit(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_wallet(&wallet, bitcoind).await?;

    Ok(())
}

// Test claiming an already confirmed deposit
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_claim_confirmed_deposit(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    // Generate a deposit address
    let deposit_address = wallet.generate_deposit_address(false).await?;
    info!("Generated deposit address: {}", deposit_address);

    // Fund the deposit address with a certain amount
    let deposit_amount = Amount::from_sat(100_000);
    let txid = bitcoind
        .fund_address(&deposit_address, deposit_amount)
        .await?;
    info!(
        "Funded deposit address with {}, txid: {}",
        deposit_amount, txid
    );

    // Mine blocks to confirm the transaction before claiming
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&txid, 1).await?;
    info!("Transaction confirmed");

    // Wait for the Service Operator to log that the deposit is confirmed
    info!("Waiting for SO to log confirmation message");
    fixture
        .fixtures
        .spark_so
        .wait_for_log("Deposit confirmed before tree creation or tree already available")
        .await?;
    info!("SO confirmed deposit is ready to claim");

    // Get the transaction to claim
    let tx = bitcoind.get_transaction(&txid).await?;
    info!("Got transaction: {:?}", tx);

    // Find the output index for our address
    let mut output_index = None;
    for (vout, output) in tx.output.iter().enumerate() {
        if let Ok(address) =
            bitcoin::Address::from_script(&output.script_pubkey, bitcoin::Network::Regtest)
            && address == deposit_address
        {
            output_index = Some(vout as u32);
            break;
        }
    }

    let vout = output_index.expect("Could not find deposit address in transaction outputs");
    info!("Found deposit output at index: {}", vout);

    let leaves = wallet.claim_deposit(tx, vout).await?;
    info!("Claimed deposit, got leaves: {:?}", leaves);

    // Check that balance increased immediately after claiming
    let balance = wallet.get_balance().await?;
    assert_eq!(balance, 100_000, "Balance should be the deposit amount");
    Ok(())
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_renew_timelocks(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;

    let mut alice = fixture.alice_wallet;
    let mut bob = fixture.bob_wallet;

    deposit_wallet(&alice, &fixture.fixtures.bitcoind).await?;

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
