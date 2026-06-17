//! Token conversion integration tests (mainnet, env-gated).
//!
//! These exercise the Flashnet token↔Bitcoin conversion paths, which need real
//! pool liquidity that regtest lacks. They run against **mainnet** and skip
//! automatically (logging a warning, returning `Ok`) unless the credentials
//! below are set, so normal CI is unaffected.
//!
//! # Required environment variables
//! - `MAINNET_TEST_MNEMONIC` — mnemonic of a pre-funded mainnet test account
//!   ("Alice", the funder). Also the primary gate.
//! - `BREEZ_API_KEY` — API key the mainnet SDK requires to function.
//!
//! # Optional
//! - `MAINNET_TEST_TOKEN_ID` — token to convert against; defaults to USDB.
//!
//! # Wallets & funds
//! Alice is the env account (keeps her funds). "Bob" is derived deterministically
//! from Alice's mnemonic (fixed passphrase). There is no per-test sweep — the
//! dedicated `mainnet_teardown` test drains Bob back to Alice (converting tokens
//! to sats) once per run, and recovers anything a failed run left behind on the
//! next run. Amounts are derived from the live pool via `fetch_conversion_limits`;
//! slippage/timeout inherit the SDK defaults (`None`).
//!
//! # Run locally
//! ```bash
//! MAINNET_TEST_MNEMONIC="..." BREEZ_API_KEY="..." \
//!   cargo test -p breez-sdk-itest --test mainnet_token_conversion -- --test-threads=1 --nocapture
//! ```

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use tracing::{info, warn};

/// Test token conversions in both directions:
/// - Part A: Bitcoin → Token (Alice pays Bob's Spark address with token conversion)
/// - Part B: Token → Bitcoin (Bob pays Alice's Lightning invoice using received tokens)
#[test_log::test(tokio::test)]
async fn test_token_conversion_success() -> Result<()> {
    let Some((mut alice, mut bob, token_id, snap)) = mainnet_test_setup(false, true).await? else {
        return Ok(());
    };
    info!("=== Starting test_token_conversion_success ===");

    // The pool only exposes input minimums, so size off the Token→BTC
    // minimum token input. Part A delivers Bob 3x that so he has enough
    // for Part B (with margin).
    let to_btc_min_token = tobtc_min_token_input(&alice.sdk, &token_id).await?;
    let part_a_token_amount = to_btc_min_token.saturating_mul(3);

    // ==========================================
    // Part A: Bitcoin → Token conversion
    // ==========================================
    info!("--- Part A: Bitcoin to Token conversion ---");

    // Verify Bob's starting token balance
    let bob_initial_token_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    info!("Bob initial token balance: {bob_initial_token_balance} units");

    // Bob exposes a Spark address
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Bob's Spark address: {bob_spark_address}");

    // Alice prepares and sends payment with token conversion (Bitcoin → Token).
    // Slippage/timeout inherit the SDK defaults.
    let prepare_btc_to_token = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address.clone(),
            },
            amount: Some(part_a_token_amount),
            token_identifier: Some(token_id.clone()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: None,
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
        prepare_btc_to_token.amount, conversion_estimate.amount_out, conversion_estimate.fee
    );

    // Skip rather than fail if Alice can't cover the actual Bitcoin input.
    let part_a_cost = conversion_estimate
        .amount_in
        .saturating_add(conversion_estimate.fee);
    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    if u128::from(alice_balance) < part_a_cost {
        warn!(
            "Skipping test_token_conversion_success: Alice balance {alice_balance} sats \
                     < Part A cost ~{part_a_cost} sats"
        );
        return Ok(());
    }

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
        btc_to_token_conversion_details.conversions.len(),
        1,
        "Should have exactly one conversion (AMM)"
    );
    let btc_to_token_conv = &btc_to_token_conversion_details.conversions[0];
    assert_eq!(
        btc_to_token_conv.from.chain,
        ConversionChain::Spark,
        "From chain should be spark"
    );
    assert_eq!(
        btc_to_token_conv.from.asset.ticker, "BTC",
        "From asset should be BTC"
    );
    assert_eq!(
        btc_to_token_conv.from.asset.decimals, 0,
        "From side (BTC/sats) should report decimals=0"
    );
    assert_eq!(
        btc_to_token_conv.to.chain,
        ConversionChain::Spark,
        "To chain should be spark"
    );
    assert_ne!(
        btc_to_token_conv.to.asset.ticker, "BTC",
        "To asset should be a token"
    );
    assert!(
        btc_to_token_conv.to.asset.decimals > 0,
        "To side (token) should have decimals > 0"
    );
    // The conversion charges a fee (attributed to the token leg on mainnet).
    assert!(
        btc_to_token_conv.from.fee + btc_to_token_conv.to.fee > 0,
        "Conversion should charge a fee"
    );

    // Wait for Bob to receive the token payment
    info!("Waiting for Bob to receive token payment...");
    let bob_received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

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
        .get(&token_id)
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
    info!("Alice balance before Token→Bitcoin: {alice_balance_before_token_to_btc} sats");

    // Size the invoice off the sats produced by converting the minimum
    // token input (the pool exposes no output minimum). Bob holds 3x that,
    // so he can comfortably cover it.
    let invoice_sats = u64::try_from(
        estimate_tobtc_sats_out(&bob.sdk, &bob_spark_address, &token_id, to_btc_min_token).await?,
    )?;
    info!("Part B invoice sized at {invoice_sats} sats (from pool estimate)");

    // Alice creates a Lightning invoice for Bob to pay
    let alice_invoice = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Token to Bitcoin test".to_string(),
                amount_sats: Some(invoice_sats),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;
    info!("Alice's Lightning invoice: {alice_invoice}");

    // Bob prepares payment using his tokens (Token → Bitcoin conversion)
    let prepare_token_to_btc = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: alice_invoice.clone(),
            },
            amount: None, // Amount from invoice
            token_identifier: None,
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::ToBitcoin {
                    from_token_identifier: token_id.clone(),
                },
                max_slippage_bps: None,
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
        prepare_token_to_btc.amount, conversion_estimate.amount_out, conversion_estimate.fee
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
        token_to_btc_conversion_details.conversions.len(),
        1,
        "Should have exactly one conversion (AMM)"
    );
    let token_to_btc_conv = &token_to_btc_conversion_details.conversions[0];
    assert_eq!(
        token_to_btc_conv.from.chain,
        ConversionChain::Spark,
        "From chain should be spark"
    );
    assert_ne!(
        token_to_btc_conv.from.asset.ticker, "BTC",
        "From asset should be a token"
    );
    assert!(
        token_to_btc_conv.from.asset.decimals > 0,
        "From side (token) should have decimals > 0"
    );
    assert_eq!(
        token_to_btc_conv.to.chain,
        ConversionChain::Spark,
        "To chain should be spark"
    );
    assert_eq!(
        token_to_btc_conv.to.asset.ticker, "BTC",
        "To asset should be BTC"
    );
    assert_eq!(
        token_to_btc_conv.to.asset.decimals, 0,
        "To side (BTC/sats) should report decimals=0"
    );
    // The conversion charges a fee (attributed to the token leg on mainnet).
    assert!(
        token_to_btc_conv.from.fee + token_to_btc_conv.to.fee > 0,
        "Conversion should charge a fee"
    );

    // Wait for Alice to receive the Bitcoin payment
    info!("Waiting for Alice to receive Bitcoin payment...");
    let alice_received_payment =
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 60).await?;

    assert_eq!(
        alice_received_payment.payment_type,
        PaymentType::Receive,
        "Alice should receive a payment"
    );
    assert_eq!(
        alice_received_payment.amount,
        u128::from(invoice_sats),
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
        .get(&token_id)
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
    log_test_diff(
        "test_token_conversion_success",
        &alice,
        &bob,
        &token_id,
        &snap,
    )
    .await?;
    info!("=== Test test_token_conversion_success PASSED ===");
    Ok(())
}

/// Test token conversion failure paths:
/// - Part A: Non-existent token (rejected at prepare; no pool).
/// - Part B: Insufficient funds (prepare succeeds; send rejects, balance intact).
#[test_log::test(tokio::test)]
async fn test_token_conversion_failure() -> Result<()> {
    // Part A doesn't need Alice funded; Part B checks her balance inline.
    // No on-chain sends complete here (Part A's prepare fails; Part B's send
    // is rejected on insufficient funds), so a `log_test_diff` would be ~0.
    let Some((alice, bob, token_id, _snap)) = mainnet_test_setup(false, false).await? else {
        return Ok(());
    };
    info!("=== Starting test_token_conversion_failure ===");
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Bob's Spark address: {bob_spark_address}");

    // ==========================================
    // Part A: Non-existent token rejected at prepare
    // ==========================================
    info!("--- Part A: Non-existent token failure ---");
    let non_existent_token_id =
        "btkn1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsm5wuq";
    let result = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address.clone(),
            },
            amount: Some(1_000_000),
            token_identifier: Some(non_existent_token_id.to_string()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: None,
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await;
    assert!(
        result.is_err(),
        "prepare_send_payment should fail with non-existent token"
    );
    info!("Part A: Non-existent token correctly rejected at prepare");

    // ==========================================
    // Part B: Insufficient funds rejected at send
    // ==========================================
    info!("--- Part B: Insufficient funds failure ---");
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    if alice_balance == 0 {
        warn!("Skipping Part B: Alice has 0 sats, insufficient-funds path is trivial");
        return Ok(());
    }
    info!("Alice balance: {alice_balance} sats");

    // Sample the pool's sats-per-token rate via a small prepare. Prepare doesn't
    // check the caller's balance — it just quotes against pool state — so this
    // succeeds regardless of how much Alice has. We use 2x the ToBitcoin token
    // minimum to ensure the resulting BTC-side `amount_in` clears the pool's
    // BTC asset input minimum (the ToBitcoin-side token min doesn't apply here).
    let to_btc_min_token = tobtc_min_token_input(&alice.sdk, &token_id).await?;
    let sample_token = to_btc_min_token.saturating_mul(2);
    let sample_sats_in = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address.clone(),
            },
            amount: Some(sample_token),
            token_identifier: Some(token_id.clone()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: None,
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?
        .conversion_estimate
        .as_ref()
        .expect("sample conversion estimate")
        .amount_in;

    // Scale the sample so the required sats input exceeds Alice's balance (with
    // a small headroom buffer to avoid the borderline case).
    let required_sats = u128::from(alice_balance).saturating_add(1_000);
    let scale = required_sats.div_ceil(sample_sats_in).max(2);
    let oversized_target = sample_token.saturating_mul(scale);
    info!(
        "Oversized target: {oversized_target} tokens (scale={scale}x, sample_sats_in={sample_sats_in})"
    );

    let prepare_oversize = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address,
            },
            amount: Some(oversized_target),
            token_identifier: Some(token_id),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: None,
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;
    let oversize_amount_in = prepare_oversize
        .conversion_estimate
        .as_ref()
        .expect("oversized conversion estimate")
        .amount_in;
    info!("Oversized prepare: amount_in={oversize_amount_in} sats (alice balance={alice_balance})");
    assert!(
        oversize_amount_in > u128::from(alice_balance),
        "test setup: oversized amount_in ({oversize_amount_in}) should exceed Alice's balance ({alice_balance})"
    );

    let send_result = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_oversize,
            options: None,
            idempotency_key: None,
        })
        .await;
    info!("Insufficient-funds send rejected: {}", send_result.is_err());
    assert!(
        send_result.is_err(),
        "send should reject when input exceeds balance"
    );

    // Alice's balance must be intact — a rejected send spends nothing.
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_balance_after = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    assert_eq!(
        alice_balance_after, alice_balance,
        "Alice's balance should be unchanged by a rejected send"
    );
    info!("Part B: Insufficient funds correctly rejected at send");

    info!("=== Test test_token_conversion_failure PASSED ===");
    Ok(())
}

/// Bitcoin→Token conversion targeting a Spark **invoice** (not a Spark address).
/// Exercises the `prepare/spark_invoice.rs` + `send/spark_invoice.rs` paths
/// introduced by the payments refactor (#919) — Bob creates an invoice asking
/// for USDB, Alice pays it from BTC with an explicit FromBitcoin conversion.
#[test_log::test(tokio::test)]
async fn test_token_conversion_spark_invoice_success() -> Result<()> {
    let Some((alice, bob, token_id, snap)) = mainnet_test_setup(false, true).await? else {
        return Ok(());
    };
    info!("=== Starting test_token_conversion_spark_invoice_success ===");

    let to_btc_min_token = tobtc_min_token_input(&alice.sdk, &token_id).await?;
    let invoice_token_amount = to_btc_min_token.saturating_mul(3);

    let bob_token_balance_before = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    info!("Bob token balance before: {bob_token_balance_before}");

    // Bob creates a token Spark invoice — `token_identifier` set, `amount` in
    // token base units.
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                amount: Some(invoice_token_amount),
                token_identifier: Some(token_id.clone()),
                expiry_time: None,
                description: Some("token conversion via spark invoice test".to_string()),
                sender_public_key: None,
            },
        })
        .await?
        .payment_request;
    info!("Bob's spark invoice (for {invoice_token_amount} USDB): {bob_invoice}");

    // Alice pays the invoice with an explicit Bitcoin→Token conversion: the
    // amount + token_identifier come from the invoice, conversion options
    // attach the BTC funding source.
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input { input: bob_invoice },
            amount: None,
            token_identifier: None,
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: None,
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;

    let conversion_estimate = prepare
        .conversion_estimate
        .as_ref()
        .expect("Conversion estimate should be present");
    info!(
        "Prepared: amount={:?}, amount_in={} sats, amount_out={}, fee={}",
        prepare.amount,
        conversion_estimate.amount_in,
        conversion_estimate.amount_out,
        conversion_estimate.fee
    );

    let cost = conversion_estimate
        .amount_in
        .saturating_add(conversion_estimate.fee);
    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    if u128::from(alice_balance) < cost {
        warn!(
            "Skipping test_token_conversion_spark_invoice_success: Alice balance {alice_balance} \
             sats < cost ~{cost} sats"
        );
        return Ok(());
    }

    let send_result = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    info!(
        "Alice sent spark-invoice payment via FromBitcoin conversion: status={:?}, method={:?}",
        send_result.payment.status, send_result.payment.method
    );
    assert!(
        matches!(
            send_result.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Bitcoin to Token payment should be completed or pending"
    );

    let details = send_result
        .payment
        .conversion_details
        .expect("conversion details");
    assert_eq!(
        details.conversions.len(),
        1,
        "Should have exactly one conversion (AMM)"
    );
    let conv = &details.conversions[0];
    assert_eq!(
        conv.from.chain,
        ConversionChain::Spark,
        "From chain should be spark"
    );
    assert_eq!(conv.from.asset.ticker, "BTC", "From asset should be BTC");
    assert_eq!(
        conv.from.asset.decimals, 0,
        "From side (BTC/sats) should report decimals=0"
    );
    assert_eq!(
        conv.to.chain,
        ConversionChain::Spark,
        "To chain should be spark"
    );
    assert_ne!(conv.to.asset.ticker, "BTC", "To asset should be a token");
    assert!(
        conv.to.asset.decimals > 0,
        "To side (token) should have decimals > 0"
    );
    assert!(
        conv.from.fee + conv.to.fee > 0,
        "Conversion should charge a fee"
    );

    let bob_token_balance_after =
        wait_for_token_balance_increase(&bob.sdk, &token_id, bob_token_balance_before, 120).await?;
    info!("Bob token balance after: {bob_token_balance_after} (was {bob_token_balance_before})");

    log_test_diff(
        "test_token_conversion_spark_invoice_success",
        &alice,
        &bob,
        &token_id,
        &snap,
    )
    .await?;
    info!("=== Test test_token_conversion_spark_invoice_success PASSED ===");
    Ok(())
}
