use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

/// Test send/receive over lightning using external signer
#[rstest]
#[test_log::test(tokio::test)]
async fn test_external_signer_send_receive(
    #[future] alice_external_signer_sdk: Result<SdkInstance>,
    #[future] bob_external_signer_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_external_signer_send_receive ===");

    let mut alice = alice_external_signer_sdk.await?;
    let mut bob = bob_external_signer_sdk.await?;

    // Ensure Alice is funded
    ensure_funded(&mut alice, 1000).await?;

    info!("Alice funded with 1000 sats");

    // Bob creates a Spark address to receive payment
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    info!("Bob's Spark address: {}", bob_spark_address);

    // Alice prepares to send 100 sats to Bob
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            pay_amount: Some(PayAmount::Bitcoin { amount_sats: 100 }),
            conversion_options: None,
        })
        .await?;

    info!("Alice sending 100 sats to Bob...");

    // Alice sends the payment
    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    assert_eq!(send_resp.payment.payment_type, PaymentType::Send);
    info!("Payment sent, status: {:?}", send_resp.payment.status);

    // Wait for Bob to receive the payment
    info!("Waiting for Bob to receive payment...");
    let received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

    assert_eq!(received_payment.amount, 100);
    assert_eq!(received_payment.payment_type, PaymentType::Receive);
    assert_eq!(received_payment.status, PaymentStatus::Completed);

    info!("Bob received payment: {} sats", received_payment.amount);

    // Verify Bob's balance increased
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    assert!(
        bob_balance >= 100,
        "Bob should have received at least 100 sats, got {}",
        bob_balance
    );

    info!("Bob's final balance: {} sats", bob_balance);

    // Verify Alice's payment is completed
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_payment = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: send_resp.payment.id,
        })
        .await?
        .payment;

    assert_eq!(alice_payment.status, PaymentStatus::Completed);
    assert_eq!(alice_payment.payment_type, PaymentType::Send);
    assert_eq!(alice_payment.amount, 100);

    info!("=== Test test_external_signer_send_receive PASSED ===");
    Ok(())
}
