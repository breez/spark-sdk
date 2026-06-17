//! Server-mode token conversion integration tests (mainnet, env-gated).
//!
//! Server mode runs without the background sync / auto-conversion / leaf
//! optimization workers; the caller drives `sync_wallet` explicitly. This
//! file covers the conversion-send paths in that mode — a prior regression
//! caused `send_payment` to hang because the received-leg spark transfer
//! wasn't observed by the server-mode SDK.
//!
//! Skips automatically (logging a warning, returning `Ok`) unless the
//! credentials below are set, so normal CI is unaffected.
//!
//! # Required environment variables
//! - `MAINNET_TEST_MNEMONIC` — mnemonic of a pre-funded mainnet test account.
//! - `BREEZ_API_KEY` — API key the mainnet SDK requires to function.
//!
//! # Optional
//! - `MAINNET_TEST_TOKEN_ID` — token to convert against; defaults to USDB.
//!
//! # Wallets & funds
//! Same env mnemonic / deterministic Bob passphrase as the client-mode mainnet
//! tests, so the on-chain identity (and the teardown) is shared. Stable balance
//! is unused (not supported in server mode).
//!
//! # Run locally
//! ```bash
//! MAINNET_TEST_MNEMONIC="..." BREEZ_API_KEY="..." \
//!   cargo test -p breez-sdk-itest --test mainnet_server_mode_conversion -- --test-threads=1 --nocapture
//! ```

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use tracing::{info, warn};

/// Bitcoin → Token conversion send in server mode. Alice (server) sends with
/// explicit `FromBitcoin` to Bob's (server) Spark address; we poll Bob's token
/// balance for the conversion to land (no background event delivery in server
/// mode, plus the conversion-leg events are suppressed by the SDK middleware).
#[test_log::test(tokio::test)]
async fn test_server_mode_bitcoin_to_token() -> Result<()> {
    let Some((mnemonic, api_key)) = mainnet_test_creds() else {
        warn!("Skipping mainnet test: set MAINNET_TEST_MNEMONIC and BREEZ_API_KEY to run it");
        return Ok(());
    };
    let (alice, bob) = mainnet_alice_bob_server_mode(&mnemonic, &api_key).await?;

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

    info!("=== Starting test_server_mode_bitcoin_to_token ===");
    let token_id = mainnet_test_token_id();
    let snap = snapshot_test_pair(&alice, &bob, &token_id).await?;

    let to_btc_min_token = tobtc_min_token_input(&alice.sdk, &token_id).await?;
    // 2x the ToBitcoin minimum gives a non-trivial conversion while keeping
    // the per-test cost low.
    let target_token_amount = to_btc_min_token.saturating_mul(2);

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_token_before = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    info!("Bob token balance before: {bob_token_before}");

    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address,
            },
            amount: Some(target_token_amount),
            token_identifier: Some(token_id.clone()),
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
    if u128::from(alice_balance) < cost {
        warn!(
            "Skipping test_server_mode_bitcoin_to_token: Alice balance {alice_balance} sats \
             < cost ~{cost} sats"
        );
        return Ok(());
    }

    let send = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    info!(
        "Alice (server mode) sent Bitcoin→Token: status={:?}, method={:?}",
        send.payment.status, send.payment.method
    );
    assert!(
        matches!(
            send.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Bitcoin→Token send should not hang in server mode"
    );

    let details = send
        .payment
        .conversion_details
        .expect("Conversion-send payment should carry conversion_details");
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
    assert_ne!(conv.to.asset.ticker, "BTC", "To asset should be a token");
    assert!(
        conv.from.fee + conv.to.fee > 0,
        "Conversion should charge a fee"
    );

    // Server-mode Bob has no background sync — poll via sync_wallet + get_info.
    let bob_token_after =
        wait_for_token_balance_increase(&bob.sdk, &token_id, bob_token_before, 120).await?;
    info!("Bob token balance after: {bob_token_after} (was {bob_token_before})");

    log_test_diff(
        "test_server_mode_bitcoin_to_token",
        &alice,
        &bob,
        &token_id,
        &snap,
    )
    .await?;
    info!("=== Test test_server_mode_bitcoin_to_token PASSED ===");
    Ok(())
}

/// Token → Bitcoin conversion send in server mode. Bob (server) pays Alice's
/// (server) Bolt11 invoice via explicit `ToBitcoin`; we poll Alice's sat
/// balance for the receipt.
///
/// Requires Bob to already hold USDB. Tests run alphabetically within a binary,
/// so `test_server_mode_bitcoin_to_token` runs first and seeds him. If somehow
/// Bob is empty (e.g., this test runs in isolation against a freshly-drained
/// wallet), the test skips with a warning.
#[test_log::test(tokio::test)]
async fn test_server_mode_token_to_bitcoin() -> Result<()> {
    let Some((mnemonic, api_key)) = mainnet_test_creds() else {
        warn!("Skipping mainnet test: set MAINNET_TEST_MNEMONIC and BREEZ_API_KEY to run it");
        return Ok(());
    };
    let (alice, bob) = mainnet_alice_bob_server_mode(&mnemonic, &api_key).await?;

    info!("=== Starting test_server_mode_token_to_bitcoin ===");
    let token_id = mainnet_test_token_id();
    let snap = snapshot_test_pair(&alice, &bob, &token_id).await?;

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_token_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    if bob_token_balance == 0 {
        warn!(
            "Skipping test_server_mode_token_to_bitcoin: Bob has no USDB to send \
             (test_server_mode_bitcoin_to_token must run first to seed him, \
             or a prior run must have left tokens)"
        );
        return Ok(());
    }
    info!("Bob's token balance: {bob_token_balance}");

    // Size Alice's invoice via the pool's forward estimate of the minimum
    // token input, same approach as the client-mode token_conversion Part B.
    let to_btc_min_token = tobtc_min_token_input(&bob.sdk, &token_id).await?;
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let invoice_sats = u64::try_from(
        estimate_tobtc_sats_out(&bob.sdk, &bob_spark_address, &token_id, to_btc_min_token).await?,
    )?;
    info!("Invoice sats sized at {invoice_sats} (from pool estimate)");

    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_sats_before = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    let alice_invoice = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Server-mode Token→Bitcoin test".to_string(),
                amount_sats: Some(invoice_sats),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;
    info!("Alice's Lightning invoice: {alice_invoice}");

    let prepare = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: alice_invoice,
            },
            amount: None,
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
    let estimate = prepare
        .conversion_estimate
        .as_ref()
        .expect("conversion estimate");
    info!(
        "Bob prepared Token→Bitcoin: amount={:?}, token_in={}, sats_out={}, fee={}",
        prepare.amount, estimate.amount_in, estimate.amount_out, estimate.fee
    );

    let send = bob
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    info!(
        "Bob (server mode) sent Token→Bitcoin: status={:?}, method={:?}",
        send.payment.status, send.payment.method
    );
    assert!(
        matches!(
            send.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Token→Bitcoin send should not hang in server mode"
    );

    let details = send
        .payment
        .conversion_details
        .expect("Conversion-send payment should carry conversion_details");
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
    assert_eq!(
        conv.to.chain,
        ConversionChain::Spark,
        "To chain should be spark"
    );
    assert_eq!(conv.to.asset.ticker, "BTC", "To asset should be BTC");
    assert!(
        conv.from.fee + conv.to.fee > 0,
        "Conversion should charge a fee"
    );

    // Server-mode Alice has no background sync — poll for the inbound sats.
    let alice_sats_after = wait_for_balance(
        &alice.sdk,
        Some(alice_sats_before.saturating_add(1)),
        None,
        120,
    )
    .await?;
    info!("Alice sats balance: {alice_sats_after} (was {alice_sats_before})");

    log_test_diff(
        "test_server_mode_token_to_bitcoin",
        &alice,
        &bob,
        &token_id,
        &snap,
    )
    .await?;
    info!("=== Test test_server_mode_token_to_bitcoin PASSED ===");
    Ok(())
}
