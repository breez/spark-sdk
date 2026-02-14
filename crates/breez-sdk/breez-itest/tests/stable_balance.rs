use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

/// Default token identifier for regtest
const REGTEST_TOKEN_ID: &str = "btknrt1ra8lrwpqgqfz7gcy3gfcucaw3fh62tp3d6qkjxafx0cnxm5gmd3q0xy27c";

/// Test stable balance auto-conversion feature:
/// 1. Fund bob and send sats to Alice (below threshold) - verify no auto-conversion
/// 2. Bob sends sats again to Alice (to exceed threshold) - verify auto-conversion triggers
/// 3. Alice pays Bob again using token-to-bitcoin conversion
#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "Skipping due liquidity issues causing test to fail"]
async fn test_stable_balance_auto_conversion(
    #[future] alice_sdk_stable_balance: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_stable_balance_auto_conversion ===");

    let mut alice = alice_sdk_stable_balance.await?;
    let mut bob = bob_sdk.await?;

    // ==========================================
    // Part 1: Fund bob and send sats below threshold - no auto-conversion
    // ==========================================
    info!("--- Part 1: Fund bob and send sats below threshold (500 sats) ---");

    // Fund Bob with Bitcoin
    ensure_funded(&mut bob, 10_000).await?;

    // Alice creates a Spark address to receive
    let alice_spark_address = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    // Record Alice's token balance before receiving
    let alice_token_balance_before = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    // Bob sends 500 sats to Alice (below threshold of 1000)
    let prepare_small = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_spark_address.clone(),
            amount: Some(500),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_small,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Wait for Bob to send the payment
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Send, 30).await?;

    // Wait for Alice to receive the payment
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 30).await?;

    // Wait for synced event
    wait_for_synced_event(&mut alice.events, 3).await?;

    let alice_info_after_small = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;

    info!(
        "Alice after receiving 500 sats: {} sats, {} tokens",
        alice_info_after_small.balance_sats,
        alice_info_after_small
            .token_balances
            .get(REGTEST_TOKEN_ID)
            .map(|b| b.balance)
            .unwrap_or(0)
    );

    // Verify sats balance increased (no auto-conversion because below threshold)
    assert!(
        alice_info_after_small.balance_sats >= 500,
        "Alice should have at least 500 sats (got {})",
        alice_info_after_small.balance_sats
    );

    // Verify token balance didn't change (no auto-conversion)
    let alice_token_balance_after_small = alice_info_after_small
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    assert_eq!(
        alice_token_balance_after_small, alice_token_balance_before,
        "Alice's token balance should not change when sats below threshold"
    );

    info!("Part 1 complete: No auto-conversion (below threshold)");

    // ==========================================
    // Part 2: Bob sends more sats to exceed threshold - auto-conversion triggers
    // ==========================================
    info!("--- Part 2: Bob sends sats to exceed threshold (5000 sats) ---");

    // Bob sends 5000 more sats to Alice (now total exceeds threshold)
    let prepare_large = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_spark_address.clone(),
            amount: Some(5000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_large,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Wait for Bob to send the payment
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Send, 30).await?;

    // Wait for all auto-conversion events: receive spark → send spark → receive token
    wait_for_auto_conversion_events(&mut alice.events, PaymentMethod::Spark, 60).await?;

    // Sync to get updated balances
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_info_after_large = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let alice_token_balance_after_large = alice_info_after_large
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    info!(
        "Alice after receiving 5000 more sats: {} sats, {} tokens (was {} tokens)",
        alice_info_after_large.balance_sats,
        alice_token_balance_after_large,
        alice_token_balance_after_small
    );

    // Verify auto-conversion happened (sats decreased, tokens increased)
    assert_eq!(
        alice_info_after_large.balance_sats, 0,
        "Alice's sats should be converted (got {} sats)",
        alice_info_after_large.balance_sats
    );

    assert!(
        alice_token_balance_after_large > alice_token_balance_after_small,
        "Alice's token balance should increase after auto-conversion ({} > {})",
        alice_token_balance_after_large,
        alice_token_balance_after_small
    );

    info!("Part 2 complete: Auto-conversion triggered successfully");

    // ==========================================
    // Part 3: Alice sends tokens directly to Bob
    // ==========================================
    info!("--- Part 3: Alice sends tokens to Bob ---");

    let token_payment_amount: u128 = 1_000_000;

    // Bob creates a Spark address to receive
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    // Alice sends tokens directly to Bob
    let prepare_token = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(token_payment_amount),
            token_identifier: Some(REGTEST_TOKEN_ID.to_string()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!(
        "Alice prepared token payment: amount={:?}",
        prepare_token.amount
    );

    let send_result = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_token,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!(
        "Alice sent token payment: status={:?}",
        send_result.payment.status
    );

    // Wait for Alice to send the payment
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 30).await?;

    // Wait for Bob to receive the payment
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;

    // Verify Alice's token balance decreased
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_final_info = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;

    let alice_final_token_balance = alice_final_info
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    info!(
        "Alice final balances: {} sats, {} tokens (was {} tokens)",
        alice_final_info.balance_sats, alice_final_token_balance, alice_token_balance_after_large
    );

    assert!(
        alice_final_token_balance < alice_token_balance_after_large,
        "Alice's token balance should decrease after token payment"
    );

    info!("Part 3 complete: Token payment successful");
    info!("=== Test test_stable_balance_auto_conversion PASSED ===");
    Ok(())
}

/// Test stable balance with reserved sats:
/// 1. Fund bob and send sats to Alice (above threshold + reserve)
/// 2. Verify auto-conversion triggers but leaves reserved sats as Bitcoin
/// 3. Alice pays Bob using the reserved Bitcoin balance (no conversion needed)
#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "Skipping due liquidity issues causing test to fail"]
async fn test_stable_balance_reserved_sats(
    #[future] alice_sdk_stable_balance_with_reserve: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_stable_balance_reserved_sats ===");

    let mut alice = alice_sdk_stable_balance_with_reserve.await?;
    let mut bob = bob_sdk.await?;

    // Config: threshold_sats=1000, reserved_sats=2000
    // Auto-conversion triggers when balance > 1000 + 2000 = 3000
    // And converts only (balance - 2000) sats

    // ==========================================
    // Part 1: Fund bob and send sats to Alice in two payments (above threshold+reserve)
    // This creates multiple token outputs so Part 3 can use them concurrently
    // ==========================================
    info!("--- Part 1: Send sats above threshold+reserve (two payments of 5000 sats) ---");

    // Fund Bob with Bitcoin
    ensure_funded(&mut bob, 20_000).await?;

    // Record Alice's initial token balance
    let alice_token_balance_before = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    // First payment: 5000 sats to Alice
    let alice_spark_address = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare_1 = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_spark_address.clone(),
            amount: Some(5000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_1,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!("Wait for Bob to send the payment");
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Send, 30).await?;

    info!("Wait for all auto-conversion events: receive spark → send spark → receive token");
    wait_for_auto_conversion_events(&mut alice.events, PaymentMethod::Spark, 60).await?;

    // Sync to get updated balances
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_info_after_first = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let alice_token_balance_after_first = alice_info_after_first
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    info!(
        "Alice after first payment: {} tokens (was {})",
        alice_token_balance_after_first, alice_token_balance_before
    );

    info!("--- Part 1b: Second payment: 5000 sats to Alice (creates a second token output) ---");
    // Second payment: 5000 sats to Alice (creates a second token output)
    let prepare_2 = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_spark_address,
            amount: Some(5000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_2,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!("Wait for Bob to send the payment");
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Send, 30).await?;

    info!("Wait for all auto-conversion events: receive spark → send spark → receive token");
    wait_for_auto_conversion_events(&mut alice.events, PaymentMethod::Spark, 60).await?;

    // Sync to get updated sats balance
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_info = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let alice_token_balance = alice_info
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    info!(
        "Alice after two payments: {} sats, {} tokens (was {} tokens)",
        alice_info.balance_sats, alice_token_balance, alice_token_balance_before
    );

    // Verify auto-conversion happened but reserved ~2000 sats
    // Allow tolerance for conversion fees/rounding
    assert!(
        alice_info.balance_sats >= 2000 && alice_info.balance_sats <= 2100,
        "Alice should have ~2000 reserved sats (got {} sats)",
        alice_info.balance_sats
    );

    assert!(
        alice_token_balance > alice_token_balance_before,
        "Alice's token balance should increase after auto-conversion ({} > {})",
        alice_token_balance,
        alice_token_balance_before
    );

    info!("Part 1 complete: Auto-conversion with reserved sats");

    // ==========================================
    // Part 2: Alice pays Bob using reserved Bitcoin (no conversion)
    // ==========================================
    info!("--- Part 2: Alice pays Bob using reserved Bitcoin ---");

    let payment_amount: u128 = 1_000;

    // Bob creates a Spark address to receive
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    // Alice prepares payment using her reserved sats (no conversion needed)
    let prepare_btc = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(payment_amount),
            token_identifier: None,
            conversion_options: None, // No conversion - using reserved sats
            fee_policy: None,
        })
        .await?;

    info!(
        "Alice prepared Bitcoin payment: amount={:?}",
        prepare_btc.amount
    );

    let send_result = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_btc,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!(
        "Alice sent Bitcoin payment: status={:?}",
        send_result.payment.status
    );

    info!("Wait for Alice to send the payment");
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 30).await?;

    info!("Wait for Bob to receive the payment");
    let bob_received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;

    assert_eq!(
        bob_received.amount, payment_amount,
        "Bob should receive the exact payment amount"
    );

    // Verify Alice's sats balance decreased but tokens unchanged
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_final_info = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let alice_final_token_balance = alice_final_info
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    info!(
        "Alice final balances: {} sats (was ~2000), {} tokens (was {})",
        alice_final_info.balance_sats, alice_final_token_balance, alice_token_balance
    );

    assert!(
        alice_final_info.balance_sats < alice_info.balance_sats,
        "Alice's sats should decrease after payment"
    );

    // Token balance should remain the same (no conversion used)
    assert_eq!(
        alice_final_token_balance, alice_token_balance,
        "Alice's token balance should remain unchanged when paying with reserved sats"
    );

    info!("Part 2 complete: Bitcoin payment using reserved sats successful");

    // ==========================================
    // Part 3: Alice pays Bob over the reserve (require token→BTC conversion)
    // ==========================================
    info!("--- Part 3: Alice pays Bob over the reserve ---");

    let payment_amount: u128 = 2_000;

    // Bob creates a Spark address to receive
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    // Alice prepares payment (over reserve, needs token→BTC)
    let prepare_btc = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(payment_amount),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    info!(
        "Alice prepared Bitcoin payment: amount={:?}",
        prepare_btc.amount
    );
    assert!(
        prepare_btc.conversion_estimate.is_some(),
        "Should have a conversion estimate"
    );

    let send_result = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_btc,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!(
        "Alice sent Bitcoin payment: status={:?}",
        send_result.payment.status
    );

    info!("Wait for all conversion events: send token → receive spark → send spark");
    wait_for_payment_conversion_events(&mut alice.events, PaymentMethod::Spark, 60).await?;

    info!("Wait for Bob to receive the payment");
    let bob_received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;

    assert_eq!(
        bob_received.amount, payment_amount,
        "Bob should receive the exact payment amount"
    );

    // Verify Alice's sats balance decreased but tokens unchanged
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_final_info = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let alice_final_token_balance = alice_final_info
        .token_balances
        .get(REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);

    info!(
        "Alice final balances: {} sats (was ~1000), {} tokens (was {})",
        alice_final_info.balance_sats, alice_final_token_balance, alice_token_balance
    );

    assert!(
        alice_final_info.balance_sats >= alice_info.balance_sats,
        "Alice's sats might increase marginally after payment"
    );
    assert!(
        alice_final_token_balance < alice_token_balance,
        "Alice's token balance should decrease after payment"
    );

    info!("Part 3 complete: Bitcoin payment using reserved sats successful");

    // ==========================================
    // Part 4: Concurrent Bitcoin payments
    // - 2 payments over the reserve (require token→BTC conversion, using separate token outputs)
    // - 1 payment under the reserve (uses BTC directly)
    // ==========================================
    info!("--- Part 4: Concurrent Bitcoin payments (2 conversion + 1 direct) ---");

    let conversion_payment_amount: u128 = 2_000;
    let direct_payment_amount: u128 = 500;

    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    // Prepare two conversion payments (over reserve, needs token→BTC)
    let prepare_conv_1 = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(conversion_payment_amount),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let prepare_conv_2 = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(conversion_payment_amount),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    // Prepare one direct BTC payment (under reserve, no conversion)
    let prepare_direct = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address,
            amount: Some(direct_payment_amount),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let send_fut_1 = alice.sdk.send_payment(SendPaymentRequest {
        prepare_response: prepare_conv_1,
        options: None,
        idempotency_key: None,
    });
    let send_fut_2 = alice.sdk.send_payment(SendPaymentRequest {
        prepare_response: prepare_conv_2,
        options: None,
        idempotency_key: None,
    });
    let send_fut_3 = alice.sdk.send_payment(SendPaymentRequest {
        prepare_response: prepare_direct,
        options: None,
        idempotency_key: None,
    });

    let results = futures::future::try_join_all(vec![send_fut_1, send_fut_2, send_fut_3]).await?;

    // All three payments should succeed
    for (i, result) in results.iter().enumerate() {
        assert!(
            matches!(result.payment.status, PaymentStatus::Completed),
            "Payment {} should complete (got {:?})",
            i + 1,
            result.payment.status
        );
    }

    // Wait for Bob to receive all three payments
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;

    info!("Part 4 complete: Concurrent payments successful");
    info!("=== Test test_stable_balance_reserved_sats PASSED ===");

    Ok(())
}
