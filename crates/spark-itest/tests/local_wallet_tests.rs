use anyhow::Result;
use bitcoin::Amount;
use rstest::*;
use spark_itest::helpers::{WalletsFixture, deposit_to_wallet, wallets};
use tracing::info;

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

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_claim_unconfirmed_deposit(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_to_wallet(&wallet, bitcoind).await?;

    Ok(())
}

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
        .wait_for_log("tree not found in available or creating status")
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
