use anyhow::Result;
use bitcoin::Amount;
use rstest::*;
use spark_wallet::{DefaultSigner, SparkWallet, WalletEvent};
use tracing::{debug, info};

use spark_itest::fixtures::setup::{TestFixtures, create_test_signer};

// Setup test fixtures
#[fixture]
async fn fixtures() -> TestFixtures {
    TestFixtures::new()
        .await
        .expect("Failed to initialize test fixtures")
}

pub struct WalletFixture {
    #[allow(dead_code)]
    fixtures: TestFixtures,
    wallet: SparkWallet<DefaultSigner>,
}

// Create a wallet for testing
#[fixture]
async fn wallet(#[future] fixtures: TestFixtures) -> WalletFixture {
    let fixtures = fixtures.await;
    let config = fixtures
        .create_wallet_config()
        .await
        .expect("failed to create wallet config");
    let signer = create_test_signer();

    let wallet = SparkWallet::connect(config, signer)
        .await
        .expect("Failed to connect wallet");

    let mut listener = wallet.subscribe_events();
    loop {
        let event = listener
            .recv()
            .await
            .expect("Failed to receive wallet event");
        info!("Wallet event: {:?}", event);
        if event == WalletEvent::Synced {
            break;
        }
    }
    WalletFixture { fixtures, wallet }
}
// Test creating a deposit address
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_create_deposit_address(#[future] wallet: WalletFixture) -> Result<()> {
    let fixture = wallet.await;
    let wallet = fixture.wallet;

    let address = wallet.generate_deposit_address(false).await?;
    info!("Generated deposit address: {}", address);

    assert!(
        !address.to_string().is_empty(),
        "Address should not be empty"
    );

    Ok(())
}

// Test claiming a deposit
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_claim_unconfirmed_deposit(#[future] wallet: WalletFixture) -> Result<()> {
    let fixture = wallet.await;
    let wallet = fixture.wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

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

// Test claiming an already confirmed deposit
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_claim_confirmed_deposit(#[future] wallet: WalletFixture) -> Result<()> {
    let fixture = wallet.await;
    let wallet = fixture.wallet;
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
