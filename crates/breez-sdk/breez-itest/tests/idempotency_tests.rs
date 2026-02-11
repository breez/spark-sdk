use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;
use uuid::Uuid;

/// Test 1: Send payment from Alice to Bob using Spark transfer with idempotency key
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_spark_idempotency_key(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_01_spark_idempotency_key ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    ensure_funded(&mut alice, 10000).await?;

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Alice balance: {} sats", alice_balance);

    // Get Bob's initial balance
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_initial_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Bob initial balance: {} sats", bob_initial_balance);

    // Bob exposes a Spark address
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    info!("Bob's Spark address: {}", bob_spark_address);

    // Alice prepares and sends 5 sats to Bob
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

    info!("Sending 5 sats from Alice to Bob via Spark...");

    let idempotency_key = Uuid::now_v7().to_string();
    info!("Idempotency key: {}", idempotency_key);

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: None,
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;

    info!("Alice send payment id: {:?}", send_resp.payment.id);
    assert_eq!(
        send_resp.payment.id, idempotency_key,
        "Payment ID should match idempotency key"
    );

    info!("Resending the same payment with the same idempotency key before payment is synced");
    let resend_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: None,
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;
    assert_eq!(
        send_resp.payment.id, resend_resp.payment.id,
        "Resent payment should have the same ID"
    );
    assert_eq!(
        send_resp.payment.timestamp, resend_resp.payment.timestamp,
        "Resent payment should have the same timestamp"
    );

    info!("Syncing Alice's wallet to finalize the payment...");
    let received_payment =
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;
    assert_eq!(
        received_payment.payment_type,
        PaymentType::Send,
        "Alice should have sent first payment"
    );

    info!("Resending the same payment with the same idempotency key after payment is synced");
    let resend_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;
    assert_eq!(
        send_resp.payment.id, resend_resp.payment.id,
        "Resent payment should have the same ID"
    );
    assert_eq!(
        send_resp.payment.timestamp, resend_resp.payment.timestamp,
        "Resent payment should have the same timestamp"
    );

    // Wait for Bob to receive payment succeeded event
    info!("Waiting for Bob to receive payment succeeded event...");
    let received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(
        received_payment.payment_type,
        PaymentType::Receive,
        "Bob should receive a payment"
    );
    assert!(
        received_payment.amount >= 5,
        "Bob should receive at least 5 sats"
    );

    // Verify Bob's balance increased
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!(
        "Bob's balance: {} -> {} sats (change: +{})",
        bob_initial_balance,
        bob_final_balance,
        bob_final_balance as i64 - bob_initial_balance as i64
    );
    assert!(
        bob_final_balance > bob_initial_balance,
        "Bob's balance should increase"
    );

    info!("=== Test test_01_spark_idempotency_key PASSED ===");
    Ok(())
}

/// Test 2: Send payment from Alice to Bob using Lightning with idempotency key
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_lightning_idempotency_key(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_02_lightning_idempotency_key ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    ensure_funded(&mut alice, 10000).await?;

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Alice balance: {} sats", alice_balance);

    // Get Bob's initial balance
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_initial_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Bob initial balance: {} sats", bob_initial_balance);

    // Bob creates a Lightning invoice
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "idempotency test".to_string(),
                amount_sats: Some(5),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    info!("Bob's Lightning invoice: {}", bob_invoice);

    // Alice prepares and sends 5 sats to Bob
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice.clone(),
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!("Sending 5 sats from Alice to Bob via Lightning...");

    let idempotency_key = Uuid::now_v7().to_string();
    info!("Idempotency key: {}", idempotency_key);

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: None,
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;

    info!("Alice send payment id: {:?}", send_resp.payment.id);
    assert_eq!(
        send_resp.payment.id, idempotency_key,
        "Payment ID should match idempotency key"
    );

    info!("Resending the same payment with the same idempotency key before payment is synced");
    let resend_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: None,
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;
    assert_eq!(
        send_resp.payment.id, resend_resp.payment.id,
        "Resent payment should have the same ID"
    );
    assert_eq!(
        send_resp.payment.timestamp, resend_resp.payment.timestamp,
        "Resent payment should have the same timestamp"
    );

    info!("Syncing Alice's wallet to finalize the payment...");
    let received_payment =
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;
    assert_eq!(
        received_payment.payment_type,
        PaymentType::Send,
        "Alice should have sent first payment"
    );

    info!("Resending the same payment with the same idempotency key after payment is synced");
    let resend_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;
    assert_eq!(
        send_resp.payment.id, resend_resp.payment.id,
        "Resent payment should have the same ID"
    );
    assert_eq!(
        send_resp.payment.timestamp, resend_resp.payment.timestamp,
        "Resent payment should have the same timestamp"
    );

    // Wait for Bob to receive payment succeeded event
    info!("Waiting for Bob to receive payment succeeded event...");
    let received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(
        received_payment.payment_type,
        PaymentType::Receive,
        "Bob should receive a payment"
    );
    assert!(
        received_payment.amount >= 5,
        "Bob should receive at least 5 sats"
    );

    // Verify Bob's balance increased
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!(
        "Bob's balance: {} -> {} sats (change: +{})",
        bob_initial_balance,
        bob_final_balance,
        bob_final_balance as i64 - bob_initial_balance as i64
    );
    assert!(
        bob_final_balance > bob_initial_balance,
        "Bob's balance should increase"
    );

    info!("=== Test test_02_lightning_idempotency_key PASSED ===");
    Ok(())
}

/// Send on-chain from Alice to Bob's static deposit address with idempotency key
#[rstest]
#[test_log::test(tokio::test)]
async fn test_03_bitcoin_idempotency_key(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_03_bitcoin_idempotency_key ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Ensure Alice has enough funds for withdraw amount + fees
    ensure_funded(&mut alice, 50_000).await?;

    // Record Bob's initial balance
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_initial_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    // Bob exposes a static deposit address
    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress,
        })
        .await?
        .payment_request;
    info!("Bob deposit address: {}", bob_address);

    // Alice prepares and sends 15_000 sats on-chain to Bob
    let amount = 15_000u64;
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_address.clone(),
            amount: Some(amount as u128),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let idempotency_key = Uuid::now_v7().to_string();
    info!("Idempotency key: {}", idempotency_key);

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: None,
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;

    info!("Alice withdraw id: {:?}", send_resp.payment.id);
    assert_eq!(
        send_resp.payment.id, idempotency_key,
        "Payment ID should match idempotency key"
    );

    info!("Resending the same withdraw with the same idempotency key before payment is synced");
    let resend_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: None,
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;
    assert_eq!(
        send_resp.payment.id, resend_resp.payment.id,
        "Resent payment should have the same ID"
    );
    assert_eq!(
        send_resp.payment.timestamp, resend_resp.payment.timestamp,
        "Resent payment should have the same timestamp"
    );

    // Trigger Bob sync and wait for receive + claim
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let recv_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 180).await?;
    assert!(matches!(recv_payment.method, PaymentMethod::Deposit));

    info!("Resending the same withdraw with the same idempotency key after payment is synced");
    let resend_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: Some(idempotency_key),
        })
        .await?;
    assert_eq!(
        send_resp.payment.id, resend_resp.payment.id,
        "Resent payment should have the same ID"
    );
    assert_eq!(
        send_resp.payment.timestamp, resend_resp.payment.timestamp,
        "Resent payment should have the same timestamp"
    );

    // Verify Bob's balance increased
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!(
        "Bob's balance: {} -> {} sats (change: +{})",
        bob_initial_balance,
        bob_final_balance,
        bob_final_balance as i64 - bob_initial_balance as i64
    );
    assert!(
        bob_final_balance > bob_initial_balance,
        "Bob's balance should increase"
    );

    info!("=== Test test_03_bitcoin_idempotency_key PASSED ===");
    Ok(())
}

/// Test 4: Send payment from Alice to Bob using Spark HTLC with idempotency key
#[rstest]
#[test_log::test(tokio::test)]
async fn test_04_spark_htlc_idempotency_key(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_04_spark_htlc_idempotency_key ===");

    let mut alice = alice_sdk.await?;
    let bob = bob_sdk.await?;

    ensure_funded(&mut alice, 10000).await?;

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Alice balance: {} sats", alice_balance);

    // Get Bob's initial balance
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_initial_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    info!("Bob initial balance: {} sats", bob_initial_balance);

    // Bob exposes a Spark address
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    info!("Bob's Spark address: {}", bob_spark_address);

    // Alice prepares and sends 5 sats to Bob
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

    let idempotency_key = Uuid::now_v7().to_string();
    info!("Idempotency key: {}", idempotency_key);

    let (_, payment_hash) = generate_preimage_hash_pair();

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: Some(SendPaymentOptions::SparkAddress {
                htlc_options: Some(SparkHtlcOptions {
                    payment_hash: payment_hash.clone(),
                    expiry_duration_secs: 180,
                }),
            }),
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;

    info!("Alice send payment id: {:?}", send_resp.payment.id);
    assert_eq!(
        send_resp.payment.id, idempotency_key,
        "Payment ID should match idempotency key"
    );

    info!("Resending the same payment with the same idempotency key before payment is synced");
    let resend_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: Some(SendPaymentOptions::SparkAddress {
                htlc_options: Some(SparkHtlcOptions {
                    payment_hash: payment_hash.clone(),
                    expiry_duration_secs: 180,
                }),
            }),
            idempotency_key: Some(idempotency_key.clone()),
        })
        .await?;
    assert_eq!(
        send_resp.payment.id, resend_resp.payment.id,
        "Resent payment should have the same ID"
    );
    assert_eq!(
        send_resp.payment.timestamp, resend_resp.payment.timestamp,
        "Resent payment should have the same timestamp"
    );

    info!("=== Test test_04_spark_htlc_idempotency_key PASSED ===");
    Ok(())
}
