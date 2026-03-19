use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

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
        .get(SHELL_REGTEST_TOKEN_ID)
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
            .get(SHELL_REGTEST_TOKEN_ID)
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
        .get(SHELL_REGTEST_TOKEN_ID)
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
        .get(SHELL_REGTEST_TOKEN_ID)
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
            token_identifier: Some(SHELL_REGTEST_TOKEN_ID.to_string()),
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
        .get(SHELL_REGTEST_TOKEN_ID)
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
