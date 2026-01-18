use anyhow::Result;
use bitcoin::Address;
use rstest::*;
use spark_itest::{
    faucet::RegtestFaucet,
    helpers::{create_regtest_wallet, wait_for_event},
    mempool::MempoolClient,
};
use spark_wallet::WalletEvent;
use tracing::info;

/// Test non-static deposit using deployed regtest with faucet funding.
///
/// This test:
/// 1. Creates a wallet connected to deployed regtest operators
/// 2. Generates a non-static deposit address
/// 3. Funds it via the regtest faucet
/// 4. Fetches the funding transaction from mempool
/// 5. Claims the deposit manually
/// 6. Waits for deposit confirmation
/// 7. Verifies the balance increased
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_non_static_deposit_with_faucet() -> Result<()> {
    info!("=== Starting test_non_static_deposit_with_faucet ===");

    let faucet = RegtestFaucet::new()?;
    let mempool = MempoolClient::new()?;

    // Create wallet with deployed regtest operators
    let (wallet, mut listener) = create_regtest_wallet().await?;

    // Record initial balance (should be 0 for fresh wallet)
    let initial_balance = wallet.get_balance().await?;
    info!("Initial balance: {} sats", initial_balance);

    // Generate non-static deposit address
    let deposit_address = wallet.generate_deposit_address(false).await?;
    info!("Generated deposit address: {}", deposit_address);

    // Fund via faucet
    let deposit_amount = 50_000u64;
    let txid = faucet
        .fund_address(&deposit_address.to_string(), deposit_amount)
        .await?;
    info!("Faucet funded address, txid: {}", txid);

    // Fetch transaction from mempool
    let tx = mempool.get_transaction(&txid).await?;
    info!("Fetched transaction with {} outputs", tx.output.len());

    // Find the output index for our deposit address
    let vout = tx
        .output
        .iter()
        .enumerate()
        .find(|(_, output)| {
            Address::from_script(&output.script_pubkey, bitcoin::Network::Regtest)
                .is_ok_and(|addr| addr == deposit_address)
        })
        .map(|(i, _)| i as u32)
        .expect("Could not find deposit address in transaction outputs");
    info!("Found deposit output at index: {}", vout);

    // Claim the deposit
    let leaves = wallet.claim_deposit(tx, vout).await?;
    info!("Claimed deposit, got {} leaves", leaves.len());

    // Wait for deposit confirmation
    wait_for_event(&mut listener, 180, "DepositConfirmed", |e| match e {
        WalletEvent::DepositConfirmed(_) => Ok(Some(e)),
        _ => Ok(None),
    })
    .await?;
    info!("Deposit confirmed");

    // Verify balance increased
    let final_balance = wallet.get_balance().await?;
    info!("Final balance: {} sats", final_balance);

    assert_eq!(
        final_balance,
        initial_balance + deposit_amount,
        "Balance should increase by deposit amount"
    );

    info!("=== Test test_non_static_deposit_with_faucet PASSED ===");
    Ok(())
}
