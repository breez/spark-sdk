use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use rstest_reuse::{apply, template};
use tracing::{debug, info};

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
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!("Sending 5 sats from Alice to Bob via Spark...");

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
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

    // Wait for Bob to receive payment pending event
    info!("Waiting for Bob to receive pending payment event...");
    let pending_payment =
        wait_for_payment_pending_event(&mut bob.events, PaymentType::Receive, 60).await?;

    // Confirm payment is immediately available for listing
    let payment = bob
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: pending_payment.id,
        })
        .await?
        .payment;
    assert_eq!(
        payment.status,
        PaymentStatus::Pending,
        "Payment should be pending"
    );

    // Wait for Bob to receive payment succeeded event
    info!("Waiting for Bob to receive payment succeeded event...");
    let received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(
        received_payment.payment_type,
        PaymentType::Receive,
        "Bob should receive a payment"
    );
    assert!(
        received_payment.amount >= 5,
        "Bob should receive at least 5 sats"
    );

    // Confirm payment is now completed
    let payment = bob
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: received_payment.id,
        })
        .await?
        .payment;
    assert_eq!(
        payment.status,
        PaymentStatus::Completed,
        "Payment should be completed"
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
    let (deposit_address, txid) = receive_and_fund(&mut alice, 10_000, true).await?;

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
                expiry_secs: None,
                payment_hash: None,
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
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!("Payment prepared - amount: {:?}", prepare.amount);

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
            idempotency_key: None,
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
    let received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
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
    let sent_payment =
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;
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

/// Test 5: Lightning invoice with prefer_spark true should use spark fee path
#[rstest]
#[test_log::test(tokio::test)]
async fn test_05_lightning_invoice_prefer_spark_fee_path(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_05_lightning_invoice_prefer_spark_fee_path ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Ensure Alice is funded (cover amount + any fees)
    ensure_funded(&mut alice, 50_000).await?;

    // Bob creates a Lightning invoice with a fixed amount
    let invoice_amount_sats = 2_000u64;
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Prefer spark test".to_string(),
                amount_sats: Some(invoice_amount_sats),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    // Prepare payment; expect spark_transfer_fee_sats is Some (likely 0) when invoice contains spark route hint
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice.clone(),
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    // Validate preparation outputs
    if let SendPaymentMethod::Bolt11Invoice {
        spark_transfer_fee_sats,
        lightning_fee_sats,
        ..
    } = &prepare.payment_method
    {
        info!(
            "Prepared fees: spark={:?}, lightning={}",
            spark_transfer_fee_sats, lightning_fee_sats
        );
        // If spark hint exists, spark fee should be defined (0 expected in current setup)
        assert!(
            spark_transfer_fee_sats.is_some(),
            "Expected spark_transfer_fee_sats to be present"
        );
    } else {
        anyhow::bail!("Expected Bolt11Invoice payment method in prepare response");
    }

    // Send with prefer_spark = true and wait for completion
    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: true,
                completion_timeout_secs: Some(10),
            }),
            idempotency_key: None,
        })
        .await?;

    info!(
        "Alice send status: {:?}, method: {:?}, fees: {}",
        send_resp.payment.status, send_resp.payment.method, send_resp.payment.fees
    );
    // Prefer spark should route via spark path with zero fees; method may be Spark depending on path
    assert_eq!(
        send_resp.payment.fees, 0,
        "Expect zero fee when using prefer_spark"
    );
    assert!(matches!(send_resp.payment.payment_type, PaymentType::Send));

    // Bob should receive the amount
    let received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(received.amount, invoice_amount_sats as u128);
    // Receiver should see Spark method when routed via prefer_spark
    assert!(matches!(received.method, PaymentMethod::Spark));

    info!("=== Test test_05_lightning_invoice_prefer_spark_fee_path PASSED ===");
    Ok(())
}

/// Test 6: Lightning payment with short completion timeout returns quickly, then completes
#[rstest]
#[test_log::test(tokio::test)]
async fn test_06_lightning_timeout_and_wait(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_06_lightning_timeout_and_wait ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    ensure_funded(&mut alice, 60_000).await?;

    // Bob creates a zero-amount invoice
    let expected_amount = 7_000u64;
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Timeout test".to_string(),
                amount_sats: None,
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice.clone(),
            amount: Some(expected_amount as u128),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    // Send with a very short completion timeout to force an early return if still pending
    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(1),
            }),
            idempotency_key: None,
        })
        .await?;
    info!("Immediate return status: {:?}", send_resp.payment.status);
    assert!(matches!(send_resp.payment.status, PaymentStatus::Pending));
    // Bob should have received the exact amount
    let received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(received.amount, expected_amount as u128);

    info!("=== Test test_06_lightning_timeout_and_wait PASSED ===");
    Ok(())
}

/// Test 7: Send payment from Alice to Bob using Spark invoice
#[rstest]
#[test_log::test(tokio::test)]
async fn test_07_spark_invoice(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_07_spark_invoice ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Ensure Alice is funded (100 sats minimum for small test)
    ensure_funded(&mut alice, 100).await?;

    let alice_initial_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Alice balance: {} sats", alice_initial_balance);

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

    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let expiry_time = current_time + 120;

    // Get Alice's identity public key from her Spark address to use as sender public key in the invoice
    let alice_spark_address = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let InputType::SparkAddress(address_details) =
        bob.sdk.parse(&alice_spark_address).await.unwrap()
    else {
        return Err(anyhow::anyhow!("Failed to parse Alice's Spark address"));
    };
    let alice_identity_public_key = address_details.identity_public_key;

    // Bob creates a Spark invoice
    let bob_spark_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                amount: Some(5),
                token_identifier: None,
                expiry_time: Some(expiry_time),
                description: Some("Test invoice".to_string()),
                sender_public_key: Some(alice_identity_public_key),
            },
        })
        .await?
        .payment_request;

    info!("Bob's Spark invoice: {}", bob_spark_invoice);

    // Alice prepares and sends 5 sats to Bob
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_invoice.clone(),
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!("Sending 5 sats from Alice to Bob via Spark...");

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
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
    let received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

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

    info!("=== Test test_07_spark_invoice PASSED ===");
    Ok(())
}

/// Test 8: Lightning invoice with custom expiry_secs
#[rstest]
#[test_log::test(tokio::test)]
async fn test_08_lightning_invoice_expiry_secs(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_08_lightning_invoice_expiry_secs ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Ensure Alice is funded
    ensure_funded(&mut alice, 50_000).await?;

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

    // Test with custom expiry_secs (1 hour = 3600 seconds)
    let custom_expiry_secs: u32 = 3600 - 1;
    let invoice_amount_sats = 5_000u64;

    // Bob creates a Lightning invoice with custom expiry
    let receive_response = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Test invoice with custom expiry".to_string(),
                amount_sats: Some(invoice_amount_sats),
                expiry_secs: Some(custom_expiry_secs),
                payment_hash: None,
            },
        })
        .await?;

    let bob_invoice = receive_response.payment_request;
    info!(
        "Bob's Lightning invoice with {} secs expiry: {}",
        custom_expiry_secs, bob_invoice
    );

    // Parse the invoice to verify expiry is set
    let parsed = bob.sdk.parse(&bob_invoice).await?;
    if let InputType::Bolt11Invoice(invoice_details) = parsed {
        info!(
            "Parsed invoice - amount: {:?} msat, expiry: {} secs",
            invoice_details.amount_msat, invoice_details.expiry
        );

        // Verify the expiry matches what we requested
        assert_eq!(
            invoice_details.expiry, custom_expiry_secs as u64,
            "Invoice expiry should match requested expiry_secs"
        );

        // Verify the amount is correct (in millisats)
        assert_eq!(
            invoice_details.amount_msat,
            Some(invoice_amount_sats * 1000),
            "Invoice amount should match requested amount"
        );
    } else {
        anyhow::bail!("Expected Bolt11Invoice input type");
    }

    // Alice prepares to pay Bob's invoice
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice.clone(),
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!("Payment prepared - amount: {:?}", prepare.amount);

    // Alice sends the payment
    info!(
        "Sending {} sats from Alice to Bob via Lightning with custom expiry...",
        invoice_amount_sats
    );

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(10),
            }),
            idempotency_key: None,
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

    // Wait for Bob to receive the payment
    info!("Waiting for Bob to receive payment event...");
    let received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

    assert_eq!(
        received_payment.payment_type,
        PaymentType::Receive,
        "Bob should receive a payment"
    );
    assert_eq!(
        received_payment.amount, invoice_amount_sats as u128,
        "Bob should receive the exact amount"
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

    info!("=== Test test_08_lightning_invoice_expiry_secs PASSED ===");
    Ok(())
}

/// Test 9: Bolt11 send all with fee overpayment
/// Tests FeePolicy::FeesIncluded when fee drops between prepare and send
#[rstest]
#[test_log::test(tokio::test)]
async fn test_09_bolt11_send_all_with_fee_overpayment(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_09_bolt11_send_all_with_fee_overpayment ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Fund Alice with enough sats to have room for searching fee tiers
    ensure_funded(&mut alice, 50_000).await?;

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice balance after funding: {} sats", alice_balance);

    // Bob creates an amountless Lightning invoice
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Bolt11 FeesIncluded overpayment test".to_string(),
                amount_sats: None,
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;
    info!("Bob's amountless invoice: {}", bob_invoice);

    // Minimum sendable amount for Lightning
    let min_sendable_sats = 1_000u64;

    // Helper to get lightning fee for an amount
    async fn get_fee(sdk: &BreezSdk, invoice: &str, amount: u64) -> Result<u64> {
        let prepare = sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: invoice.to_string(),
                amount: Some(amount as u128),
                token_identifier: None,
                conversion_options: None,
                fee_policy: None,
            })
            .await?;
        match prepare.payment_method {
            SendPaymentMethod::Bolt11Invoice {
                lightning_fee_sats, ..
            } => Ok(lightning_fee_sats),
            _ => anyhow::bail!("Expected Bolt11Invoice payment method"),
        }
    }

    // Search for fee tier boundary using binary search
    info!("Searching for fee tier boundary using binary search...");

    let fee_at_min = get_fee(&alice.sdk, &bob_invoice, min_sendable_sats).await?;
    let fee_at_max = get_fee(&alice.sdk, &bob_invoice, alice_balance).await?;

    info!(
        "Fee at min ({} sats): {} sats, fee at max ({} sats): {} sats",
        min_sendable_sats, fee_at_min, alice_balance, fee_at_max
    );

    if fee_at_min >= fee_at_max {
        anyhow::bail!(
            "No fee tier boundary found - fees are constant or decreasing ({} -> {})",
            fee_at_min,
            fee_at_max
        );
    }

    // Binary search to find where fee changes
    let mut low = min_sendable_sats;
    let mut high = alice_balance;
    let fee_low = fee_at_min;

    while high - low > 1 {
        let mid = low + (high - low) / 2;
        let fee_mid = get_fee(&alice.sdk, &bob_invoice, mid).await?;
        debug!(
            "Binary search: low={}, mid={}, high={}, fee_mid={}",
            low, mid, high, fee_mid
        );
        if fee_mid == fee_low {
            low = mid;
        } else {
            high = mid;
        }
    }

    // high is now the boundary where fee increases
    let target_balance = high;
    let fee1 = get_fee(&alice.sdk, &bob_invoice, target_balance).await?;
    let adjusted = target_balance.saturating_sub(fee1);
    let fee2 = get_fee(&alice.sdk, &bob_invoice, adjusted).await?;

    info!(
        "Found fee tier boundary at {} sats: fee1={}, fee2={} (for adjusted={})",
        target_balance, fee1, fee2, adjusted
    );

    if fee2 >= fee1 {
        anyhow::bail!(
            "Fee stepping not found at boundary: fee({})={}, fee({})={}",
            target_balance,
            fee1,
            adjusted,
            fee2
        );
    }

    let (expected_fee1, expected_fee2) = (fee1, fee2);

    info!(
        "Using stepping balance: {} sats (fee will step from {} to {})",
        target_balance, expected_fee1, expected_fee2
    );

    // Adjust Alice's balance to target using Spark transfer
    if alice_balance > target_balance {
        let excess = alice_balance - target_balance;
        info!(
            "Adjusting Alice's balance: sending {} sats to Bob via Spark",
            excess
        );

        // Bob creates a Spark address
        let bob_spark_address = bob
            .sdk
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::SparkAddress,
            })
            .await?
            .payment_request;

        // Alice sends excess to Bob
        let prepare = alice
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: bob_spark_address,
                amount: Some(excess as u128),
                token_identifier: None,
                conversion_options: None,
                fee_policy: None,
            })
            .await?;

        alice
            .sdk
            .send_payment(SendPaymentRequest {
                prepare_response: prepare,
                options: None,
                idempotency_key: None,
            })
            .await?;

        // Wait for Spark transfer to complete
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

        // Sync and verify
        alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
        let new_balance = alice
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?
            .balance_sats;

        info!(
            "Alice balance after adjustment: {} sats (target was {})",
            new_balance, target_balance
        );
        assert_eq!(
            new_balance, target_balance,
            "Alice's balance should match target"
        );
    }

    // Execute payment with FeesIncluded
    info!("Executing bolt11 payment with fee overpayment...");

    let prepare_response = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice.clone(),
            amount: Some(target_balance as u128),
            token_identifier: None,
            conversion_options: None,
            fee_policy: Some(FeePolicy::FeesIncluded),
        })
        .await?;

    // Get the fee from the prepare response
    // For amountless Bolt11 with FeesIncluded, invoice_details.amount_msat is None
    let prepared_fee = match &prepare_response.payment_method {
        SendPaymentMethod::Bolt11Invoice {
            lightning_fee_sats, ..
        } => *lightning_fee_sats,
        _ => anyhow::bail!("Expected Bolt11Invoice payment method"),
    };

    info!("Prepared payment: fee={} sats", prepared_fee);

    // The fee should be expected_fee1 (the higher fee for full balance)
    assert_eq!(
        prepared_fee, expected_fee1,
        "Payment fee should match expected fee for full balance"
    );

    // Execute the payment
    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_response.clone(),
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(30),
            }),
            idempotency_key: None,
        })
        .await?;

    info!("Bolt11 full balance payment initiated");

    // Wait for payment to complete on both sides
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;
    info!("Full balance payment completed on Alice's side");

    let bob_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    info!("Full balance payment completed on Bob's side");

    // Verify Alice's balance is zero
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_final = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice final balance: {} sats", alice_final);

    assert_eq!(alice_final, 0, "Alice's balance should be fully spent");

    // With fee overpayment, Bob receives target_balance - fee2 (the actual lower fee)
    // Because: receiver_amount = target_balance - fee1, overpayment = fee1 - fee2
    // Final amount = receiver_amount + overpayment = target_balance - fee2
    let expected_bob_amount = target_balance - expected_fee2;
    assert_eq!(
        bob_payment.amount,
        expected_bob_amount.into(),
        "Bob should receive target_balance minus actual fee (with overpayment applied)"
    );

    // Verify payment details
    let alice_payment = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: send_resp.payment.id,
        })
        .await?
        .payment;

    assert_eq!(alice_payment.payment_type, PaymentType::Send);
    assert_eq!(alice_payment.method, PaymentMethod::Lightning);
    assert_eq!(alice_payment.status, PaymentStatus::Completed);

    info!(
        "Fee overpayment test passed! Expected overpayment: {} sats",
        expected_fee1 - expected_fee2
    );
    info!("=== Test test_09_bolt11_send_all_with_fee_overpayment PASSED ===");
    Ok(())
}
