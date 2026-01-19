use std::sync::Arc;

use anyhow::Result;
use bitcoin::Amount;
use rand::Rng;
use rstest::*;
use spark_wallet::{DefaultSigner, Network, SparkWallet, SparkWalletConfig, WalletEvent};
use tokio::sync::broadcast::Receiver;
use tracing::{debug, info};

use crate::fixtures::{
    bitcoind::BitcoindFixture,
    setup::{TestFixtures, create_test_signer_alice, create_test_signer_bob},
};

pub async fn wait_for_event<F>(
    event_rx: &mut Receiver<WalletEvent>,
    timeout_secs: u64,
    event_name: &str,
    mut matcher: F,
) -> Result<WalletEvent>
where
    F: FnMut(WalletEvent) -> Result<Option<WalletEvent>>,
{
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!(
                "Timeout waiting for {} event after {} seconds",
                event_name,
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Ok(event)) => {
                match matcher(event) {
                    Ok(Some(result)) => return Ok(result),
                    Ok(None) => {
                        // Not the event we're looking for, keep waiting
                        continue;
                    }
                    Err(e) => {
                        // Matcher returned an error (e.g., failure event)
                        return Err(e);
                    }
                }
            }
            Ok(Err(_)) => {
                anyhow::bail!("Event channel closed unexpectedly");
            }
            Err(_) => {
                anyhow::bail!(
                    "Timeout waiting for {} event after {} seconds",
                    event_name,
                    timeout_secs
                );
            }
        }
    }
}

/// Create a wallet connected to deployed regtest operators with a random seed.
/// Waits for the wallet to sync before returning.
///
/// # Returns
/// A tuple of (SparkWallet, event listener)
pub async fn create_regtest_wallet() -> Result<(SparkWallet, Receiver<WalletEvent>)> {
    let config = SparkWalletConfig::default_config(Network::Regtest);

    let mut seed = [0u8; 32];
    rand::thread_rng().fill(&mut seed);
    let signer = Arc::new(DefaultSigner::new(&seed, Network::Regtest)?);

    info!("Connecting wallet to deployed regtest operators...");
    let wallet = SparkWallet::connect(config, signer).await?;
    let mut listener = wallet.subscribe_events();

    // Wait for initial sync
    wait_for_event(&mut listener, 60, "Synced", |e| match e {
        WalletEvent::Synced => Ok(Some(e)),
        _ => Ok(None),
    })
    .await?;

    info!("Wallet synced successfully");
    Ok((wallet, listener))
}

// ============================================================================
// Local Docker test fixtures
// ============================================================================

#[fixture]
pub async fn test_fixtures() -> TestFixtures {
    TestFixtures::new()
        .await
        .expect("Failed to initialize test fixtures")
}

pub struct WalletsFixture {
    pub fixtures: TestFixtures,
    pub alice_wallet: SparkWallet,
    pub bob_wallet: SparkWallet,
}

#[fixture]
pub async fn wallets(#[future] test_fixtures: TestFixtures) -> WalletsFixture {
    let fixtures = test_fixtures.await;
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
            .expect("Failed to receive alice wallet event");
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

pub async fn deposit_to_wallet(wallet: &SparkWallet, bitcoind: &BitcoindFixture) -> Result<()> {
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
