use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

async fn send_htlc_alice_to_bob(
    alice: &mut SdkInstance,
    bob: &mut SdkInstance,
    payment_hash: &str,
    expiry_duration_secs: u64,
) -> Result<()> {
    // Bob exposes a Spark address
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    info!("Bob's Spark address: {}", bob_spark_address);

    // Alice prepares and sends 5 sats to Bob using a Spark HTLC
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(5),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!("Sending 5 sats from Alice to Bob via Spark HTLC...");

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::SparkAddress {
                htlc_options: Some(SparkHtlcOptions {
                    payment_hash: payment_hash.to_string(),
                    expiry_duration_secs,
                }),
            }),
            idempotency_key: None,
        })
        .await?;

    info!("Alice send payment status: {:?}", send_resp.payment.status);
    assert!(
        matches!(send_resp.payment.status, PaymentStatus::Pending),
        "Payment should be pending"
    );

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_list_payments_response = bob
        .sdk
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Pending]),
            type_filter: Some(vec![PaymentType::Receive]),
            payment_details_filter: Some(vec![PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await?;
    let bob_pending_payment = bob_list_payments_response
        .payments
        .first()
        .ok_or(anyhow::anyhow!("No pending payment found"))?;

    info!("Verifying Bob's pending payment...");

    assert_eq!(bob_pending_payment.status, PaymentStatus::Pending);
    assert_eq!(bob_pending_payment.payment_type, PaymentType::Receive);
    assert_eq!(bob_pending_payment.amount, 5);
    assert!(matches!(
    &bob_pending_payment.details,
    Some(PaymentDetails::Spark {
        htlc_details: Some(details), .. })
        if details.payment_hash == payment_hash
        && details.preimage.is_none()
        && details.status == SparkHtlcStatus::WaitingForPreimage
    ));

    let alice_list_payments_response = alice
        .sdk
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Pending]),
            type_filter: Some(vec![PaymentType::Send]),
            payment_details_filter: Some(vec![PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await?;
    let alice_pending_payment = alice_list_payments_response
        .payments
        .first()
        .ok_or(anyhow::anyhow!("No pending payment found"))?;

    info!("Verifying Alice's pending payment...");

    assert_eq!(alice_pending_payment.status, PaymentStatus::Pending);
    assert_eq!(alice_pending_payment.payment_type, PaymentType::Send);
    assert_eq!(alice_pending_payment.amount, 5);
    assert!(matches!(
    &alice_pending_payment.details,
    Some(PaymentDetails::Spark {
        htlc_details: Some(details), .. })
        if details.payment_hash == payment_hash
        && details.preimage.is_none()
        && details.status == SparkHtlcStatus::WaitingForPreimage
    ));

    Ok(())
}

/// Test 1: Send payment from Alice to Bob using Spark transfer
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_htlc_success(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_01_htlc_success ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Ensure Alice is funded (100 sats minimum for small test)
    ensure_funded(&mut alice, 100).await?;

    let (preimage, payment_hash) = generate_preimage_hash_pair();

    send_htlc_alice_to_bob(&mut alice, &mut bob, &payment_hash, 180).await?;

    info!("Claiming Bob's HTLC payment...");

    bob.sdk
        .claim_htlc_payment(ClaimHtlcPaymentRequest {
            preimage: preimage.clone(),
        })
        .await?;

    // Wait for Bob to receive payment succeeded event
    info!("Waiting for Bob to receive payment succeeded event...");
    let received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

    let bob_received_payment = bob
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: received_payment.id,
        })
        .await?
        .payment;
    assert_eq!(bob_received_payment.status, PaymentStatus::Completed);
    assert_eq!(bob_received_payment.payment_type, PaymentType::Receive);
    assert_eq!(bob_received_payment.amount, 5);
    assert!(matches!(
    &bob_received_payment.details,
    Some(PaymentDetails::Spark {
        htlc_details: Some(details), .. })
        if details.payment_hash == payment_hash
        && details.preimage == Some(preimage)
        && details.status == SparkHtlcStatus::PreimageShared
    ));

    // Verify Bob's balance increased
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    assert_eq!(bob_final_balance, 5);

    info!("=== Test test_01_htlc_success PASSED ===");
    Ok(())
}

/// Test 2: Send payment from Alice to Bob using Spark transfer and fail to claim before expiry
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_htlc_refund(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_02_htlc_refund ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Ensure Alice is funded (100 sats minimum for small test)
    ensure_funded(&mut alice, 100).await?;

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Alice balance: {} sats", alice_balance);

    let (_, payment_hash) = generate_preimage_hash_pair();

    send_htlc_alice_to_bob(&mut alice, &mut bob, &payment_hash, 5).await?;

    let alice_balance_after_send = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!(
        "Alice balance after send: {} sats",
        alice_balance_after_send
    );
    assert_eq!(alice_balance_after_send, alice_balance - 5);

    info!("Waiting for HTLC to expire...");

    // HTLC fails and is returned a little bit after the expiry
    wait_for_payment_failed_event(&mut bob.events, PaymentType::Receive, 120).await?;

    info!("Verifying Bob's failed payment...");

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let bob_payments = bob
        .sdk
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Failed]),
            type_filter: Some(vec![PaymentType::Receive]),
            payment_details_filter: Some(vec![PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![SparkHtlcStatus::Returned]),
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await?;
    let bob_payment = bob_payments
        .payments
        .first()
        .expect("No failed payment found");

    assert_eq!(bob_payment.status, PaymentStatus::Failed);
    assert_eq!(bob_payment.payment_type, PaymentType::Receive);
    assert_eq!(bob_payment.amount, 5);
    assert!(matches!(
    &bob_payment.details,
    Some(PaymentDetails::Spark {
        htlc_details: Some(details), .. })
        if details.payment_hash == payment_hash
        && details.preimage.is_none()
        && details.status == SparkHtlcStatus::Returned
    ));

    info!("Verifying Alice's failed payment...");

    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_payments = alice
        .sdk
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Failed]),
            type_filter: Some(vec![PaymentType::Send]),
            payment_details_filter: Some(vec![PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![SparkHtlcStatus::Returned]),
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await?;
    let alice_payment = alice_payments
        .payments
        .first()
        .expect("No pending payment found");

    assert_eq!(alice_payment.status, PaymentStatus::Failed);
    assert_eq!(alice_payment.payment_type, PaymentType::Send);
    assert_eq!(alice_payment.amount, 5);
    assert!(matches!(
    &alice_payment.details,
    Some(PaymentDetails::Spark {
        htlc_details: Some(details), .. })
        if details.payment_hash == payment_hash
        && details.preimage.is_none()
        && details.status == SparkHtlcStatus::Returned
    ));

    // Verify Alice's balance went back to the initial balance.
    // The HTLC refund leaf transfer may not have fully settled yet,
    // so poll until the balance is restored.
    let alice_balance_after_refund =
        wait_for_balance(&alice.sdk, Some(alice_balance), None, 30).await?;
    assert_eq!(alice_balance_after_refund, alice_balance);

    info!("=== Test test_02_htlc_refund PASSED ===");
    Ok(())
}
