//! Stable-balance integration tests (mainnet, env-gated).
//!
//! Exercises the stable-balance worker + its hooks into the payment flow.
//! Needs real Flashnet pool liquidity that regtest lacks, so runs against
//! **mainnet** and skips automatically (logging a warning, returning `Ok`)
//! unless the credentials below are set — normal CI is unaffected.
//!
//! # Tests in this binary
//! - `test_stable_balance_auto_conversion` — cumulative-threshold path: two
//!   sub-threshold sat receives that together cross the threshold trigger the
//!   batch auto-convert of Bob's full balance.
//! - `test_stable_balance_per_receive_conversion` — per-receive path: a single
//!   sat receive above the pool minimum fires `per_receive_convert` directly,
//!   bypassing the batch threshold.
//! - `test_stable_balance_send_lightning_address` — send-side **auto-fill**
//!   path: Bob pays Alice's LN address denominated in sats with no explicit
//!   `ConversionOptions`; stable balance auto-populates a Token→BTC conversion
//!   from his active token because his sat balance can't cover the payment.
//! - `test_stable_balance_zz_deactivation` — deactivation: unsetting Bob's
//!   active stable token kicks `deactivation_convert` which drains his held
//!   tokens back to sats. `zz_` prefix sorts it last in the binary.
//!
//! # Required environment variables
//! - `MAINNET_TEST_MNEMONIC` — mnemonic of a pre-funded mainnet test account
//!   ("Alice", the funder/sender). Also the primary gate.
//! - `BREEZ_API_KEY` — API key the mainnet SDK requires to function.
//!
//! # Optional
//! - `MAINNET_TEST_TOKEN_ID` — stable token; defaults to USDB.
//!
//! # Wallets & funds
//! Alice is the env account (the funder, keeps her funds). "Bob" is the
//! stable-balance receiver, derived deterministically from Alice's mnemonic
//! (fixed passphrase). There is no per-test sweep — the dedicated
//! `mainnet_teardown` test drains Bob back to Alice (converting tokens to sats)
//! once per run, and recovers anything a failed run left behind on the next run.
//! Thresholds and amounts are derived from the live pool
//! (`fetch_conversion_limits`); slippage inherits the SDK default.
//!
//! # Run locally
//! ```bash
//! MAINNET_TEST_MNEMONIC="..." BREEZ_API_KEY="..." \
//!   cargo test -p breez-sdk-itest --test mainnet_stable_balance -- --test-threads=1 --nocapture
//! ```

use anyhow::{Result, anyhow};
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use tracing::{info, warn};

/// Test stable balance auto-conversion feature. Sends two payments each just
/// over half the threshold: each is individually below the per-receive minimum
/// (so neither converts on receipt), but together they cross the threshold and
/// trigger the batch auto-conversion of Bob's full balance.
/// 1. Alice sends the first sub-threshold payment - verify no conversion
/// 2. Alice sends the second - the accumulated balance triggers auto-conversion
/// 3. Bob spends his stable balance: pays Alice's invoice with sats auto-converted
///    from his USDB (no explicit conversion options)
#[test_log::test(tokio::test)]
async fn test_stable_balance_auto_conversion() -> Result<()> {
    let Some((mut alice, mut bob, token_id, snap)) = mainnet_test_setup(true, true).await? else {
        return Ok(());
    };
    info!("=== Starting test_stable_balance_auto_conversion ===");

    // The effective auto-conversion threshold is the pool's minimum
    // FromBitcoin amount (since the config leaves threshold_sats=None).
    let pool_min_sats = alice
        .sdk
        .fetch_conversion_limits(FetchConversionLimitsRequest {
            conversion_type: ConversionType::FromBitcoin,
            token_identifier: Some(token_id.clone()),
        })
        .await?
        .min_from_amount
        .ok_or_else(|| anyhow!("FromBitcoin min_from_amount missing"))?;
    let pool_min_sats = u64::try_from(pool_min_sats)?;

    if pool_min_sats < 3 {
        warn!("Skipping: pool minimum {pool_min_sats} too small to split");
        return Ok(());
    }

    // Two payments, each below the per-receive minimum (= the threshold when
    // threshold_sats is None), so neither is converted on receipt. Together
    // their sum is ~1.8x the threshold, which gives the auto-convert's dust
    // check (`produces_token_dust`) comfortable headroom over the ToBitcoin
    // token minimum — sizing too close to 1x can produce tokens just over the
    // dust floor and trip slippage/rate noise.
    let half_payment = pool_min_sats.saturating_mul(9) / 10;

    // Skip rather than fail if the funder can't cover the sends.
    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    let needed = half_payment.saturating_mul(2).saturating_add(pool_min_sats); // headroom for fees
    if alice_balance < needed {
        warn!(
            "Skipping test_stable_balance_auto_conversion: Alice balance {alice_balance} \
                     sats < needed ~{needed} sats"
        );
        return Ok(());
    }

    // ==========================================
    // Part 1: First sub-threshold payment - no conversion
    // ==========================================
    info!("--- Part 1: Send {half_payment} sats (below per-receive min {pool_min_sats}) ---");

    // Bob (stable-balance receiver) creates a Spark address
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    // Record Bob's token balance before receiving
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

    // Alice sends the first sub-threshold payment to Bob
    let prepare_small = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address.clone(),
            },
            amount: Some(u128::from(half_payment)),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_small,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Wait for Alice to send the payment
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;

    // Wait for Bob to receive the payment
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

    // Wait for synced event
    wait_for_synced_event(&mut bob.events, 5).await?;

    let bob_info_after_small = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;

    info!(
        "Bob after receiving {} sats: {} sats, {} tokens",
        half_payment,
        bob_info_after_small.balance_sats,
        bob_info_after_small
            .token_balances
            .get(&token_id)
            .map(|b| b.balance)
            .unwrap_or(0)
    );

    // Verify token balance didn't change (payment below per-receive min)
    let bob_token_balance_after_small = bob_info_after_small
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);

    assert_eq!(
        bob_token_balance_after_small, bob_token_balance_before,
        "Bob's token balance should not change for a sub-minimum payment"
    );

    info!("Part 1 complete: No conversion (payment below per-receive min)");

    // ==========================================
    // Part 2: Second sub-threshold payment crosses the threshold - auto-conversion triggers
    // ==========================================
    info!("--- Part 2: Send another {half_payment} sats to cross the threshold ---");

    let prepare_large = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address.clone(),
            },
            amount: Some(u128::from(half_payment)),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_large,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Wait for Alice to send the payment
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;

    // Poll Bob's token balance for the auto-conversion to land. The SDK's
    // `token_conversion::middleware` suppresses the individual Send-Spark/
    // Receive-Token legs of an auto-conversion from external event listeners
    // (they're treated as child payments of one user-visible conversion), so
    // there's no PaymentSucceeded event to wait for here — the balance change
    // is the user-visible signal.
    let bob_token_balance_after_large =
        wait_for_token_balance_increase(&bob.sdk, &token_id, bob_token_balance_after_small, 120)
            .await?;
    let bob_info_after_large = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;

    info!(
        "Bob after receiving {} more sats: {} sats, {} tokens (was {} tokens)",
        half_payment,
        bob_info_after_large.balance_sats,
        bob_token_balance_after_large,
        bob_token_balance_after_small
    );

    // Verify auto-conversion happened (sats converted away, tokens increased)
    assert_eq!(
        bob_info_after_large.balance_sats, 0,
        "Bob's sats should be converted (got {} sats)",
        bob_info_after_large.balance_sats
    );

    assert!(
        bob_token_balance_after_large > bob_token_balance_after_small,
        "Bob's token balance should increase after auto-conversion ({} > {})",
        bob_token_balance_after_large,
        bob_token_balance_after_small
    );

    info!("Part 2 complete: Auto-conversion triggered successfully");

    // ==========================================
    // Part 3: Bob spends his stable balance. He pays Alice's Lightning
    // invoice with no explicit conversion options — since his sats are 0,
    // the stable-balance feature auto-fills a Token→BTC conversion.
    // ==========================================
    info!("--- Part 3: Bob pays Alice's invoice from his USDB stable balance ---");

    // Size the invoice off the sats produced by converting the minimum
    // token input (the pool exposes no output minimum). Bob holds far more
    // USDB than this, so the auto-fill can comfortably cover it.
    let to_btc_min_token = tobtc_min_token_input(&bob.sdk, &token_id).await?;
    let invoice_sats = u64::try_from(
        estimate_tobtc_sats_out(&bob.sdk, &bob_spark_address, &token_id, to_btc_min_token).await?,
    )?;
    info!("Part 3 invoice sized at {invoice_sats} sats (from pool estimate)");

    // Alice creates a Lightning invoice for Bob to pay
    let alice_invoice = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Stable balance spend test".to_string(),
                amount_sats: Some(invoice_sats),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    // Bob prepares WITHOUT explicit conversion options. The stable-balance
    // feature auto-populates a Token→BTC conversion because his sats (0)
    // are insufficient to cover the payment.
    let prepare_spend = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: alice_invoice.clone(),
            },
            amount: None, // from invoice
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let conversion_estimate = prepare_spend
        .conversion_estimate
        .as_ref()
        .expect("Stable balance should auto-populate a conversion estimate");
    info!(
        "Bob prepared invoice payment via auto-conversion: amount={:?}, token amount={}, fee={}",
        prepare_spend.amount, conversion_estimate.amount_out, conversion_estimate.fee
    );
    assert!(
        matches!(
            conversion_estimate.options.conversion_type,
            ConversionType::ToBitcoin { .. }
        ),
        "Auto-filled conversion should be Token→Bitcoin"
    );

    let send_result = bob
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_spend,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!(
        "Bob sent invoice payment via stable balance: status={:?}",
        send_result.payment.status
    );

    // The sent payment should carry a Token→Bitcoin conversion.
    let conversion_details = send_result
        .payment
        .conversion_details
        .expect("Spend should include conversion details");
    assert_eq!(
        conversion_details.conversions.len(),
        1,
        "Should have exactly one conversion (AMM)"
    );
    assert_ne!(
        conversion_details.conversions[0].from.asset.ticker, "BTC",
        "From asset should be a token"
    );
    assert!(
        conversion_details.conversions[0].from.asset.decimals > 0,
        "From side (token) should have decimals > 0"
    );

    // Wait for Bob to send and Alice to receive
    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Send, 90).await?;
    let alice_received =
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 90).await?;
    assert_eq!(
        alice_received.amount,
        u128::from(invoice_sats),
        "Alice should receive the invoice amount in sats"
    );

    // Verify Bob's token balance decreased
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final_token_balance = bob
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
        "Bob token balance after spend: {} (was {})",
        bob_final_token_balance, bob_token_balance_after_large
    );
    assert!(
        bob_final_token_balance < bob_token_balance_after_large,
        "Bob's token balance should decrease after spending via conversion"
    );

    info!("Part 3 complete: Stable balance spend (auto Token→BTC) successful");
    log_test_diff(
        "test_stable_balance_auto_conversion",
        &alice,
        &bob,
        &token_id,
        &snap,
    )
    .await?;
    info!("=== Test test_stable_balance_auto_conversion PASSED ===");
    Ok(())
}

/// Per-receive auto-conversion: a single Spark receive whose amount is at or
/// above `pool_min` triggers `per_receive_convert` on the new payment directly,
/// without waiting for the batch-threshold path tested in
/// [`test_stable_balance_auto_conversion`].
#[test_log::test(tokio::test)]
async fn test_stable_balance_per_receive_conversion() -> Result<()> {
    let Some((mut alice, bob, token_id, snap)) = mainnet_test_setup(true, true).await? else {
        return Ok(());
    };
    info!("=== Starting test_stable_balance_per_receive_conversion ===");

    let pool_min_sats = alice
        .sdk
        .fetch_conversion_limits(FetchConversionLimitsRequest {
            conversion_type: ConversionType::FromBitcoin,
            token_identifier: Some(token_id.clone()),
        })
        .await?
        .min_from_amount
        .ok_or_else(|| anyhow!("FromBitcoin min_from_amount missing"))?;
    let pool_min_sats = u64::try_from(pool_min_sats)?;

    // A single payment 10% above the per-receive minimum fires per_receive_convert
    // on the new payment (a different code path from the batch auto-convert).
    let payment_sats = pool_min_sats.saturating_mul(11) / 10;
    let needed = payment_sats.saturating_add(pool_min_sats); // fee headroom
    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    if alice_balance < needed {
        warn!(
            "Skipping test_stable_balance_per_receive_conversion: Alice balance {alice_balance} \
             sats < needed ~{needed} sats"
        );
        return Ok(());
    }

    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

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

    info!("--- Sending {payment_sats} sats (above per-receive min {pool_min_sats}) ---");
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address,
            },
            amount: Some(u128::from(payment_sats)),
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
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;

    // Same balance-poll pattern as Part 2: the conversion legs are suppressed
    // from external listeners by the token_conversion middleware.
    let bob_token_balance_after =
        wait_for_token_balance_increase(&bob.sdk, &token_id, bob_token_balance_before, 120).await?;
    info!(
        "Bob token balance after per-receive conversion: {bob_token_balance_after} (was {bob_token_balance_before})"
    );

    log_test_diff(
        "test_stable_balance_per_receive_conversion",
        &alice,
        &bob,
        &token_id,
        &snap,
    )
    .await?;
    info!("=== Test test_stable_balance_per_receive_conversion PASSED ===");
    Ok(())
}

/// Stable-balance send-side **auto-fill** of conversion options, via a
/// Lightning Address destination.
///
/// Bob holds USDB with stable balance active. He pays Alice's LN address with
/// `prepare_lnurl_pay(amount=sats, conversion_options=None, token_identifier=None)`
/// — i.e. denominating the payment in sats, no caller-supplied conversion. The
/// SDK's stable-balance hook in
/// [`payments/conversion.rs`](`get_conversion_options_for_payment`) detects
/// Bob doesn't have enough sats to cover the LN payment and auto-populates a
/// Token→BTC conversion from his active stable token.
///
/// Exercises the LNURL-pay code path together with the stable-balance-driven
/// send-side auto-fill (distinct from the receive-side auto-conversion the
/// rest of this file covers).
///
/// Alice registers `mainnet-itest-alice@breez.tips` on first run; later runs
/// recover from the lnurl server (tied to the env mnemonic's pubkey). If the
/// username has been claimed by a different pubkey, the test logs and skips.
#[test_log::test(tokio::test)]
async fn test_stable_balance_send_lightning_address() -> Result<()> {
    // Bob built with stable balance ON so the LN-pay below picks up his active
    // token automatically when his sat balance can't cover the payment.
    let Some((mut alice, bob, token_id, snap)) = mainnet_test_setup(true, true).await? else {
        return Ok(());
    };
    info!("=== Starting test_stable_balance_send_lightning_address ===");

    // Step 1: ensure Alice has a Lightning Address on the mainnet lnurl server.
    // `get_lightning_address` recovers from the server when the local cache is
    // empty (the temp-dir storage is fresh per run); the registration only
    // hits the wire on the very first run for this mnemonic.
    let alice_la = match alice.sdk.get_lightning_address().await? {
        Some(info) => info.lightning_address,
        None => match alice
            .sdk
            .register_lightning_address(RegisterLightningAddressRequest {
                username: "mainnet-itest-alice".to_string(),
                description: Some("Mainnet itest Alice".to_string()),
            })
            .await
        {
            Ok(info) => info.lightning_address,
            Err(e) => {
                warn!(
                    "Skipping: failed to register Alice's lightning address \
                     (likely claimed by another pubkey): {e:#}"
                );
                return Ok(());
            }
        },
    };
    info!("Alice's lightning address: {alice_la}");

    // Step 2: ensure Bob has enough tokens for the auto-fill to source from.
    let to_btc_min_token = tobtc_min_token_input(&alice.sdk, &token_id).await?;
    let min_required_tokens = to_btc_min_token.saturating_mul(2);
    let seed_amount = to_btc_min_token.saturating_mul(3);
    if !ensure_bob_has_tokens(&alice, &bob, &token_id, min_required_tokens, seed_amount).await? {
        return Ok(());
    }

    // Step 3: Bob → Alice's lightning address, denominated in sats, with **no
    // explicit conversion options**. Stable balance should auto-fill ToBitcoin
    // from Bob's active token because his sat balance can't cover the payment.
    //
    // Size the sats amount off the pool quote: convert `to_btc_min_token` token
    // units to sats so we know it clears the pool's swap minimums on the way out.
    let bob_spark_for_quote = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let send_sats = u64::try_from(
        estimate_tobtc_sats_out(&bob.sdk, &bob_spark_for_quote, &token_id, to_btc_min_token)
            .await?,
    )?;
    info!("LN-address payment sized at {send_sats} sats (from pool estimate)");

    let alice_sats_before = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    clear_event_receiver(&mut alice.events).await;

    let parsed = bob.sdk.parse(&alice_la).await?;
    let InputType::LightningAddress(la_details) = parsed else {
        anyhow::bail!("Expected LightningAddress input, got {parsed:?}");
    };

    let prepare = bob
        .sdk
        .prepare_lnurl_pay(PrepareLnurlPayRequest {
            amount: u128::from(send_sats),
            pay_request: la_details.pay_request,
            comment: Some("mainnet itest LN-address auto-fill conversion".to_string()),
            validate_success_action_url: None,
            // Auto-fill path: no explicit token_identifier / conversion_options.
            // Stable balance config drives the source token + ToBitcoin conversion.
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let estimate = prepare.conversion_estimate.as_ref().expect(
        "conversion estimate should be auto-filled from stable balance when Bob has \
             insufficient sats",
    );
    info!(
        "Prepared LNURL-pay (stable-balance auto-fill): amount_sats={}, token_in={}, sats_out={}, conv_fee={}, ln_fee={}",
        prepare.amount_sats,
        estimate.amount_in,
        estimate.amount_out,
        estimate.fee,
        prepare.fee_sats
    );
    assert!(
        matches!(
            estimate.options.conversion_type,
            ConversionType::ToBitcoin { .. }
        ),
        "Auto-filled conversion type should be ToBitcoin"
    );

    let resp = bob
        .sdk
        .lnurl_pay(LnurlPayRequest {
            prepare_response: prepare,
            idempotency_key: None,
        })
        .await?;
    info!(
        "Bob paid Alice's LN address (auto-filled Token→BTC): status={:?}, method={:?}",
        resp.payment.status, resp.payment.method
    );
    assert!(
        matches!(
            resp.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "LNURL-pay with auto-filled conversion should be completed or pending"
    );

    let details = resp
        .payment
        .conversion_details
        .expect("conversion details on LNURL-pay Token→BTC");
    assert_eq!(
        details.conversions.len(),
        1,
        "Should have exactly one conversion (AMM)"
    );
    let conv = &details.conversions[0];
    assert_ne!(
        conv.from.asset.ticker, "BTC",
        "From asset should be a token"
    );
    assert!(
        conv.from.asset.decimals > 0,
        "From side (token) should have decimals > 0"
    );
    assert_eq!(
        conv.to.chain,
        ConversionChain::Spark,
        "To chain should be spark (swap pool's sats delivery; the outer \
         payment's `method` is Lightning)"
    );
    assert_eq!(conv.to.asset.ticker, "BTC", "To asset should be BTC");
    assert_eq!(
        conv.to.asset.decimals, 0,
        "To side (BTC/sats) should report decimals=0"
    );
    assert_eq!(
        resp.payment.method,
        PaymentMethod::Lightning,
        "Outer payment method should be Lightning"
    );
    assert!(
        conv.from.fee + conv.to.fee > 0,
        "Conversion should charge a fee"
    );

    info!("Waiting for Alice to receive lightning payment...");
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 120).await?;

    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_sats_after = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice balance: {alice_sats_after} sats (was {alice_sats_before})");
    assert!(
        alice_sats_after > alice_sats_before,
        "Alice's sat balance should increase after receiving Token→BTC LNURL-pay"
    );

    log_test_diff(
        "test_stable_balance_send_lightning_address",
        &alice,
        &bob,
        &token_id,
        &snap,
    )
    .await?;
    info!("=== Test test_stable_balance_send_lightning_address PASSED ===");
    Ok(())
}

/// Stable balance deactivation: when Bob deactivates his stable balance, the
/// `deactivation_convert` worker drains his held tokens back to BTC. `zz_`
/// prefix sorts this test last in the binary so it doesn't strand the others
/// in an inactive state (each test creates its own SDK with a fresh cache, but
/// running this one last keeps the intent obvious).
#[test_log::test(tokio::test)]
async fn test_stable_balance_zz_deactivation() -> Result<()> {
    // Operates entirely on Bob — Alice's balance check is skipped.
    let Some((alice, bob, token_id, snap)) = mainnet_test_setup(true, false).await? else {
        return Ok(());
    };
    info!("=== Starting test_stable_balance_zz_deactivation ===");

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_info_before = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let bob_tokens_before = bob_info_before
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    let bob_sats_before = bob_info_before.balance_sats;
    info!("Bob before deactivation: {bob_tokens_before} tokens, {bob_sats_before} sats");

    if bob_tokens_before == 0 {
        warn!("Skipping: Bob has no USDB to convert back; deactivation has nothing to do");
        return Ok(());
    }

    info!("Deactivating Bob's stable balance...");
    bob.sdk
        .update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: None,
            stable_balance_active_label: Some(StableBalanceActiveLabel::Unset),
        })
        .await?;

    // Wait for the deactivation_convert worker to drain Bob's tokens back to
    // sats. The user-visible signal is Bob's sat balance rising above its prior
    // value (the conversion legs are suppressed from external listeners).
    let bob_sats_after =
        wait_for_balance(&bob.sdk, Some(bob_sats_before.saturating_add(1)), None, 120).await?;
    info!("Bob sats after deactivation: {bob_sats_after} (was {bob_sats_before})");

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_tokens_after = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    info!("Bob tokens after deactivation: {bob_tokens_after}");
    assert_eq!(
        bob_tokens_after, 0,
        "Bob's USDB should be fully converted back to sats after deactivation"
    );

    log_test_diff(
        "test_stable_balance_zz_deactivation",
        &alice,
        &bob,
        &token_id,
        &snap,
    )
    .await?;
    info!("=== Test test_stable_balance_zz_deactivation PASSED ===");
    Ok(())
}
