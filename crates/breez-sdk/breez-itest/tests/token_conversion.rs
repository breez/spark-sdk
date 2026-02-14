use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

/// Default token identifier for regtest (no USDB available)
const REGTEST_TOKEN_ID: &str = "btknrt1ra8lrwpqgqfz7gcy3gfcucaw3fh62tp3d6qkjxafx0cnxm5gmd3q0xy27c";

/// Test token conversions in both directions:
/// - Part A: Bitcoin → Token (Alice pays Bob's Spark address with token conversion)
/// - Part B: Token → Bitcoin (Bob pays Alice's Lightning invoice using received tokens)
#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "Skipping due liquidity issues causing test to fail"]
async fn test_token_conversion_success(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_token_conversion_success ===");
    let sats_to_token_success_amount: u128 = 20_000_000_000;
    let token_to_sats_success_amount: u64 = 2_500;

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // ==========================================
    // Part A: Bitcoin → Token conversion
    // ==========================================
    info!("--- Part A: Bitcoin to Token conversion ---");

    // Fund Alice with Bitcoin
    ensure_funded(&mut alice, 10_000).await?;

    let alice_initial_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice initial balance: {} sats", alice_initial_balance);

    // Verify Bob has no tokens initially
    let bob_initial_token_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);
    info!(
        "Bob initial token balance: {} units",
        bob_initial_token_balance
    );

    // Bob exposes a Spark address
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Bob's Spark address: {}", bob_spark_address);

    // Alice prepares and sends payment with token conversion (Bitcoin → Token)
    let prepare_btc_to_token = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(sats_to_token_success_amount),
            token_identifier: Some(REGTEST_TOKEN_ID.to_string()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: Some(200), // 2%
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;
    let conversion_estimate = prepare_btc_to_token
        .conversion_estimate
        .as_ref()
        .expect("Conversion estimate should be present");
    info!(
        "Prepared token payment: amount={:?} (converting bitcoin amount={}, fee={})",
        prepare_btc_to_token.amount, conversion_estimate.amount, conversion_estimate.fee
    );

    let send_btc_to_token = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_btc_to_token,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!(
        "Alice sent Bitcoin→Token payment: status={:?}, method={:?}",
        send_btc_to_token.payment.status, send_btc_to_token.payment.method
    );
    assert!(
        matches!(
            send_btc_to_token.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Bitcoin to Token payment should be completed or pending"
    );

    // Check payment conversion details
    let btc_to_token_conversion_details = send_btc_to_token.payment.conversion_details.unwrap();
    assert_eq!(
        btc_to_token_conversion_details.from.method,
        PaymentMethod::Spark,
        "From step should be a spark payment"
    );
    assert!(
        btc_to_token_conversion_details.from.fee > 0,
        "From step should have a fee"
    );
    assert!(
        btc_to_token_conversion_details
            .from
            .token_metadata
            .is_none(),
        "From step should have no token metadata"
    );
    assert_eq!(
        btc_to_token_conversion_details.to.method,
        PaymentMethod::Token,
        "To step should be a token payment"
    );
    assert_eq!(
        btc_to_token_conversion_details.to.fee, 0,
        "To step should have no fee"
    );
    assert!(
        btc_to_token_conversion_details.to.token_metadata.is_some(),
        "To step should have token metadata"
    );

    // Wait for Bob to receive the token payment
    info!("Waiting for Bob to receive token payment...");
    let bob_received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;

    assert_eq!(
        bob_received_payment.payment_type,
        PaymentType::Receive,
        "Bob should receive a payment"
    );
    assert_eq!(
        bob_received_payment.method,
        PaymentMethod::Token,
        "Bob should receive a token payment"
    );
    assert!(
        bob_received_payment.conversion_details.is_none(),
        "Should contain no conversion_details"
    );

    // Verify Bob received tokens
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_token_balance_after_btc_to_token = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    info!(
        "Bob token balance after Bitcoin→Token: {} units (received {} units)",
        bob_token_balance_after_btc_to_token,
        bob_token_balance_after_btc_to_token - bob_initial_token_balance
    );

    assert!(
        bob_token_balance_after_btc_to_token > bob_initial_token_balance,
        "Bob's token balance should increase after receiving token payment"
    );

    info!("Part A: Bitcoin to Token conversion completed successfully");

    // ==========================================
    // Part B: Token → Bitcoin conversion
    // ==========================================
    info!("--- Part B: Token to Bitcoin conversion ---");
    clear_event_receiver(&mut alice.events).await;

    // Get Alice's initial balance before receiving Bitcoin
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_balance_before_token_to_btc = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!(
        "Alice balance before Token→Bitcoin: {} sats",
        alice_balance_before_token_to_btc
    );

    // Alice creates a Lightning invoice for Bob to pay
    let alice_invoice = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Token to Bitcoin test".to_string(),
                amount_sats: Some(token_to_sats_success_amount),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;
    info!("Alice's Lightning invoice: {}", alice_invoice);

    // Bob prepares payment using his tokens (Token → Bitcoin conversion)
    let prepare_token_to_btc = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_invoice.clone(),
            amount: None, // Amount from invoice
            token_identifier: None,
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::ToBitcoin {
                    from_token_identifier: REGTEST_TOKEN_ID.to_string(),
                },
                max_slippage_bps: Some(200), // 2%
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;

    let conversion_estimate = prepare_token_to_btc
        .conversion_estimate
        .as_ref()
        .expect("Conversion estimate should be present");
    info!(
        "Prepared bitcoin payment: amount={:?} (converting token amount={}, fee={})",
        prepare_token_to_btc.amount, conversion_estimate.amount, conversion_estimate.fee
    );

    let send_token_to_btc = bob
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_token_to_btc,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!(
        "Bob sent Token→Bitcoin payment: status={:?}, method={:?}",
        send_token_to_btc.payment.status, send_token_to_btc.payment.method
    );
    assert!(
        matches!(
            send_token_to_btc.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Token to Bitcoin payment should be completed or pending"
    );

    // Check payment conversion details
    let token_to_btc_conversion_details = send_token_to_btc.payment.conversion_details.unwrap();
    assert_eq!(
        token_to_btc_conversion_details.from.method,
        PaymentMethod::Token,
        "From step should be a token payment"
    );
    assert!(
        token_to_btc_conversion_details.from.fee > 0,
        "From step should have a fee"
    );
    assert!(
        token_to_btc_conversion_details
            .from
            .token_metadata
            .is_some(),
        "From step should have token metadata"
    );
    assert_eq!(
        token_to_btc_conversion_details.to.method,
        PaymentMethod::Spark,
        "To step should be a spark payment"
    );
    assert_eq!(
        token_to_btc_conversion_details.to.fee, 0,
        "To step should have no fee"
    );
    assert!(
        token_to_btc_conversion_details.to.token_metadata.is_none(),
        "To step should have no token metadata"
    );

    // Wait for Alice to receive the Bitcoin payment
    info!("Waiting for Alice to receive Bitcoin payment...");
    let alice_received_payment =
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 30).await?;

    assert_eq!(
        alice_received_payment.payment_type,
        PaymentType::Receive,
        "Alice should receive a payment"
    );
    assert_eq!(
        alice_received_payment.amount, token_to_sats_success_amount as u128,
        "Alice should receive the exact invoice amount"
    );
    assert!(
        alice_received_payment.conversion_details.is_none(),
        "Should contain no conversion_details"
    );

    // Verify Alice received Bitcoin
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_balance_after_token_to_btc = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!(
        "Alice balance after Token→Bitcoin: {} sats (received {} sats)",
        alice_balance_after_token_to_btc,
        alice_balance_after_token_to_btc - alice_balance_before_token_to_btc
    );

    assert!(
        alice_balance_after_token_to_btc > alice_balance_before_token_to_btc,
        "Alice's Bitcoin balance should increase after receiving payment"
    );

    // Verify Bob's token balance decreased
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_token_balance_after_token_to_btc = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    info!(
        "Bob token balance after Token→Bitcoin: {} units (spent {} units)",
        bob_token_balance_after_token_to_btc,
        bob_token_balance_after_btc_to_token - bob_token_balance_after_token_to_btc
    );

    assert!(
        bob_token_balance_after_token_to_btc < bob_token_balance_after_btc_to_token,
        "Bob's token balance should decrease after Token→Bitcoin payment"
    );

    info!("Part B: Token to Bitcoin conversion completed successfully");
    info!("=== Test test_token_conversion_success PASSED ===");
    Ok(())
}

/// Test token conversion failure cases:
/// - Part A: Non-existent token (fails at prepare stage)
/// - Part B: Low slippage failure with refund verification
#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "Skipping due liquidity issues causing test to fail"]
async fn test_token_conversion_failure(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_token_conversion_failure ===");
    let sats_to_token_failure_amount: u128 = 40_000_000_000;

    let mut alice = alice_sdk.await?;
    let bob = bob_sdk.await?;

    // Fund Alice with Bitcoin (used for both parts)
    ensure_funded(&mut alice, 20_000).await?;

    let alice_initial_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice initial balance: {} sats", alice_initial_balance);

    // Bob exposes a Spark address (used for both parts)
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Bob's Spark address: {}", bob_spark_address);

    // ==========================================
    // Part A: Non-existent token (fails at prepare)
    // ==========================================
    info!("--- Part A: Non-existent token failure ---");

    let non_existent_token_id = "btknrt1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqdare99";

    let prepare_non_existent_result = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(sats_to_token_failure_amount),
            token_identifier: Some(non_existent_token_id.to_string()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: Some(100),
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await;
    assert!(
        prepare_non_existent_result.is_err(),
        "prepare_send_payment should fail with non-existent token"
    );
    info!("Part A: Non-existent token correctly rejected at prepare stage");

    // ==========================================
    // Part B: Low slippage failure with refund
    // ==========================================
    info!("--- Part B: Low slippage failure with refund ---");
    clear_event_receiver(&mut alice.events).await;

    // Record Alice's balance before the failed payment
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_balance_before_failure = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!(
        "Alice balance before low slippage attempt: {} sats",
        alice_balance_before_failure
    );

    // Prepare payment with very low slippage (2 bps = 0.02%)
    let prepare_low_slippage = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(sats_to_token_failure_amount),
            token_identifier: Some(REGTEST_TOKEN_ID.to_string()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: Some(2), // 0.02% - very low, likely to fail
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;

    let conversion_estimate = prepare_low_slippage
        .conversion_estimate
        .as_ref()
        .expect("Conversion estimate should be present");
    info!(
        "Prepared token payment: amount={:?} (converting bitcoin amount={}, fee={})",
        prepare_low_slippage.amount, conversion_estimate.amount, conversion_estimate.fee
    );

    // Send the payment - expect it to fail due to slippage
    let send_low_slippage_result = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_low_slippage,
            options: None,
            idempotency_key: None,
        })
        .await;
    assert!(
        send_low_slippage_result.is_err(),
        "send_payment should fail with low slippage"
    );

    // Wait for payment refund event
    info!("Waiting for payment refund event...");
    let refund_payment =
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 30).await?;

    assert_eq!(
        refund_payment.payment_type,
        PaymentType::Receive,
        "Alice should receive a refund payment"
    );
    assert_eq!(
        refund_payment.method,
        PaymentMethod::Spark,
        "Alice's refund should be via Spark"
    );

    // Verify Alice's balance is restored
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_balance_after_refund = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!(
        "Alice balance after refund: {} sats (was {} sats before failure)",
        alice_balance_after_refund, alice_balance_before_failure
    );
    assert_eq!(
        alice_balance_after_refund, alice_balance_before_failure,
        "Alice's balance should be restored after refund"
    );

    info!("Part B: Low slippage failure with refund completed successfully");
    info!("=== Test test_token_conversion_failure PASSED ===");
    Ok(())
}
