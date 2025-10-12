use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tempdir::TempDir;
use tokio_with_wasm::alias as tokio;
use tracing::{debug, info};

// ---------------------
// Fixtures
// ---------------------

/// Fixture: Alice's SDK with temporary storage
#[fixture]
async fn alice_sdk() -> Result<SdkInstance> {
    let alice_dir = TempDir::new("breez-sdk-alice")?;
    let path = alice_dir.path().to_string_lossy().to_string();
    info!("Initializing Alice's SDK at: {}", path);
    build_sdk_with_dir(path, [2u8; 32], Some(alice_dir)).await
}

/// Fixture: Bob's SDK with temporary storage
#[fixture]
async fn bob_sdk() -> Result<SdkInstance> {
    let bob_dir = TempDir::new("breez-sdk-bob")?;
    let path = bob_dir.path().to_string_lossy().to_string();
    info!("Initializing Bob's SDK at: {}", path);
    build_sdk_with_dir(path, [3u8; 32], Some(bob_dir)).await
}

// ---------------------
// Helper Functions
// ---------------------

/// Ensure SDK has at least the specified balance, funding if necessary
async fn ensure_funded(sdk_instance: &mut SdkInstance, min_balance: u64) -> Result<()> {
    // Sync to get latest balance
    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let info = sdk_instance
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;

    if info.balance_sats < min_balance {
        let needed = min_balance - info.balance_sats;
        info!(
            "Current balance: {} sats, need {} more sats. Funding with 50,000 sats...",
            info.balance_sats, needed
        );
        receive_and_fund(sdk_instance, 50_000).await?;
    } else {
        info!(
            "Already funded with {} sats (minimum: {} sats)",
            info.balance_sats, min_balance
        );
    }

    Ok(())
}

// ---------------------
// Tests
// ---------------------

/// Test 1: Send payment from Alice to Bob using Spark transfer
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_spark_transfer(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_01_spark_transfer ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Ensure Alice is funded (100 sats minimum for small test)
    ensure_funded(&mut alice, 100).await?;

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Alice balance: {} sats", alice_balance);

    // Get Bob's initial balance
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_initial_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Bob initial balance: {} sats", bob_initial_balance);

    // Bob exposes a Spark address
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    info!("Bob's Spark address: {}", bob_spark_address);

    // Alice prepares and sends 5 sats to Bob
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount_sats: Some(5),
        })
        .await?;

    info!("Sending 5 sats from Alice to Bob via Spark...");

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
        })
        .await?;

    info!("Alice send payment status: {:?}", send_resp.payment.status);
    assert!(
        matches!(
            send_resp.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Payment should be completed or pending"
    );

    // Wait for Bob to receive the payment via event
    info!("Waiting for Bob to receive payment event...");
    let received_payment = wait_for_payment_event(&mut bob.events, 60).await?;

    assert_eq!(
        received_payment.payment_type,
        PaymentType::Receive,
        "Bob should receive a payment"
    );
    assert!(
        received_payment.amount >= 5,
        "Bob should receive at least 5 sats"
    );

    info!(
        "Bob received payment: {} sats, status: {:?}",
        received_payment.amount, received_payment.status
    );

    // Verify Bob's balance increased
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!(
        "Bob's balance: {} -> {} sats (change: +{})",
        bob_initial_balance,
        bob_final_balance,
        bob_final_balance as i64 - bob_initial_balance as i64
    );

    assert!(
        bob_final_balance > bob_initial_balance,
        "Bob's balance should increase"
    );

    info!("=== Test test_01_spark_transfer PASSED ===");
    Ok(())
}

/// Test 2: Verify deposit claim functionality
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_deposit_claim(#[future] alice_sdk: Result<SdkInstance>) -> Result<()> {
    info!("=== Starting test_02_deposit_claim ===");

    let mut alice = alice_sdk.await?;

    // Ensure Alice has some funds to begin with
    ensure_funded(&mut alice, 100).await?;

    let initial_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Alice initial balance: {} sats", initial_balance);

    // Fund with a small amount to test claim (10,000 sats)
    info!("Funding additional 10,000 sats to test auto-claim...");
    let (deposit_address, txid) = receive_and_fund(&mut alice, 10_000).await?;

    info!(
        "Funded deposit address: {}, txid: {}",
        deposit_address, txid
    );

    // Balance should have increased (auto-claimed by SDK)
    let final_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    debug!("Alice final balance: {} sats", final_balance);
    assert!(
        final_balance > initial_balance,
        "Balance should increase after deposit claim"
    );

    info!(
        "Balance increased from {} to {} sats (+{})",
        initial_balance,
        final_balance,
        final_balance - initial_balance
    );

    info!("=== Test test_02_deposit_claim PASSED ===");
    Ok(())
}
