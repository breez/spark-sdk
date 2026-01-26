use anyhow::Result;
use bitcoin::Amount;
use rstest::*;
use spark_itest::helpers::{
    WalletsFixture, deposit_to_wallet, deposit_with_amount, wait_for, wallets,
};
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

/// Test that the greedy leaf selection algorithm's second pass succeeds.
/// When greedy fails due to a non-power-of-two leaf, it retries with only
/// power-of-two leaves and should find a valid combination without triggering a swap.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_transfer_with_odd_leaf_greedy_succeeds(
    #[future] wallets: WalletsFixture,
) -> Result<()> {
    let fixture = wallets.await;
    let alice = &fixture.alice_wallet;
    let bob = &fixture.bob_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    // Deposit leaves: [3000, 2048, 1024]
    // Total: 6072 sats
    deposit_with_amount(alice, bitcoind, 3000).await?;
    deposit_with_amount(alice, bitcoind, 2048).await?;
    deposit_with_amount(alice, bitcoind, 1024).await?;

    let alice_balance = alice.get_balance().await?;
    info!("Alice balance after deposits: {}", alice_balance);
    assert_eq!(alice_balance, 6072);

    // Transfer 3072 sats
    // Greedy pass 1: picks 3000, remaining=72, can't find → fails
    // Greedy pass 2: filters to [2048, 1024], picks 2048+1024=3072 → succeeds!
    // No swap needed
    let bob_address = bob.get_spark_address()?;
    info!("Bob's Spark address: {:?}", bob_address);

    alice.transfer(3072, &bob_address, None).await?;
    info!("Transfer completed");

    // Wait for Bob's balance to become the expected value
    wait_for(
        || async { bob.get_balance().await.unwrap_or(0) == 3072 },
        30,
        "Bob's balance to become 3072",
    )
    .await?;

    let bob_balance = bob.get_balance().await?;
    info!("Bob balance after transfer: {}", bob_balance);
    assert_eq!(bob_balance, 3072);

    let alice_balance_after = alice.get_balance().await?;
    info!("Alice balance after transfer: {}", alice_balance_after);
    assert_eq!(alice_balance_after, 3000); // Only the odd leaf remains

    Ok(())
}
