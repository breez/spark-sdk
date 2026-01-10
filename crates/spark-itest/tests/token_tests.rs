use anyhow::Result;
use rand::Rng;
use rstest::*;
use spark_itest::helpers::wait_for_event;
use spark_wallet::{DefaultSigner, Network, SelectionStrategy, TransferTokenOutput, WalletEvent};
use tracing::info;

/// Test creating many outputs via self-sends then send all to Bob
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_many_outputs() -> Result<()> {
    use spark_wallet::{SparkWallet, SparkWalletConfig};
    use std::sync::Arc;

    info!("=== Starting test_many_outputs ===");

    // Use production operators for this test since they have DKG keys for test issuers
    let mut config = SparkWalletConfig::default_config(Network::Regtest);
    config.self_payment_allowed = true;

    // Use random seeds for Alice and Bob
    let mut alice_seed = [0u8; 32];
    rand::thread_rng().fill(&mut alice_seed);
    let signer_alice = Arc::new(DefaultSigner::new(&alice_seed, Network::Regtest).unwrap());

    let mut bob_seed = [0u8; 32];
    rand::thread_rng().fill(&mut bob_seed);
    let signer_bob = Arc::new(DefaultSigner::new(&bob_seed, Network::Regtest).unwrap());

    let alice_wallet = SparkWallet::connect(config.clone(), signer_alice).await?;
    let bob_wallet = SparkWallet::connect(config, signer_bob).await?;

    // Wait for wallets to sync
    let mut alice_listener = alice_wallet.subscribe_events();
    let mut bob_listener = bob_wallet.subscribe_events();
    wait_for_event(&mut alice_listener, 30, "Synced", |event| match event {
        WalletEvent::Synced => Ok(Some(event)),
        _ => Ok(None),
    })
    .await?;
    wait_for_event(&mut bob_listener, 30, "Synced", |event| match event {
        WalletEvent::Synced => Ok(Some(event)),
        _ => Ok(None),
    })
    .await?;

    // Create and mint test token with larger supply for many outputs
    alice_wallet
        .create_issuer_token(
            "Many Outputs",
            "MANY",
            2,
            false,
            2_000_000, // Larger supply for many small outputs
        )
        .await?;
    info!("Created token");

    alice_wallet.mint_issuer_token(2_000_000).await?;
    info!("Minted 2,000,000 tokens");

    // Sync wallet to ensure minting is processed
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    alice_wallet.sync().await?;

    // Get token metadata to get the identifier
    let token_metadata = alice_wallet.get_issuer_token_metadata().await?;
    let token_identifier = token_metadata.identifier.clone();

    info!(
        "Created and minted token: {} ({}) with 2,000,000 supply",
        token_metadata.name, token_identifier
    );

    // Verify initial balance
    let initial_balance = alice_wallet
        .get_token_balances()
        .await?
        .get(&token_identifier)
        .unwrap()
        .balance;

    assert_eq!(
        initial_balance, 2_000_000,
        "Alice should have 2,000,000 tokens initially"
    );

    // Get Alice's own Spark address for self-sends
    let alice_spark_address = alice_wallet.get_spark_address()?;
    info!(
        "Alice's Spark address for self-sends: {:?}",
        alice_spark_address
    );

    // Perform self-sends to create many outputs
    let num_self_sends = 4;
    let outputs_per_send = 300; // total outputs = 4 * 300 = 1200 > 500 * 2 (will require 2 optimization rounds)
    let self_send_amount = 5;

    info!(
        "Starting {num_self_sends} self-sends creating {outputs_per_send} outputs each (total {} outputs)...",
        num_self_sends * outputs_per_send
    );

    for i in 0..num_self_sends {
        info!("Progress: {i}/{num_self_sends} self-sends completed");

        // Create multiple outputs in a single transaction
        let outputs: Vec<TransferTokenOutput> = (0..outputs_per_send)
            .map(|_| TransferTokenOutput {
                token_id: token_identifier.clone(),
                amount: self_send_amount,
                receiver_address: alice_spark_address.clone(),
                spark_invoice: None,
            })
            .collect();

        alice_wallet
            .transfer_tokens(outputs, None, Some(SelectionStrategy::LargestFirst))
            .await?;
    }

    info!("Completed {} self-sends", num_self_sends);

    // Sync wallet to ensure all self-sends are processed
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    alice_wallet.sync().await?;

    // Verify balance after self-sends (should be unchanged since sending to self)
    let balance_after_self_sends = alice_wallet
        .get_token_balances()
        .await?
        .get(&token_identifier)
        .unwrap()
        .balance;

    // Balance should be the same since we're sending to ourselves
    assert_eq!(
        balance_after_self_sends, initial_balance,
        "Balance should remain the same after self-sends"
    );

    info!(
        "Balance after {} self-sends: {}",
        num_self_sends, balance_after_self_sends
    );

    // Now attempt to send ALL funds to Bob
    let bob_spark_address = bob_wallet.get_spark_address()?;
    info!("Bob's Spark address: {:?}", bob_spark_address);

    // Create a single output sending all tokens to Bob
    let outputs_to_bob = vec![TransferTokenOutput {
        token_id: token_identifier.clone(),
        amount: balance_after_self_sends,
        receiver_address: bob_spark_address,
        spark_invoice: None,
    }];

    alice_wallet
        .transfer_tokens(outputs_to_bob, None, None)
        .await?;

    // Sync both wallets to see final state
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    alice_wallet.sync().await?;
    bob_wallet.sync().await?;

    // Check final balances
    let alice_final_balance = alice_wallet
        .get_token_balances()
        .await?
        .get(&token_identifier)
        .map(|b| b.balance)
        .unwrap_or(0);

    let bob_final_balance = bob_wallet
        .get_token_balances()
        .await?
        .get(&token_identifier)
        .map(|b| b.balance)
        .unwrap_or(0);

    info!(
        "Final balances - Alice: {}, Bob: {} (total should be {})",
        alice_final_balance, bob_final_balance, initial_balance
    );

    // Verify the transfer was successful - Alice should have 0, Bob should have all tokens
    assert_eq!(
        alice_final_balance, 0,
        "Alice should have 0 tokens after sending all to Bob"
    );
    assert_eq!(
        bob_final_balance, initial_balance,
        "Bob should have all {} tokens after receiving from Alice",
        initial_balance
    );

    info!("=== Test test_many_outputs PASSED ===");
    Ok(())
}
