use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tokio_with_wasm::alias as tokio;
use tracing::{debug, info};

// ---------------------
// Tests
// ---------------------

#[rstest]
#[test_log::test(tokio::test)]
async fn test_breez_sdk_deposit_claim() -> Result<()> {
    info!("=== Starting test_breez_sdk_deposit_claim ===");

    // Create SDK (alice)
    let data_dir = tempdir::TempDir::new("breez-sdk-deposit")?;
    let sdk = build_sdk(data_dir.path().to_string_lossy().to_string(), [1u8; 32]).await?;

    // Get a deposit address and fund it automatically
    let (deposit_address, _txid) = receive_and_fund(&sdk, 10_000).await?;

    info!("Successfully funded deposit address: {}", deposit_address);

    // Balance should have increased (auto-claimed by SDK background sync)
    let info_res = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;

    debug!("Wallet balance after claim: {} sats", info_res.balance_sats);
    assert!(
        info_res.balance_sats > 0,
        "Balance should increase after deposit claim"
    );

    info!("=== Test test_breez_sdk_deposit_claim PASSED ===");
    Ok(())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_breez_sdk_send_payment_prefer_spark() -> Result<()> {
    info!("=== Starting test_breez_sdk_send_payment_prefer_spark ===");

    // Create SDKs for Alice and Bob
    let alice_dir = tempdir::TempDir::new("breez-sdk-alice")?;
    let bob_dir = tempdir::TempDir::new("breez-sdk-bob")?;

    let alice = build_sdk(alice_dir.path().to_string_lossy().to_string(), [2u8; 32]).await?;
    let bob = build_sdk(bob_dir.path().to_string_lossy().to_string(), [3u8; 32]).await?;

    // Fund Alice automatically
    info!("Funding Alice's wallet...");
    let (_alice_deposit_addr, _alice_txid) = receive_and_fund(&alice, 20_000).await?;

    let alice_balance = alice
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Alice balance after funding: {} sats", alice_balance);
    assert!(
        alice_balance >= 10_000,
        "Alice should have at least 100k sats after funding"
    );

    // Bob exposes a Spark address (no SSP required)
    let bob_spark_address = bob
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    info!("Bob's Spark address: {}", bob_spark_address);

    // Alice prepares and sends the payment, preferring spark transfer
    let prepare = alice
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount_sats: Some(5_000),
        })
        .await?;

    info!("Sending 5,000 sats from Alice to Bob...");

    let send_resp = alice
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
        })
        .await?;

    info!("Alice send payment status: {:?}", send_resp.payment.status);
    assert!(
        matches!(
            send_resp.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Payment should be completed or pending"
    );

    // Bob syncs and verifies he received the payment
    bob.sync_wallet(SyncWalletRequest {}).await?;

    let payments = bob
        .list_payments(ListPaymentsRequest {
            offset: Some(0),
            limit: Some(50),
        })
        .await?
        .payments;

    let received = payments
        .into_iter()
        .find(|p| p.payment_type == PaymentType::Receive && p.amount >= 5_000);

    assert!(
        received.is_some(),
        "Bob should have a received payment >= 5000 sats"
    );

    let received_payment = received.unwrap();
    info!(
        "Bob received payment: {} sats, status: {:?}",
        received_payment.amount, received_payment.status
    );

    info!("=== Test test_breez_sdk_send_payment_prefer_spark PASSED ===");
    Ok(())
}
