//! Stable-balance auto-conversion integration test (mainnet, env-gated).
//!
//! Exercises the stable-balance worker that auto-converts received BTC into a
//! configured token once a threshold is exceeded. This needs real Flashnet pool
//! liquidity that regtest lacks, so it runs against **mainnet** and skips
//! automatically (logging a warning, returning `Ok`) unless the credentials
//! below are set — normal CI is unaffected.
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
//! The threshold and amounts are derived from the live pool
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
    let Some((mnemonic, api_key)) = mainnet_test_creds() else {
        warn!("Skipping mainnet test: set MAINNET_TEST_MNEMONIC and BREEZ_API_KEY to run it");
        return Ok(());
    };
    let (mut alice, mut bob) = mainnet_alice_bob(&mnemonic, &api_key, true).await?;

    // Skip rather than fail spuriously if the test account is unfunded.
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_balance_sats = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    if alice_balance_sats == 0 {
        warn!("Skipping mainnet test: test account (Alice) has 0 sats");
        return Ok(());
    }

    info!("=== Starting test_stable_balance_auto_conversion ===");
    let token_id = mainnet_test_token_id();

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
            payment_request: bob_spark_address.clone(),
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
            payment_request: bob_spark_address.clone(),
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
            payment_request: alice_invoice.clone(),
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
        conversion_details.from.as_ref().unwrap().method,
        PaymentMethod::Token,
        "From step should be a token payment"
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
    info!("=== Test test_stable_balance_auto_conversion PASSED ===");
    Ok(())
}

/// Per-receive auto-conversion: a single Spark receive whose amount is at or
/// above `pool_min` triggers `per_receive_convert` on the new payment directly,
/// without waiting for the batch-threshold path tested in
/// [`test_stable_balance_auto_conversion`].
#[test_log::test(tokio::test)]
async fn test_stable_balance_per_receive_conversion() -> Result<()> {
    let Some((mnemonic, api_key)) = mainnet_test_creds() else {
        warn!("Skipping mainnet test: set MAINNET_TEST_MNEMONIC and BREEZ_API_KEY to run it");
        return Ok(());
    };
    let (mut alice, bob) = mainnet_alice_bob(&mnemonic, &api_key, true).await?;

    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    if alice_balance == 0 {
        warn!("Skipping mainnet test: test account (Alice) has 0 sats");
        return Ok(());
    }

    info!("=== Starting test_stable_balance_per_receive_conversion ===");
    let token_id = mainnet_test_token_id();

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
            payment_request: bob_spark_address,
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

    info!("=== Test test_stable_balance_per_receive_conversion PASSED ===");
    Ok(())
}

/// Stable balance deactivation: when Bob deactivates his stable balance, the
/// `deactivation_convert` worker drains his held tokens back to BTC. `zz_`
/// prefix sorts this test last in the binary so it doesn't strand the others
/// in an inactive state (each test creates its own SDK with a fresh cache, but
/// running this one last keeps the intent obvious).
#[test_log::test(tokio::test)]
async fn test_stable_balance_zz_deactivation() -> Result<()> {
    let Some((mnemonic, api_key)) = mainnet_test_creds() else {
        warn!("Skipping mainnet test: set MAINNET_TEST_MNEMONIC and BREEZ_API_KEY to run it");
        return Ok(());
    };
    // `alice` only needed to keep the wallet pool warm / for symmetry; the test
    // operates entirely on Bob.
    let (_alice, bob) = mainnet_alice_bob(&mnemonic, &api_key, true).await?;

    info!("=== Starting test_stable_balance_zz_deactivation ===");
    let token_id = mainnet_test_token_id();

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

    info!("=== Test test_stable_balance_zz_deactivation PASSED ===");
    Ok(())
}
