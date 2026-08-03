use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

/// Test 1: Create a Lightning HODL invoice, pay it, and claim with preimage
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_lightning_hodl_success(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_01_lightning_hodl_success ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    ensure_funded(&mut bob, 60_000).await?;

    // Bob generates a preimage/hash pair and creates a HODL invoice for Alice.
    let (preimage, payment_hash) = generate_preimage_hash_pair();
    info!("Generated payment_hash: {payment_hash}");
    let alice_identity_pubkey = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .identity_pubkey;

    let invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "HODL invoice test".to_string(),
                amount_sats: Some(10_000),
                expiry_secs: None,
                payment_hash: Some(payment_hash.clone()),
                receiver_identity_public_key: Some(alice_identity_pubkey),
            },
        })
        .await?
        .payment_request;

    info!("HODL invoice for Alice: {invoice}");

    // Bob prepares and sends. Use a short timeout so it returns while still pending.
    let prepare = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input { input: invoice },
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!("Sending 10_000 sats from Bob to Alice via Lightning HODL...");

    let send_resp = bob
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(1),
            }),
            idempotency_key: None,
        })
        .await?;

    info!("Bob send payment status: {:?}", send_resp.payment.status);
    assert!(
        matches!(send_resp.payment.status, PaymentStatus::Pending),
        "Payment should be pending (HODL invoice not yet claimed)"
    );

    // Alice syncs and verifies the pending HODL receive.
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let alice_pending = alice
        .sdk
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Pending]),
            type_filter: Some(vec![PaymentType::Receive]),
            payment_details_filter: Some(vec![PaymentDetailsFilter::Lightning {
                htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
            }]),
            ..Default::default()
        })
        .await?;

    let alice_pending_payment = alice_pending
        .payments
        .first()
        .ok_or(anyhow::anyhow!("No pending HODL payment found for Alice"))?;

    info!("Verifying Alice's pending HODL payment...");
    assert_eq!(alice_pending_payment.status, PaymentStatus::Pending);
    assert_eq!(alice_pending_payment.payment_type, PaymentType::Receive);
    assert_eq!(alice_pending_payment.amount, 10_000);
    assert!(matches!(
        &alice_pending_payment.details,
        Some(PaymentDetails::Lightning {
            htlc_details: details, ..
        })
        if details.payment_hash == payment_hash
            && details.preimage.is_none()
            && details.status == SparkHtlcStatus::WaitingForPreimage
    ));

    // Alice claims the HODL payment with the preimage.
    info!("Alice claiming HODL payment with preimage...");
    alice
        .sdk
        .claim_htlc_payment(ClaimHtlcPaymentRequest {
            preimage: preimage.clone(),
        })
        .await?;

    // Wait for Alice's payment succeeded event.
    info!("Waiting for Alice to receive payment succeeded event...");
    let received_payment =
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 60).await?;

    // Verify the completed payment details
    let alice_completed = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: received_payment.id,
        })
        .await?
        .payment;

    assert_eq!(alice_completed.status, PaymentStatus::Completed);
    assert_eq!(alice_completed.payment_type, PaymentType::Receive);
    assert_eq!(alice_completed.amount, 10_000);
    assert!(matches!(
        &alice_completed.details,
        Some(PaymentDetails::Lightning {
            htlc_details: details, ..
        })
        if details.payment_hash == payment_hash
            && details.preimage == Some(preimage.clone())
            && details.status == SparkHtlcStatus::PreimageShared
    ));

    // Wait for Bob's payment succeeded event.
    info!("Waiting for Bob's send payment to complete...");
    let bob_completed_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Send, 60).await?;

    assert_eq!(bob_completed_payment.status, PaymentStatus::Completed);
    assert_eq!(bob_completed_payment.payment_type, PaymentType::Send);
    assert_eq!(bob_completed_payment.amount, 10_000);

    // Verify Alice's balance increased.
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_final_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    assert_eq!(alice_final_balance, 10_000);

    info!("=== Test test_01_lightning_hodl_success PASSED ===");
    Ok(())
}
