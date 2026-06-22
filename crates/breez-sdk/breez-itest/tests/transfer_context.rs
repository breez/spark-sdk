//! Integration tests for the gated bolt11 send flow: `prepare_send_payment`
//! with `include_transfer_context`, then resuming via `send_payment` with the
//! returned [`TransferContext`].
//!
//! Alice's SDK uses a [`RecordingSparkSigner`] (so a test can inspect the
//! `prepare_transfer` requests the SDK issues) and routes over Lightning
//! (`prefer_spark_over_lightning = false`) so a transfer context is produced.
//! Regtest: run with `make breez-itest`.

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;

const SEND_AMOUNT_SATS: u64 = 1_000;

/// Funds Alice, has Bob mint a bolt11 invoice, and prepares a gated send,
/// returning the prepare response (whose `transfer_context` is asserted present).
async fn prepare_gated_lightning_send(
    alice: &mut SdkInstance,
    bob: &mut SdkInstance,
) -> Result<PrepareSendPaymentResponse> {
    ensure_funded(alice, 100_000).await?;

    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "transfer-context itest".to_string(),
                amount_sats: Some(SEND_AMOUNT_SATS),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice,
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: Some(FeePolicy::FeesExcluded),
            include_transfer_context: Some(true),
        })
        .await?;

    assert!(
        prepare.transfer_context.is_some(),
        "a fees-excluded lightning send must produce a transfer context"
    );
    Ok(prepare)
}

/// prepare + resume: the `prepare_transfer` the resumed send issues to the
/// signer is byte-identical to the out-of-band authorization request, so a
/// remote signer can pre-approve exactly what the send later submits.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_resumed_send_matches_authorization_request(
    #[future] alice_recording_signer_sdk: Result<(SdkInstance, RecordedPrepareTransfers)>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    let (mut alice, recorded) = alice_recording_signer_sdk.await?;
    let mut bob = bob_sdk.await?;

    let prepare = prepare_gated_lightning_send(&mut alice, &mut bob).await?;
    let context = prepare.transfer_context.clone().expect("transfer context");

    // The request a caller would hand to its signer to authorize out of band.
    let authorization = build_transfer_authorization_request(&alice.sdk, context.clone()).await?;

    // Drop anything recorded during prepare; capture only the resumed send.
    recorded.lock().unwrap().clear();

    let send = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: None,
            idempotency_key: None,
            transfer_context: Some(context),
        })
        .await?;
    assert!(matches!(
        send.payment.status,
        PaymentStatus::Completed | PaymentStatus::Pending
    ));

    let recorded = recorded.lock().unwrap();
    assert_eq!(
        recorded.len(),
        1,
        "the resumed send should issue exactly one prepare_transfer"
    );
    assert_eq!(
        serde_json::to_value(&recorded[0])?,
        serde_json::to_value(&authorization)?,
        "the resumed send's prepare_transfer must match the authorization request",
    );
    Ok(())
}

/// prepare + resume with a pinned leaf the wallet no longer has: the resumed
/// send fails rather than silently re-selecting different leaves.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_resumed_send_with_unavailable_leaf_errors(
    #[future] alice_recording_signer_sdk: Result<(SdkInstance, RecordedPrepareTransfers)>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    let (mut alice, _recorded) = alice_recording_signer_sdk.await?;
    let mut bob = bob_sdk.await?;

    let prepare = prepare_gated_lightning_send(&mut alice, &mut bob).await?;
    let mut context = prepare.transfer_context.clone().expect("transfer context");
    assert!(!context.leaf_ids.is_empty(), "context must pin leaves");

    // Pin a leaf id the wallet does not hold (valid format, never minted).
    context.leaf_ids[0] = "00000000-0000-7000-8000-000000000000".to_string();

    let result = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: None,
            idempotency_key: None,
            transfer_context: Some(context),
        })
        .await;

    assert!(
        result.is_err(),
        "resuming a send whose pinned leaf is unavailable must error, got {result:?}"
    );
    Ok(())
}

/// prepare + resume end to end: the send completes and Bob receives the funds.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_resumed_send_succeeds(
    #[future] alice_recording_signer_sdk: Result<(SdkInstance, RecordedPrepareTransfers)>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    let (mut alice, _recorded) = alice_recording_signer_sdk.await?;
    let mut bob = bob_sdk.await?;

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_initial_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    let prepare = prepare_gated_lightning_send(&mut alice, &mut bob).await?;
    let context = prepare.transfer_context.clone().expect("transfer context");

    let send = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(10),
            }),
            idempotency_key: None,
            transfer_context: Some(context),
        })
        .await?;
    assert!(matches!(
        send.payment.status,
        PaymentStatus::Completed | PaymentStatus::Pending
    ));

    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    wait_for_balance(
        &bob.sdk,
        Some(bob_initial_balance + SEND_AMOUNT_SATS),
        None,
        20,
    )
    .await?;
    Ok(())
}

/// prepare + resume: the resumed send reserves exactly the leaves pinned in the
/// context (same ids, same order), not a freshly selected set.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_resumed_send_reserves_pinned_leaves(
    #[future] alice_recording_signer_sdk: Result<(SdkInstance, RecordedPrepareTransfers)>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    let (mut alice, recorded) = alice_recording_signer_sdk.await?;
    let mut bob = bob_sdk.await?;

    let prepare = prepare_gated_lightning_send(&mut alice, &mut bob).await?;
    let context = prepare.transfer_context.clone().expect("transfer context");

    recorded.lock().unwrap().clear();

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: None,
            idempotency_key: None,
            transfer_context: Some(context.clone()),
        })
        .await?;

    let recorded = recorded.lock().unwrap();
    let sent = recorded
        .first()
        .expect("the resumed send must issue a prepare_transfer");
    let reserved_leaf_ids: Vec<String> = sent
        .leaves
        .iter()
        .map(|leaf| leaf.node_id.id.clone())
        .collect();
    assert_eq!(
        reserved_leaf_ids, context.leaf_ids,
        "the resumed send must transfer exactly the pinned leaves, in order"
    );
    Ok(())
}
