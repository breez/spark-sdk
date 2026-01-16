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
            pay_amount: Some(PayAmount::Bitcoin { amount_sats: 500 }),
            conversion_options: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_small,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Wait for Alice to receive the payment
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 30).await?;

    // Wait for sync
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

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

    // Verify token balance didn't change significantly (no auto-conversion)
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
    clear_event_receiver(&mut alice.events).await;
    clear_event_receiver(&mut bob.events).await;

    // ==========================================
    // Part 2: Bob sends more sats to exceed threshold - auto-conversion triggers
    // ==========================================
    info!("--- Part 2: Bob sends sats to exceed threshold (9000 sats) ---");

    // Bob sends 9000 more sats to Alice (now total exceeds threshold)
    let prepare_large = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_spark_address.clone(),
            pay_amount: Some(PayAmount::Bitcoin { amount_sats: 9000 }),
            conversion_options: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_large,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Wait for Alice to receive the payment
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 30).await?;

    // Wait for sync and auto-conversion
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
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
        "Alice after receiving 9000 more sats: {} sats, {} tokens (was {} tokens)",
        alice_info_after_large.balance_sats,
        alice_token_balance_after_large,
        alice_token_balance_after_small
    );

    // Verify auto-conversion happened (sats decreased, tokens increased)
    assert!(
        alice_info_after_large.balance_sats < 100,
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
    clear_event_receiver(&mut bob.events).await;

    // ==========================================
    // Part 3: Alice pays Bob using token-to-bitcoin conversion
    // ==========================================
    info!("--- Part 3: Alice pays Bob using token-to-bitcoin conversion ---");

    let payment_amount: u64 = 2_000;

    // Bob creates a Lightning invoice
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Stable balance test payment".to_string(),
                amount_sats: Some(payment_amount),
                expiry_secs: None,
            },
        })
        .await?
        .payment_request;

    // Alice prepares payment using her tokens (Token → Bitcoin conversion)
    let prepare_token_to_btc = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice.clone(),
            pay_amount: None,
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::ToBitcoin {
                    from_token_identifier: REGTEST_TOKEN_ID.to_string(),
                },
                max_slippage_bps: Some(200), // 2%
                completion_timeout_secs: None,
            }),
        })
        .await?;

    info!(
        "Alice prepared Token→Bitcoin payment: amount={:?}",
        prepare_token_to_btc.pay_amount
    );

    let send_result = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_token_to_btc,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!(
        "Alice sent Token→Bitcoin payment: status={:?}",
        send_result.payment.status
    );

    // Wait for Bob to receive the payment
    let bob_received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;

    assert_eq!(
        bob_received.amount, payment_amount as u128,
        "Bob should receive the exact invoice amount"
    );

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
        "Alice's token balance should decrease after payment"
    );

    info!("Part 3 complete: Token-to-bitcoin payment successful");
    info!("=== Test test_stable_balance_auto_conversion PASSED ===");
    Ok(())
}
