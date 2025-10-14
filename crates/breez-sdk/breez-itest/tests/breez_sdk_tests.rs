use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use rstest_reuse::{apply, template};
use tempdir::TempDir;
use tracing::{debug, info};

// ---------------------
// Fixtures
// ---------------------

/// Fixture: Alice's SDK with temporary storage
#[fixture]
async fn alice_sdk() -> Result<SdkInstance> {
    let alice_dir = TempDir::new("breez-sdk-alice")?;
    let path = alice_dir.path().to_string_lossy().to_string();

    // Generate random seed for Alice
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    info!("Initializing Alice's SDK at: {} with random seed", path);
    build_sdk_with_dir(path, seed, Some(alice_dir)).await
}

/// Fixture: Bob's SDK with temporary storage
#[fixture]
async fn bob_sdk() -> Result<SdkInstance> {
    let bob_dir = TempDir::new("breez-sdk-bob")?;
    let path = bob_dir.path().to_string_lossy().to_string();

    // Generate random seed for Bob
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    info!("Initializing Bob's SDK at: {} with random seed", path);
    build_sdk_with_dir(path, seed, Some(bob_dir)).await
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
            amount: Some(5),
            token_identifier: None,
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

// ---------------------
// Lightning Test Template
// ---------------------

/// Template for Lightning invoice payment tests with different invoice amounts
#[template]
#[rstest]
#[case::fixed_amount(Some(10_000), None, "fixed-amount")]
#[case::zero_amount(None, Some(10_000), "zero-amount")]
fn lightning_payment_cases(
    #[case] invoice_amount_sats: Option<u64>,
    #[case] sender_amount: Option<u64>,
    #[case] test_type: &str,
) {
}

/// Shared Lightning invoice payment test with parameterized invoice amount
#[apply(lightning_payment_cases)]
#[test_log::test(tokio::test)]
async fn test_03_lightning_invoice_payment(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
    #[case] invoice_amount_sats: Option<u64>,
    #[case] sender_amount: Option<u64>,
    #[case] test_type: &str,
) -> Result<()> {
    info!(
        "=== Starting test_03_lightning_invoice_payment ({}) ===",
        test_type
    );

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Ensure Alice is funded with enough for invoice + fees
    ensure_funded(&mut alice, 100_000).await?;

    // Get Alice's initial balance
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_initial_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Alice initial balance: {} sats", alice_initial_balance);

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

    // Bob creates a Lightning invoice (with or without amount)
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: format!("Test payment ({})", test_type),
                amount_sats: invoice_amount_sats,
            },
        })
        .await?
        .payment_request;

    info!("Bob's Lightning invoice ({}): {}", test_type, bob_invoice);

    // Alice prepares to pay Bob's invoice
    // For zero-amount invoices, Alice must specify the amount
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice.clone(),
            amount: sender_amount.map(|a| a as u128),
            token_identifier: None,
        })
        .await?;

    info!("Payment prepared - amount: {} sats", prepare.amount);

    // The expected payment amount is either from the invoice or what Alice specified
    let expected_amount = invoice_amount_sats
        .or(sender_amount)
        .expect("Amount must be specified");

    // Alice sends the payment
    info!(
        "Sending {} sats from Alice to Bob via Lightning ({})...",
        expected_amount, test_type
    );

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(10),
            }),
        })
        .await?;

    info!("Alice send payment status: {:?}", send_resp.payment.status);
    info!("Alice payment fees: {} sats", send_resp.payment.fees);
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
    wait_for_balance(
        &bob.sdk,
        Some(bob_initial_balance + expected_amount),
        None,
        20,
    )
    .await?;
    assert_eq!(
        received_payment.payment_type,
        PaymentType::Receive,
        "Bob should receive a payment"
    );
    assert_eq!(
        received_payment.amount, expected_amount as u128,
        "Bob should receive exactly {} sats",
        expected_amount
    );
    assert_eq!(
        received_payment.method,
        PaymentMethod::Lightning,
        "Payment should be via Lightning"
    );

    info!(
        "Bob received payment: {} sats, fees: {} sats, status: {:?}, method: {:?}",
        received_payment.amount,
        received_payment.fees,
        received_payment.status,
        received_payment.method
    );

    // Verify Alice's balance decreased by amount + fees
    let mut sent_payment = wait_for_payment_event(&mut alice.events, 60).await?;
    if sent_payment.payment_type != PaymentType::Send {
        sent_payment = wait_for_payment_event(&mut alice.events, 60).await?;
    }
    wait_for_balance(
        &alice.sdk,
        Some(alice_initial_balance - sent_payment.amount as u64 - sent_payment.fees as u64),
        None,
        20,
    )
    .await?;
    assert_eq!(
        sent_payment.payment_type,
        PaymentType::Send,
        "Alice should send a payment"
    );
    //alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_final_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    let alice_balance_change = alice_initial_balance as i64 - alice_final_balance as i64;
    info!(
        "Alice's balance: {} -> {} sats (change: -{})",
        alice_initial_balance, alice_final_balance, alice_balance_change
    );

    assert!(
        alice_final_balance < alice_initial_balance,
        "Alice's balance should decrease"
    );

    info!(
        "Alice paid {} sats total (amount: {}, fees: {})",
        alice_balance_change, send_resp.payment.amount, send_resp.payment.fees
    );

    // Verify Bob's balance increased by invoice amount
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    let bob_balance_change = bob_final_balance as i64 - bob_initial_balance as i64;
    info!(
        "Bob's balance: {} -> {} sats (change: +{})",
        bob_initial_balance, bob_final_balance, bob_balance_change
    );

    assert!(
        bob_final_balance > bob_initial_balance,
        "Bob's balance should increase"
    );
    assert_eq!(
        bob_balance_change, expected_amount as i64,
        "Bob should receive exactly {} sats",
        expected_amount
    );

    // Verify payment appears in Alice's payment list
    info!("Verifying Alice's payment list...");
    let alice_payment = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: send_resp.payment.id,
        })
        .await?
        .payment;

    assert_eq!(
        alice_payment.payment_type,
        PaymentType::Send,
        "Alice should have a Send payment"
    );
    assert_eq!(
        alice_payment.amount, expected_amount as u128,
        "Payment amount should match invoice"
    );
    assert!(
        alice_payment.fees > 0,
        "Lightning payment should have non-zero fees"
    );
    assert_eq!(
        alice_payment.method,
        PaymentMethod::Lightning,
        "Payment method should be Lightning"
    );

    info!(
        "Alice's payment record - id: {}, amount: {} sats, fees: {} sats, method: {:?}",
        alice_payment.id, alice_payment.amount, alice_payment.fees, alice_payment.method
    );

    // Verify payment appears in Bob's payment list
    info!("Verifying Bob's payment list...");
    let bob_payment = bob
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: received_payment.id,
        })
        .await?
        .payment;

    assert_eq!(
        bob_payment.payment_type,
        PaymentType::Receive,
        "Bob should have a Receive payment"
    );
    assert_eq!(
        bob_payment.amount, expected_amount as u128,
        "Payment amount should match invoice"
    );
    assert_eq!(bob_payment.fees, 0, "Receiver should not pay fees");
    assert_eq!(
        bob_payment.method,
        PaymentMethod::Lightning,
        "Payment method should be Lightning"
    );

    info!(
        "Bob's payment record - id: {}, amount: {} sats, fees: {} sats, method: {:?}",
        bob_payment.id, bob_payment.amount, bob_payment.fees, bob_payment.method
    );

    // Final verification: Alice paid = Bob received
    assert_eq!(
        alice_payment.amount, bob_payment.amount,
        "Sent amount should equal received amount"
    );

    info!(
        "✓ Payment verified: Alice sent {} sats + {} fees, Bob received {} sats",
        alice_payment.amount, alice_payment.fees, bob_payment.amount
    );

    info!(
        "=== Test test_03_lightning_invoice_payment ({}) PASSED ===",
        test_type
    );
    Ok(())
}
