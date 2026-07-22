use std::sync::Arc;

use anyhow::Result;
use rand::Rng;
use rstest::*;
use spark_itest::backend::resolve_backend;
use spark_itest::helpers::{build_test_wallet, wait_for_event};
use spark_wallet::{
    DefaultSigner, Network, SelectionStrategy, SparkInvoiceToFulfill, SparkWallet,
    SparkWalletConfig, TransferTokenOutput, WalletEvent,
};
use tracing::info;

/// Test creating many outputs via self-sends then send all to Bob
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_many_outputs() -> Result<()> {
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

    let alice_backend = resolve_backend().await?;
    let alice_wallet = build_test_wallet(config.clone(), signer_alice, &alice_backend).await?;
    let mut alice_listener = alice_wallet.subscribe_events();
    alice_wallet.start_background_processing().await;

    let bob_backend = resolve_backend().await?;
    let bob_wallet = build_test_wallet(config, signer_bob, &bob_backend).await?;
    let mut bob_listener = bob_wallet.subscribe_events();
    bob_wallet.start_background_processing().await;

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

/// Builds a wallet that has issued and minted `supply` of its own token,
/// returning the wallet, its event listener and the token identifier.
async fn issuer_wallet(
    config: &SparkWalletConfig,
    name: &str,
    ticker: &str,
    supply: u128,
) -> Result<(SparkWallet, String)> {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill(&mut seed);
    let signer = Arc::new(DefaultSigner::new(&seed, Network::Regtest).unwrap());
    let backend = resolve_backend().await?;
    let wallet = build_test_wallet(config.clone(), signer, &backend).await?;
    let mut listener = wallet.subscribe_events();
    wallet.start_background_processing().await;
    wait_for_event(&mut listener, 30, "Synced", |event| match event {
        WalletEvent::Synced => Ok(Some(event)),
        _ => Ok(None),
    })
    .await?;

    wallet
        .create_issuer_token(name, ticker, 2, false, supply)
        .await?;
    wallet.mint_issuer_token(supply).await?;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    wallet.sync().await?;

    let token_identifier = wallet.get_issuer_token_metadata().await?.identifier;
    Ok((wallet, token_identifier))
}

async fn plain_wallet(config: &SparkWalletConfig) -> Result<SparkWallet> {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill(&mut seed);
    let signer = Arc::new(DefaultSigner::new(&seed, Network::Regtest).unwrap());
    let backend = resolve_backend().await?;
    let wallet = build_test_wallet(config.clone(), signer, &backend).await?;
    let mut listener = wallet.subscribe_events();
    wallet.start_background_processing().await;
    wait_for_event(&mut listener, 30, "Synced", |event| match event {
        WalletEvent::Synced => Ok(Some(event)),
        _ => Ok(None),
    })
    .await?;
    Ok(wallet)
}

async fn token_balance(wallet: &SparkWallet, token_identifier: &str) -> Result<u128> {
    Ok(wallet
        .get_token_balances()
        .await?
        .get(token_identifier)
        .map(|b| b.balance)
        .unwrap_or(0))
}

/// One transaction paying two recipients the same token.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_single_token_multiple_recipients() -> Result<()> {
    let config = SparkWalletConfig::default_config(Network::Regtest);

    let (alice, token) = issuer_wallet(&config, "Fan Out", "FAN", 100_000).await?;
    let bob = plain_wallet(&config).await?;
    let dave = plain_wallet(&config).await?;

    let tx = alice
        .transfer_tokens(
            vec![
                TransferTokenOutput {
                    token_id: token.clone(),
                    amount: 700,
                    receiver_address: bob.get_spark_address()?,
                    spark_invoice: None,
                },
                TransferTokenOutput {
                    token_id: token.clone(),
                    amount: 300,
                    receiver_address: dave.get_spark_address()?,
                    spark_invoice: None,
                },
            ],
            None,
            None,
        )
        .await?;
    info!("Fan-out transaction: {}", tx.hash);

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    alice.sync().await?;
    bob.sync().await?;
    dave.sync().await?;

    assert_eq!(token_balance(&bob, &token).await?, 700);
    assert_eq!(token_balance(&dave, &token).await?, 300);
    assert_eq!(token_balance(&alice, &token).await?, 100_000 - 1_000);

    // Both payments came from one transaction, and Alice's change is in it too.
    assert_eq!(tx.outputs.len(), 3, "two recipients plus Alice's change");
    Ok(())
}

/// One transaction spending two different tokens, which the operators must accept
/// as a single mixed-token transfer.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_multiple_tokens_in_one_transaction() -> Result<()> {
    let config = SparkWalletConfig::default_config(Network::Regtest);

    let (alice, token_a) = issuer_wallet(&config, "Token Alpha", "ALPH", 50_000).await?;
    let (carol, token_b) = issuer_wallet(&config, "Token Beta", "BETA", 50_000).await?;
    let bob = plain_wallet(&config).await?;

    // Carol funds Alice with token B so Alice holds both tokens.
    carol
        .transfer_tokens(
            vec![TransferTokenOutput {
                token_id: token_b.clone(),
                amount: 5_000,
                receiver_address: alice.get_spark_address()?,
                spark_invoice: None,
            }],
            None,
            None,
        )
        .await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    alice.sync().await?;
    assert_eq!(token_balance(&alice, &token_b).await?, 5_000);

    // One transaction paying Bob in both tokens.
    let tx = alice
        .transfer_tokens(
            vec![
                TransferTokenOutput {
                    token_id: token_a.clone(),
                    amount: 400,
                    receiver_address: bob.get_spark_address()?,
                    spark_invoice: None,
                },
                TransferTokenOutput {
                    token_id: token_b.clone(),
                    amount: 900,
                    receiver_address: bob.get_spark_address()?,
                    spark_invoice: None,
                },
            ],
            None,
            None,
        )
        .await?;
    info!("Mixed-token transaction: {}", tx.hash);

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    alice.sync().await?;
    bob.sync().await?;

    assert_eq!(token_balance(&bob, &token_a).await?, 400);
    assert_eq!(token_balance(&bob, &token_b).await?, 900);
    assert_eq!(token_balance(&alice, &token_a).await?, 50_000 - 400);
    assert_eq!(token_balance(&alice, &token_b).await?, 5_000 - 900);

    // Two payments plus one change output per token, all in this one transaction.
    assert_eq!(tx.outputs.len(), 4);
    let mixed = tx
        .outputs
        .iter()
        .map(|o| o.token_identifier.clone())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(mixed.len(), 2, "transaction carries both tokens");
    Ok(())
}

/// One transaction fulfilling a Spark invoice from each of two recipients.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_multiple_invoices_in_one_transaction() -> Result<()> {
    let config = SparkWalletConfig::default_config(Network::Regtest);

    let (alice, token) = issuer_wallet(&config, "Invoiced", "INV", 100_000).await?;
    let bob = plain_wallet(&config).await?;
    let dave = plain_wallet(&config).await?;

    let bob_invoice = bob
        .create_spark_invoice(Some(250), Some(token.clone()), None, None, None)
        .await?;
    let dave_invoice = dave
        .create_spark_invoice(Some(150), Some(token.clone()), None, None, None)
        .await?;

    let tx = alice
        .fulfill_token_spark_invoices(vec![
            SparkInvoiceToFulfill {
                invoice: bob_invoice,
                amount: None,
            },
            SparkInvoiceToFulfill {
                invoice: dave_invoice,
                amount: None,
            },
        ])
        .await?;
    info!("Multi-invoice transaction: {}", tx.hash);

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    alice.sync().await?;
    bob.sync().await?;
    dave.sync().await?;

    assert_eq!(token_balance(&bob, &token).await?, 250);
    assert_eq!(token_balance(&dave, &token).await?, 150);
    assert_eq!(
        tx.fulfilled_invoices.len(),
        2,
        "both invoices marked fulfilled by the one transaction"
    );
    Ok(())
}

/// Paying the same invoice twice in one call is rejected before anything is sent.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_duplicate_invoice_rejected() -> Result<()> {
    let config = SparkWalletConfig::default_config(Network::Regtest);

    let (alice, token) = issuer_wallet(&config, "Dup Check", "DUP", 10_000).await?;
    let bob = plain_wallet(&config).await?;

    let invoice = bob
        .create_spark_invoice(Some(100), Some(token.clone()), None, None, None)
        .await?;

    let result = alice
        .fulfill_token_spark_invoices(vec![
            SparkInvoiceToFulfill {
                invoice: invoice.clone(),
                amount: None,
            },
            SparkInvoiceToFulfill {
                invoice,
                amount: None,
            },
        ])
        .await;

    assert!(result.is_err(), "duplicate invoice must be rejected");
    assert_eq!(
        token_balance(&alice, &token).await?,
        10_000,
        "nothing was sent"
    );
    Ok(())
}
