//! Mainnet teardown (env-gated).
//!
//! Drains the deterministic "Bob" wallet back to the test account ("Alice"),
//! converting any tokens to **sats** via a send-all-with-conversion so Alice
//! stays sats-denominated and fundable for future runs. This doubles as a real
//! test of the send-all token→sats conversion path.
//!
//! Run **last** in CI (as an `if: always()` step) so funds are recovered even
//! when the conversion tests fail. Because Bob is deterministic, funds left by a
//! crashed run are recovered by the next teardown. Skips automatically (logging a
//! warning, returning `Ok`) unless the credentials below are set.
//!
//! # Required environment variables
//! - `MAINNET_TEST_MNEMONIC` — mnemonic of the test account. Also the primary gate.
//! - `BREEZ_API_KEY` — API key the mainnet SDK requires to function.
//!
//! # Optional
//! - `MAINNET_TEST_TOKEN_ID` — token to drain/convert; defaults to USDB.
//!
//! # Run locally
//! ```bash
//! MAINNET_TEST_MNEMONIC="..." BREEZ_API_KEY="..." \
//!   cargo test -p breez-sdk-itest --test mainnet_teardown -- --test-threads=1 --nocapture
//! ```

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use tracing::{info, warn};

/// Generous slippage for the teardown conversion — returning liquid sats matters
/// more than price here.
const TEARDOWN_MAX_SLIPPAGE_BPS: u32 = 500;

/// Drain Bob back to Alice. If Bob holds tokens, convert them (and any sats) to
/// sats via a send-all-with-conversion; otherwise return any residual sats.
#[test_log::test(tokio::test)]
async fn test_mainnet_teardown_drain_bob_to_alice() -> Result<()> {
    // Build Bob with the stable token active so send-all-with-conversion engages
    // the path that includes his existing sat balance in the same atomic drain.
    // Teardown doesn't need Alice funded — she's the recipient.
    let Some((mut alice, bob, token_id, snap)) = mainnet_test_setup(true, false).await? else {
        return Ok(());
    };

    // Reclaim anything stuck mid-conversion first.
    if let Err(e) = bob.sdk.refund_pending_conversions().await {
        warn!("refund_pending_conversions failed (continuing): {e:#}");
    }

    // If Bob inherits sats above the stable-balance auto-conversion threshold from
    // a prior test in this binary, his worker fires a FromBitcoin auto-conversion
    // immediately on init. Wait for that to settle before our own ToBitcoin drain
    // — otherwise the auto-converted tokens land *after* our drain completes and
    // Bob ends up with fresh tokens instead of empty. (The SDK's payments lock
    // serializes the two, but the worker wins the race to acquire it first.)
    let auto_threshold_sats: u64 = bob
        .sdk
        .fetch_conversion_limits(FetchConversionLimitsRequest {
            conversion_type: ConversionType::FromBitcoin,
            token_identifier: Some(token_id.clone()),
        })
        .await?
        .min_from_amount
        .and_then(|m| u64::try_from(m).ok())
        .unwrap_or(u64::MAX);

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let pre_info = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let pre_tokens = pre_info
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    if pre_info.balance_sats >= auto_threshold_sats {
        info!(
            "Teardown: Bob has {} sats ≥ auto-conversion threshold {auto_threshold_sats}; \
             waiting for in-flight auto-conversion to settle...",
            pre_info.balance_sats
        );
        if let Err(e) = wait_for_token_balance_increase(&bob.sdk, &token_id, pre_tokens, 180).await
        {
            warn!("Teardown: auto-conversion didn't complete in 180s ({e:#}); proceeding anyway");
        }
        bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    }

    let bob_info = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let bob_token_balance = bob_info
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    let bob_sats = bob_info.balance_sats;
    info!("Teardown: Bob holds {bob_token_balance} units of {token_id}, {bob_sats} sats");

    let alice_addr = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    if bob_token_balance > 0 {
        // Send-all-with-conversion: converts ALL of Bob's tokens to sats and
        // includes his existing sat balance, sending everything to Alice.
        info!(
            "Teardown: draining {bob_token_balance} tokens (+{bob_sats} sats) to Alice via token→sats conversion"
        );
        let prepare = bob
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: PaymentRequest::Input {
                    input: alice_addr.clone(),
                },
                amount: Some(bob_token_balance),
                token_identifier: Some(token_id.clone()),
                conversion_options: Some(ConversionOptions {
                    conversion_type: ConversionType::ToBitcoin {
                        from_token_identifier: token_id.clone(),
                    },
                    max_slippage_bps: Some(TEARDOWN_MAX_SLIPPAGE_BPS),
                    completion_timeout_secs: None,
                }),
                fee_policy: Some(FeePolicy::FeesIncluded),
            })
            .await?;

        // Also a test of the send-all token→sats conversion.
        let estimate = prepare
            .conversion_estimate
            .as_ref()
            .expect("Send-all conversion should produce an estimate");
        assert!(
            matches!(
                estimate.options.conversion_type,
                ConversionType::ToBitcoin { .. }
            ),
            "Drain conversion should be Token→Bitcoin"
        );

        let resp = bob
            .sdk
            .send_payment(SendPaymentRequest {
                prepare_response: prepare,
                options: None,
                idempotency_key: None,
            })
            .await?;
        let details = resp
            .payment
            .conversion_details
            .expect("Drain should include conversion details");
        assert_eq!(
            details.conversions.len(),
            1,
            "Should have exactly one conversion (AMM)"
        );
        assert_ne!(
            details.conversions[0].from.asset.ticker, "BTC",
            "From asset should be a token"
        );

        // Confirm Alice receives the converted sats.
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 120).await?;
    }

    // Drain any remaining sats. Unconditional: covers the tokenless case and
    // any residual after the token drain. Runs before the final assertions so a
    // partially-drained Bob is still cleaned up.
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let remaining_sats = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    if remaining_sats > 0 {
        info!("Teardown: draining {remaining_sats} sats to Alice");
        let prepare = bob
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: PaymentRequest::Input {
                    input: alice_addr.clone(),
                },
                amount: Some(u128::from(remaining_sats)),
                token_identifier: None,
                conversion_options: None,
                fee_policy: Some(FeePolicy::FeesIncluded),
            })
            .await?;
        bob.sdk
            .send_payment(SendPaymentRequest {
                prepare_response: prepare,
                options: None,
                idempotency_key: None,
            })
            .await?;
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 60).await?;
    }

    // Final assertion: Bob's tokens are fully drained. Deferred to the end so
    // the sat drain above still runs even on partial token cleanup.
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let final_tokens = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    assert_eq!(final_tokens, 0, "Bob's tokens should be fully drained");

    log_test_diff(
        "test_mainnet_teardown_drain_bob_to_alice",
        &alice,
        &bob,
        &token_id,
        &snap,
    )
    .await?;
    info!("Teardown complete: Bob drained to Alice");
    Ok(())
}
