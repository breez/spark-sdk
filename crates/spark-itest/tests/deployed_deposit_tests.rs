use anyhow::Result;
use bitcoin::Address;
use rstest::*;
use spark_itest::{
    faucet::RegtestFaucet,
    helpers::{create_regtest_wallet, wait_for_event},
    mempool::MempoolClient,
};
use spark_wallet::{ExitSpeed, WalletEvent};
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

    // Transfer deposited funds to a second wallet
    let (bob, mut bob_listener) = create_regtest_wallet().await?;
    let bob_address = bob.get_spark_address()?;
    info!("Bob's Spark address: {:?}", bob_address);

    let _transfer = wallet.transfer(deposit_amount, &bob_address, None).await?;
    info!("Transfer initiated from Alice to Bob");

    // Wait for TransferClaimed event on Bob
    wait_for_event(
        &mut bob_listener,
        180,
        "TransferClaimed",
        |event| match &event {
            WalletEvent::TransferClaimed(_) => Ok(Some(event)),
            _ => Ok(None),
        },
    )
    .await?;
    info!("TransferClaimed received by Bob");

    // Verify balances
    let alice_balance = wallet.get_balance().await?;
    let bob_balance = bob.get_balance().await?;
    info!(
        "Alice balance: {} sats, Bob balance: {} sats",
        alice_balance, bob_balance
    );

    assert_eq!(
        alice_balance, 0,
        "Alice should have zero balance after transfer"
    );
    assert_eq!(
        bob_balance, deposit_amount,
        "Bob should have received the full deposit amount"
    );

    info!("=== Test test_non_static_deposit_with_faucet PASSED ===");
    Ok(())
}

/// Test non-static deposit followed by cooperative withdrawal to an on-chain address.
///
/// This test exercises the `create_connector_refund_txs` path with `direct_from_cpfp_tx`,
/// which must be created regardless of whether a `direct_tx` exists.
///
/// Steps:
/// 1. Alice deposits via faucet (non-static deposit)
/// 2. Alice withdraws (coop exit) to Bob's non-static deposit address
/// 3. Verifies Alice's balance is zero after withdrawal
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_non_static_deposit_then_coop_withdraw() -> Result<()> {
    info!("=== Starting test_non_static_deposit_then_coop_withdraw ===");

    let faucet = RegtestFaucet::new()?;
    let mempool = MempoolClient::new()?;

    // Create Alice's wallet and deposit funds
    let (alice, mut alice_listener) = create_regtest_wallet().await?;

    let deposit_address = alice.generate_deposit_address(false).await?;
    info!("Alice deposit address: {}", deposit_address);

    let deposit_amount = 50_000u64;
    let txid = faucet
        .fund_address(&deposit_address.to_string(), deposit_amount)
        .await?;
    info!("Faucet funded address, txid: {}", txid);

    let tx = mempool.get_transaction(&txid).await?;
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

    let leaves = alice.claim_deposit(tx, vout).await?;
    info!("Claimed deposit, got {} leaves", leaves.len());

    wait_for_event(&mut alice_listener, 180, "DepositConfirmed", |e| match e {
        WalletEvent::DepositConfirmed(_) => Ok(Some(e)),
        _ => Ok(None),
    })
    .await?;
    info!("Deposit confirmed");

    let alice_balance = alice.get_balance().await?;
    assert_eq!(alice_balance, deposit_amount);

    // Create Bob's wallet and generate an on-chain deposit address for withdrawal target
    let (bob, _bob_listener) = create_regtest_wallet().await?;
    let bob_deposit_address = bob.generate_deposit_address(false).await?;
    info!("Bob on-chain deposit address: {}", bob_deposit_address);

    // Coop withdraw all of Alice's funds to Bob's on-chain address
    let withdrawal_address = bob_deposit_address.to_string();
    let fee_quote = alice
        .fetch_coop_exit_fee_quote(&withdrawal_address, None)
        .await?;
    let fee_sats = fee_quote.fee_sats(&ExitSpeed::Slow);
    info!("Coop exit fee quote: {} sats (slow)", fee_sats);

    let _transfer = alice
        .withdraw(&withdrawal_address, None, ExitSpeed::Slow, fee_quote, None)
        .await?;
    info!("Withdrawal initiated");

    // Verify Alice's balance is zero
    let alice_balance = alice.get_balance().await?;
    info!("Alice balance after withdrawal: {} sats", alice_balance);
    assert_eq!(
        alice_balance, 0,
        "Alice should have zero balance after withdrawal"
    );

    info!("=== Test test_non_static_deposit_then_coop_withdraw PASSED ===");
    Ok(())
}
